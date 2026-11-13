//! A short-lived bridge to the browser that supplied a reviewed video.
//!
//! The browser owns TLS, CORS and DNS for this transport. It is never eligible
//! for public-only jobs, explicit proxy routes, or arbitrary script HTTP calls.
//! No browser cookies or authorization headers cross the bridge.
use anyhow::{Context, Result, ensure};
use hyper::{
    body::{Body, Bytes, Frame},
    http,
};
use reqwest::{Response, ResponseBuilderExt, Url};
use serde::{Deserialize, Serialize};
use std::{
    io,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub mod host;

pub const CHUNK_BYTES: usize = 48 * 1024;
const HEAD_BYTES: usize = 16 * 1024;
const LIFETIME_SECONDS: i64 = 6 * 60 * 60;
const HEAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const FAILURE_FRAME: u32 = u32::MAX;

// Fixed codes only: page errors, URLs and credentials never enter IPC errors.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
enum Failure {
    #[default]
    Network = 1,
    Timeout = 2,
    TooLarge = 3,
    Unsupported = 4,
    Cancelled = 5,
    Protocol = 6,
    Http = 7,
}
impl Failure {
    fn from_code(code: u8) -> Result<Self> {
        Ok(match code {
            1 => Self::Network,
            2 => Self::Timeout,
            3 => Self::TooLarge,
            4 => Self::Unsupported,
            5 => Self::Cancelled,
            6 => Self::Protocol,
            7 => Self::Http,
            _ => anyhow::bail!("Invalid browser video failure code"),
        })
    }
    fn message(self) -> &'static str {
        match self {
            Self::Network => {
                "The browser could not finish fetching a video segment. Keep the video tab open, start playback and retry."
            }
            Self::Timeout => {
                "The browser video request timed out. Keep the video tab open and retry."
            }
            Self::TooLarge => {
                "The browser video response exceeds the allowed segment or byte-range size."
            }
            Self::Unsupported => {
                "The browser video request is outside the reviewed page or servers. Find videos again."
            }
            Self::Cancelled => {
                "The browser video request was cancelled or its page changed. Find videos again."
            }
            Self::Protocol => {
                "The browser video connection returned an invalid response. Reload the extension and find videos again."
            }
            Self::Http => {
                "The video server rejected the browser request. Start playback and find videos again."
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub port: u16,
    pub token: Uuid,
    pub expires: i64,
    pub page: String,
    pub origins: Vec<String>,
}
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserSession")
            .field("expires", &self.expires)
            .finish_non_exhaustive()
    }
}
impl Session {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.port != 0 && !self.token.is_nil(),
            "Invalid browser video session"
        );
        validate_scope(&self.page, &self.origins)?;
        let now = chrono::Utc::now().timestamp();
        ensure!(
            self.expires > 0 && self.expires <= now + LIFETIME_SECONDS + 60,
            "Invalid browser video session lifetime"
        );
        Ok(())
    }
    pub(crate) fn allows(&self, url: &Url) -> Result<()> {
        self.validate()?;
        ensure!(
            self.expires > chrono::Utc::now().timestamp(),
            "Browser video session expired. Open the video page and find videos again."
        );
        ensure!(
            self.origins.contains(&url.origin().ascii_serialization()),
            "This media server was not included in the browser review. Play the video and find videos again."
        );
        Ok(())
    }
}

pub fn validate_scope(page: &str, origins: &[String]) -> Result<()> {
    let page = crate::model::validate_url(page)?;
    ensure!(
        !origins.is_empty() && origins.len() <= 16,
        "Invalid browser video servers"
    );
    let mut seen = std::collections::HashSet::new();
    for origin in origins {
        let url = crate::model::validate_url(origin)?;
        ensure!(
            url.origin().ascii_serialization() == *origin && seen.insert(origin),
            "Invalid browser video server origin"
        );
        // HTTP is supported only for an explicitly opened loopback fixture/page,
        // never as a downgrade from a remote page or as a different local server.
        let local = page.scheme() == "http"
            && page.origin() == url.origin()
            && page
                .host_str()
                .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
                .is_some_and(|ip| ip.is_loopback());
        ensure!(
            (page.scheme() == "https" && url.scheme() == "https") || local,
            "Browser video transfers require HTTPS"
        );
        if !local
            && let Some(host) = url.host_str()
            && let Ok(ip) = host.trim_matches(['[', ']']).parse::<std::net::IpAddr>()
        {
            ensure!(
                crate::network_scope::network_scope(ip)?
                    == crate::network_scope::NetworkScope::Public,
                "Browser video cannot request a local or special-use server"
            );
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: Uuid,
    url: String,
    range: Option<crate::hls::Range>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Head {
    status: u16,
    headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure: Option<Failure>,
}

type Chunk = std::result::Result<Frame<Bytes>, io::Error>;
struct Stream {
    receiver: mpsc::Receiver<Chunk>,
    cancel: CancellationToken,
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl Body for Stream {
    type Data = Bytes;
    type Error = io::Error;
    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Chunk>> {
        self.receiver.poll_recv(cx)
    }
}

pub(crate) async fn get(
    session: &Session,
    url: &Url,
    range: Option<&crate::hls::Range>,
    cancel: &CancellationToken,
) -> Result<Response> {
    session.allows(url)?;
    // Recheck all returned addresses even for the original media origin. The
    // browser still owns its resolution/connection; this is not IP pinning.
    // Explicit loopback pages are the only exception, for local media/testing.
    if url.scheme() == "https" {
        let resolved = tokio::select! {
            biased;
            _ = cancel.cancelled() => anyhow::bail!("Browser video request cancelled"),
            result = crate::website_grabber::pinned_resolution(url) => result?,
        };
        if let Some((_, addresses)) = resolved {
            for address in addresses {
                ensure!(
                    crate::network_scope::network_scope(address.ip())?
                        == crate::network_scope::NetworkScope::Public,
                    "Browser video cannot request a local or special-use server"
                );
            }
        }
    }
    let opening = async {
        // A literal loopback address and no HTTP proxy: this is local IPC, not
        // an extension-accessible web server. The first frame authenticates it.
        let mut socket = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, session.port))
            .await
            .context(
                "Browser video connection closed. Keep the video tab open and find videos again.",
            )?;
        socket.set_nodelay(true)?;
        let request = serde_json::to_vec(&Request {
            token: session.token,
            url: url.to_string(),
            range: range.cloned(),
        })?;
        ensure!(
            request.len() <= HEAD_BYTES,
            "Browser video request is too large"
        );
        socket.write_u32_le(request.len() as u32).await?;
        socket.write_all(&request).await?;
        let length = socket.read_u32_le().await.context(
            "The browser video connection ended before sending a response. Keep the tab open and find videos again.",
        )? as usize;
        ensure!(
            length > 0 && length <= HEAD_BYTES,
            "Invalid browser video response"
        );
        let mut bytes = vec![0; length];
        socket.read_exact(&mut bytes).await?;
        let head: Head = serde_json::from_slice(&bytes)?;
        if let Some(failure) = head.failure {
            anyhow::bail!(failure.message());
        }
        ensure!(
            matches!(head.status, 200 | 206),
            "The browser could not fetch this video (HTTP {}). Keep its tab open and start playback before retrying.",
            head.status
        );
        ensure!(head.headers.len() <= 5, "Too many browser video headers");
        let mut response = http::Response::builder()
            .status(head.status)
            .url(url.clone());
        let mut seen = std::collections::HashSet::new();
        for (name, value) in head.headers {
            ensure!(
                matches!(
                    name.as_str(),
                    "content-type" | "content-range" | "etag" | "last-modified"
                ) && value.len() <= 2048
                    && seen.insert(name.clone()),
                "Invalid browser video header"
            );
            response = response.header(name, value);
        }
        Ok::<_, anyhow::Error>((socket, response))
    };
    let (mut socket, response) = tokio::select! {
        biased;
        _=cancel.cancelled()=>anyhow::bail!("Browser video request cancelled"),
        result=tokio::time::timeout(HEAD_TIMEOUT, opening)=>result.context("Browser video request timed out")??,
    };
    let stop = cancel.child_token();
    let worker_stop = stop.clone();
    let (tx, receiver) = mpsc::channel(1);
    let maximum = range.map_or(crate::hls::MAX_PART_BYTES, |r| r.length);
    tokio::spawn(async move {
        let read = async {
            let mut total = 0u64;
            loop {
                let length = tokio::time::timeout(HEAD_TIMEOUT, socket.read_u32_le())
                    .await
                    .context("Browser video read timed out")?
                    .context("The browser video connection ended before the segment was complete. Keep the tab open and retry.")?;
                if length == FAILURE_FRAME {
                    let code = tokio::time::timeout(HEAD_TIMEOUT, socket.read_u8())
                        .await
                        .context("Browser video read timed out")??;
                    anyhow::bail!(Failure::from_code(code)?.message());
                }
                if length == 0 {
                    break;
                }
                let length = length as usize;
                ensure!(length <= CHUNK_BYTES, "Invalid browser video chunk");
                total = total
                    .checked_add(length as u64)
                    .context("Browser video size overflow")?;
                ensure!(total <= maximum, "Browser video response exceeds its limit");
                let mut bytes = vec![0; length];
                tokio::time::timeout(HEAD_TIMEOUT, socket.read_exact(&mut bytes))
                    .await
                    .context("Browser video read timed out")??;
                if tx.send(Ok(Frame::data(Bytes::from(bytes)))).await.is_err() {
                    return Ok::<(), anyhow::Error>(());
                }
            }
            Ok(())
        };
        tokio::select! {
            biased;
            _=worker_stop.cancelled()=>{},
            result=read=>if let Err(error)=result {let _=tx.send(Err(io::Error::other(error.to_string()))).await;},
        }
        // Dropping the socket also tells the host to abort the browser fetch.
    });
    Ok(Response::from(response.body(reqwest::Body::wrap(
        Stream {
            receiver,
            cancel: stop,
        },
    ))?))
}
