//! Port of yt-dlp's watch-page visitor extraction, VISIONOS context and player
//! request. Only the anonymous, finite HLS path is used; see the source map.
use super::{media_url, watch_url};
use crate::{
    engine,
    network_scope::{NetworkScope, network_scope},
    website_grabber::pinned_resolution,
};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, RequestBuilder, Url, header};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const MAX_BODY: usize = 8 * 1024 * 1024;
const WEB_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const PLAYER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
const PLAYER: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";

pub(super) struct Player {
    pub title: String,
    pub manifest: String,
}

async fn client(cancel: &CancellationToken, route: &crate::proxy::Route) -> Result<Client> {
    let url = Url::parse(PLAYER)?;
    let resolved = tokio::select! { biased;
        _ = cancel.cancelled() => bail!("YouTube lookup cancelled"),
        result = tokio::time::timeout(Duration::from_secs(10), pinned_resolution(&url)) => result.context("YouTube DNS lookup timed out")??,
    }.context("YouTube hostname did not resolve")?;
    ensure!(!resolved.1.is_empty(), "YouTube hostname did not resolve");
    for address in &resolved.1 {
        ensure!(
            network_scope(address.ip())? == NetworkScope::Public,
            "YouTube resolved to a non-public server"
        );
    }
    Ok(route
        .configure(engine::client_builder())?
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&resolved.0, &resolved.1)
        .build()?)
}

async fn text(
    request: RequestBuilder,
    cancel: &CancellationToken,
    route: &crate::proxy::Route,
) -> Result<String> {
    let read = async {
        let (client, request) = request
            .header(header::ACCEPT_ENCODING, "identity")
            .build_split();
        let mut response = route.execute(&client, request?, None).await?;
        crate::auth::check_response(&response)?;
        let status = response.status();
        ensure!(
            !status.is_redirection(),
            "YouTube redirected this request. Open the video in your browser to check its availability or consent page."
        );
        ensure!(
            !matches!(status.as_u16(), 401 | 403 | 429),
            "YouTube refused this request (HTTP {}). Try again later; browser sign-in is not supported by this extractor.",
            status.as_u16()
        );
        ensure!(
            status == reqwest::StatusCode::OK,
            "YouTube returned HTTP {}",
            status.as_u16()
        );
        engine::identity(&response)?;
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= MAX_BODY as u64),
            "YouTube response exceeds 8 MiB"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= MAX_BODY,
                "YouTube response exceeds 8 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).context("YouTube response is not UTF-8")
    };
    tokio::select! { biased;
        _ = cancel.cancelled() => bail!("YouTube lookup cancelled"),
        result = tokio::time::timeout(Duration::from_secs(25), read) => result.context("YouTube request timed out")?,
    }
}

/// Read JSON data embedded in HTML without evaluating any script. Serde's depth
/// limit and our body/occurrence limits also apply to malformed documents.
pub(super) fn embedded(page: &str, marker: &str) -> Result<Option<Value>> {
    ensure!(page.len() <= MAX_BODY, "YouTube response exceeds 8 MiB");
    for (attempt, (_, tail)) in page
        .match_indices(marker)
        .map(|(offset, _)| (offset, &page[offset + marker.len()..]))
        .enumerate()
    {
        ensure!(attempt < 64, "Too many YouTube configuration blocks");
        let tail = tail.trim_start();
        if !tail.starts_with('{') || !tail[1..].trim_start().starts_with('"') {
            continue;
        }
        let value = serde_json::Deserializer::from_str(tail)
            .into_iter::<Value>()
            .next()
            .context("Missing YouTube configuration")?
            .context("Invalid YouTube configuration JSON")?;
        return Ok(Some(value));
    }
    Ok(None)
}

pub(super) fn request_body(id: &str, visitor: &str) -> Value {
    json!({"videoId":id,"context":{"client":{
        "clientName":"VISIONOS","clientVersion":"1.02","deviceMake":"Apple",
        "deviceModel":"RealityDevice17,1","userAgent":PLAYER_UA,"osName":"visionOS",
        "osVersion":"26.5.23O471","hl":"en","timeZone":"UTC","utcOffsetMinutes":0,"visitorData":visitor
    }},"playbackContext":{"contentPlaybackContext":{"html5Preference":"HTML5_PREF_WANTS"}},
        "contentCheckOk":true,"racyCheckOk":true})
}

fn availability(player: &Value) -> Result<()> {
    let status = player
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str)
        .context("YouTube response has no playback status")?;
    match status {
        "OK" => {}
        "LOGIN_REQUIRED" | "AGE_CHECK_REQUIRED" | "AGE_VERIFICATION_REQUIRED" => bail!(
            "YouTube requires sign-in, an age check, or a browser verification for this video. This extractor supports public videos without sign-in."
        ),
        "LIVE_STREAM_OFFLINE" => {
            bail!("This YouTube live stream has not finished. Live recording is not supported.")
        }
        "UNPLAYABLE" | "ERROR" => bail!(
            "YouTube did not make this video available to the anonymous player. It may be unavailable, restricted, or unsupported by this player client."
        ),
        _ => bail!(
            "YouTube returned an unsupported playback status. Open the video in your browser to check availability."
        ),
    }
    ensure!(
        !player
            .pointer("/videoDetails/isLive")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && !player
                .pointer("/videoDetails/isUpcoming")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            && !player
                .pointer("/microformat/playerMicroformatRenderer/liveBroadcastDetails/isLiveNow")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        "This YouTube video is live or upcoming. Only finished videos are supported."
    );
    Ok(())
}

pub(super) fn parse_player(player: &Value, id: &str) -> Result<Player> {
    availability(player)?;
    ensure!(
        player
            .pointer("/videoDetails/videoId")
            .and_then(Value::as_str)
            == Some(id),
        "YouTube returned a different video. Nothing was queued."
    );
    let title = player
        .pointer("/videoDetails/title")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.len() <= 4096)
        .context("YouTube response has no valid video title")?;
    let duration = player
        .pointer("/videoDetails/lengthSeconds")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<u64>().ok())
        .context("YouTube did not provide a finite duration")?;
    ensure!(
        duration > 0 && duration <= 7 * 24 * 3600,
        "YouTube video duration is not supported"
    );
    ensure!(
        player.pointer("/streamingData/licenseInfos").is_none(),
        "Encrypted YouTube videos are not supported"
    );
    let manifest = player.pointer("/streamingData/hlsManifestUrl").and_then(Value::as_str)
        .context("YouTube did not provide a supported HLS stream. JavaScript-challenged, token-required and direct adaptive formats are not supported by this extractor yet.")?;
    media_url(manifest)?;
    Ok(Player {
        title: title.chars().filter(|c| !c.is_control()).collect(),
        manifest: manifest.into(),
    })
}

pub(super) async fn extract(
    id: &str,
    cancel: &CancellationToken,
    route: &crate::proxy::Route,
) -> Result<Player> {
    let client = client(cancel, route).await?;
    extract_with(
        &client,
        &format!("{}&hl=en", watch_url(id)?),
        PLAYER,
        id,
        cancel,
        route,
    )
    .await
}

async fn extract_with(
    client: &Client,
    watch: &str,
    endpoint: &str,
    id: &str,
    cancel: &CancellationToken,
    route: &crate::proxy::Route,
) -> Result<Player> {
    let page = text(
        client.get(watch).header(header::USER_AGENT, WEB_UA),
        cancel,
        route,
    )
    .await?;
    let config = embedded(&page, "ytcfg.set(")?.context("YouTube did not return its player configuration. Check the video or consent page in your browser.")?;
    let initial = embedded(&page, "var ytInitialPlayerResponse =")?;
    if let Some(initial) = &initial {
        availability(initial)?;
        ensure!(
            initial
                .pointer("/videoDetails/videoId")
                .and_then(Value::as_str)
                == Some(id),
            "YouTube watch page returned a different video"
        );
    }
    let visitor = config
        .get("VISITOR_DATA")
        .or_else(|| config.pointer("/INNERTUBE_CONTEXT/client/visitorData"))
        .or_else(|| {
            initial
                .as_ref()
                .and_then(|p| p.pointer("/responseContext/visitorData"))
        })
        .and_then(Value::as_str)
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 2048
                && s.is_ascii()
                && !s.bytes().any(|b| b.is_ascii_control())
        })
        .context("YouTube did not return a valid visitor session. Try loading the video again.")?;
    let body = serde_json::to_vec(&request_body(id, visitor))?;
    let response = text(
        client
            .post(endpoint)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::USER_AGENT, PLAYER_UA)
            .header(header::ORIGIN, "https://www.youtube.com")
            .header("X-YouTube-Client-Name", "101")
            .header("X-YouTube-Client-Version", "1.02")
            .header("X-Goog-Visitor-Id", visitor)
            .body(body),
        cancel,
        route,
    )
    .await?;
    let player: Value = serde_json::from_str(&response).context("Invalid YouTube player JSON")?;
    parse_player(&player, id)
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
