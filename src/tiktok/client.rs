//! Reads one TikTok video page as a browser would on a first visit: the page
//! embeds the video data as JSON and sets TikTok's anonymous visitor cookies.
use crate::{
    engine,
    network_scope::{NetworkScope, network_scope},
    website_grabber::pinned_resolution,
};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Url, header};
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const MAX_PAGE: usize = 6 * 1024 * 1024;
const WEB_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const DATA: &str = "__UNIVERSAL_DATA_FOR_REHYDRATION__";

pub(super) struct Page {
    pub item: Value,
    /// `name=value` pairs from the page's Set-Cookie headers, for its video servers.
    pub cookies: String,
}

pub(super) async fn page(
    url: &str,
    cancel: &CancellationToken,
    route: &crate::proxy::Route,
) -> Result<Page> {
    let url = Url::parse(url)?;
    let resolved = tokio::select! { biased;
        _ = cancel.cancelled() => bail!("TikTok lookup cancelled"),
        result = tokio::time::timeout(Duration::from_secs(10), pinned_resolution(&url)) => result.context("TikTok DNS lookup timed out")??,
    }
    .context("TikTok hostname did not resolve")?;
    ensure!(!resolved.1.is_empty(), "TikTok hostname did not resolve");
    for address in &resolved.1 {
        ensure!(
            network_scope(address.ip())? == NetworkScope::Public,
            "TikTok resolved to a non-public server"
        );
    }
    let client = route
        .configure(engine::client_builder())?
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&resolved.0, &resolved.1)
        .build()?;
    let read = async {
        let (client, request) = client
            .get(url)
            .header(header::USER_AGENT, WEB_UA)
            .header(header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
            .header(header::ACCEPT_ENCODING, "identity")
            .build_split();
        let mut response = route.execute(&client, request?, None).await?;
        let status = response.status();
        ensure!(
            !status.is_redirection(),
            "TikTok redirected this video page. It may be removed, private or limited in your region."
        );
        ensure!(
            status == reqwest::StatusCode::OK,
            "TikTok returned HTTP {}",
            status.as_u16()
        );
        engine::identity(&response)?;
        let cookies = cookies(
            response
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .filter_map(|v| v.to_str().ok()),
        );
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= MAX_PAGE as u64),
            "TikTok page exceeds 6 MiB"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= MAX_PAGE,
                "TikTok page exceeds 6 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        let html = String::from_utf8(bytes).context("TikTok page is not UTF-8")?;
        Ok(Page {
            item: item(&html)?,
            cookies,
        })
    };
    tokio::select! { biased;
        _ = cancel.cancelled() => bail!("TikTok lookup cancelled"),
        result = tokio::time::timeout(Duration::from_secs(30), read) => result.context("TikTok request timed out")?,
    }
}

/// Keep only well-formed `name=value` pairs; attributes such as Domain or Expires
/// are dropped, and the values are only ever sent back to tiktok.com.
pub(super) fn cookies<'a>(headers: impl Iterator<Item = &'a str>) -> String {
    headers
        .filter_map(|value| value.split(';').next())
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(|(name, value)| {
            !name.is_empty()
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                && value.len() <= 4096
                && value
                    .bytes()
                    .all(|b| b.is_ascii_graphic() && b != b';' && b != b',')
        })
        .take(16)
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The video's data from the page's embedded JSON, without running any script.
pub(super) fn item(html: &str) -> Result<Value> {
    // TikTok sometimes answers automated page loads with a browser check that needs
    // JavaScript. Download Engine does not try to pass it; it reports it instead.
    ensure!(
        !html.contains("_wafchallengeid") && !html.contains("SlardarWAF"),
        "TikTok is asking this computer for a browser check right now. Open TikTok in your browser, then try again in a few minutes."
    );
    let missing = "TikTok did not return the video data. Open the video in your browser, then try again later.";
    let start = html.find(&format!("id=\"{DATA}\"")).context(missing)?;
    let body = &html[start..];
    let open = body.find('>').context(missing)? + 1;
    let close = body[open..].find("</script>").context(missing)?;
    let data: Value =
        serde_json::from_str(&body[open..open + close]).context("Invalid TikTok video data")?;
    let detail = data
        .pointer("/__DEFAULT_SCOPE__/webapp.video-detail")
        .context(missing)?;
    match detail
        .get("statusCode")
        .and_then(Value::as_i64)
        .unwrap_or(0)
    {
        0 => {}
        10204 => bail!("This TikTok video was removed or does not exist"),
        10222 => bail!("This TikTok video is private"),
        10216 => bail!("This TikTok video is under review and cannot be downloaded yet"),
        code => {
            let message: String = detail
                .get("statusMsg")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .chars()
                .filter(|c| !c.is_control())
                .take(120)
                .collect();
            bail!("TikTok refused this video (code {code}: {message})")
        }
    }
    detail
        .pointer("/itemInfo/itemStruct")
        .cloned()
        .context(missing)
}
