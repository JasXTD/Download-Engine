//! Torrent-only interface policy. Binding is enforced on the socket, before any
//! connect/DNS/TLS request. Normal application downloads do not use this policy.
use anyhow::{Context, Result, ensure};
use futures_util::{StreamExt, future::BoxFuture};
use http_body_util::BodyExt;
use hyper::{Request, body::Bytes};
use hyper_util::rt::TokioIo;
use librqbit_core::network::is_local_address;
pub use librqbit_dualstack_sockets::{
    BindDevice,
    interfaces::{Interface, interfaces},
};
use librqbit_dualstack_sockets::{ConnectOpts, dns, tcp_connect};
use librqbit_tracker_comms::HttpTransport;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::time::{Instant, timeout_at};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub interface: String,
    pub address: IpAddr,
    /// Numeric server address; no unbound bootstrap lookup or system DNS fallback.
    pub dns: IpAddr,
}
impl Binding {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.interface.is_empty()
                && self.interface.len() <= 256
                && !self.interface.chars().any(char::is_control),
            "Invalid torrent interface name"
        );
        for address in [self.address, self.dns] {
            ensure!(
                !address.is_unspecified()
                    && !address.is_multicast()
                    && !matches!(address, IpAddr::V6(ip) if ip.is_unicast_link_local()),
                "Choose a usable torrent address and DNS server"
            );
        }
        ensure!(
            self.address.is_ipv6() == self.dns.is_ipv6(),
            "Torrent address and DNS server must use the same IP version"
        );
        Ok(())
    }
    pub fn open(&self) -> Result<BindDevice> {
        self.validate()?;
        BindDevice::new(&self.interface, self.address, Some(SocketAddr::new(self.dns, 53)))
            .context("The selected torrent interface/address is unavailable. Torrents will not use another connection.")
    }
}

pub struct BoundHttp {
    device: Arc<BindDevice>,
    allow_local: bool,
    tls: tokio_native_tls::TlsConnector,
}
struct Abort(tokio::task::JoinHandle<()>);
impl Drop for Abort {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl BoundHttp {
    pub fn new(device: Arc<BindDevice>, allow_local: bool) -> Result<Self> {
        Ok(Self {
            device,
            allow_local,
            tls: native_tls::TlsConnector::new()?.into(),
        })
    }
    pub async fn send(&self, request: reqwest::Request) -> Result<reqwest::Response> {
        let deadline = Instant::now() + Duration::from_secs(30);
        timeout_at(deadline, self.send_headers(request, deadline))
            .await
            .context("Bound HTTP request timed out")?
    }
    async fn send_headers(
        &self,
        request: reqwest::Request,
        deadline: Instant,
    ) -> Result<reqwest::Response> {
        self.device.check()?;
        ensure!(
            request.method() == reqwest::Method::GET && request.body().is_none(),
            "Unsupported bound HTTP request"
        );
        let url = request.url();
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none(),
            "Invalid torrent HTTP URL"
        );
        let host = url
            .host_str()
            .context("Missing HTTP host")?
            .trim_matches(['[', ']']);
        let port = url.port_or_known_default().context("Missing HTTP port")?;
        let addresses = dns::lookup(Some(&self.device), host, port).await?;
        let mut stream = None;
        let mut last = None;
        for address in addresses
            .into_iter()
            .filter(|address| self.allow_local || !is_local_address(address.ip()))
            .take(8)
        {
            match tokio::time::timeout(
                Duration::from_secs(5),
                tcp_connect(
                    address,
                    ConnectOpts {
                        bind_device: Some(&self.device),
                        ..Default::default()
                    },
                ),
            )
            .await
            {
                Ok(Ok(value)) => {
                    stream = Some(value);
                    break;
                }
                Ok(Err(error)) => last = Some(error.to_string()),
                Err(_) => last = Some("Connection timed out".to_owned()),
            }
        }
        let stream = stream.with_context(|| {
            last.unwrap_or_else(|| "No permitted address for torrent HTTP request".to_owned())
        })?;
        let stream: crate::peer::BoxedStream = if url.scheme() == "https" {
            // Native trust store, hostname and certificate verification remain enabled.
            Box::new(self.tls.connect(host, stream).await?)
        } else {
            Box::new(stream)
        };
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
        let task = Abort(tokio::spawn(async move {
            let _ = connection.await;
        }));
        let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
        let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
        let mut outgoing = Request::builder()
            .method("GET")
            .uri(path)
            .body(http_body_util::Empty::<Bytes>::new())?;
        *outgoing.headers_mut() = request.headers().clone();
        outgoing
            .headers_mut()
            .insert(hyper::header::HOST, authority.parse()?);
        outgoing
            .headers_mut()
            .insert(hyper::header::CONNECTION, "close".parse()?);
        let response = sender.send_request(outgoing).await?;
        let (parts, body) = response.into_parts();
        // The connection task belongs to the body: cancellation, parser failure and
        // pause/stop drop it immediately; there is no detached HTTP connection.
        let stream = futures_util::stream::unfold(
            Some((body.into_data_stream(), task)),
            move |state| async move {
                let (mut body, task) = state?;
                match timeout_at(deadline, body.next()).await {
                    Ok(Some(item)) => {
                        Some((item.map_err(std::io::Error::other), Some((body, task))))
                    }
                    Ok(None) => None,
                    Err(_) => Some((
                        Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "Bound HTTP body timed out",
                        )),
                        None,
                    )),
                }
            },
        );
        Ok(hyper::Response::from_parts(parts, reqwest::Body::wrap_stream(stream)).into())
    }
}
impl HttpTransport for BoundHttp {
    fn get(&self, request: reqwest::Request) -> BoxFuture<'_, Result<reqwest::Response>> {
        Box::pin(self.send(request))
    }
}
