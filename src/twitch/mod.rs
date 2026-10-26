//! Twitch videos (past broadcasts, highlights and uploads at twitch.tv/videos/…),
//! read as Twitch's web player does: one public GraphQL query returns the title
//! and an anonymous playback token, which opens the video's HLS master playlist.
//! The qualities' own playlists need no token, so a chosen quality downloads and
//! resumes as an ordinary finished HLS stream. Subscriber-only videos, live
//! channels and clips are not supported.
mod client;

use crate::hls::{self, Inspection};
use anyhow::{Context, Result, bail, ensure};
use reqwest::Url;
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct Video {
    pub title: String,
    pub inspection: Inspection,
}

pub fn is_url(value: &str) -> bool {
    video_id(value).is_ok()
}

fn video_id(value: &str) -> Result<String> {
    let url = crate::model::validate_url(value)?;
    ensure!(
        matches!(
            url.host_str(),
            Some("www.twitch.tv" | "twitch.tv" | "m.twitch.tv")
        ) && url.port().is_none(),
        "Use a Twitch video link (twitch.tv/videos/…)"
    );
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    let ["videos", id] = parts.as_slice() else {
        bail!("This Twitch link does not identify one video");
    };
    ensure!(valid_id(id), "This Twitch link does not identify one video");
    Ok((*id).to_owned())
}

fn valid_id(id: &str) -> bool {
    (1..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit()) && !id.starts_with('0')
}

/// The stable page address saved with a download.
pub fn source_url(id: &str) -> Result<String> {
    ensure!(valid_id(id), "Invalid Twitch video ID");
    Ok(format!("https://www.twitch.tv/videos/{id}"))
}

fn clean(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect::<String>()
        .trim()
        .to_owned()
}

struct Access {
    title: String,
    token: String,
    signature: String,
}

/// The GraphQL reply: the video's details and its anonymous playback token.
fn access(reply: &Value, id: &str) -> Result<Access> {
    if let Some(message) = reply.pointer("/errors/0/message").and_then(Value::as_str) {
        bail!("Twitch refused this request ({})", clean(message, 120));
    }
    let video = reply
        .pointer("/data/video")
        .filter(|v| !v.is_null())
        .context("This Twitch video was removed, is private or does not exist")?;
    ensure!(
        video["id"].as_str() == Some(id),
        "Twitch returned a different video"
    );
    let token = reply
        .pointer("/data/videoPlaybackAccessToken")
        .filter(|v| !v.is_null())
        .context("Twitch did not allow playing this video")?;
    let value = token["value"]
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= 16 * 1024)
        .context("Twitch returned an invalid playback token")?;
    let signature = token["signature"]
        .as_str()
        .filter(|s| (1..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .context("Twitch returned an invalid playback token")?;
    let title = video["title"].as_str().map(|t| clean(t, 200));
    let owner = video
        .pointer("/owner/displayName")
        .and_then(Value::as_str)
        .map(|o| clean(o, 64));
    let title = match (
        owner.filter(|o| !o.is_empty()),
        title.filter(|t| !t.is_empty()),
    ) {
        (Some(owner), Some(title)) => format!("{owner} - {title}"),
        (None, Some(title)) => title,
        (Some(owner), None) => format!("{owner} - Twitch video {id}"),
        (None, None) => format!("Twitch video {id}"),
    };
    Ok(Access {
        title: clean(&title, 200),
        token: value.to_owned(),
        signature: signature.to_owned(),
    })
}

/// The master playlist address Twitch's web player opens with the token.
fn usher_url(id: &str, access: &Access) -> Result<Url> {
    ensure!(valid_id(id), "Invalid Twitch video ID");
    Ok(Url::parse_with_params(
        &format!("https://usher.ttvnw.net/vod/{id}.m3u8"),
        [
            ("allow_source", "true"),
            ("allow_audio_only", "true"),
            ("player", "twitchweb"),
            ("playlist_include_framerate", "true"),
            ("sig", access.signature.as_str()),
            ("token", access.token.as_str()),
        ],
    )?)
}

/// Fetch the video's details and its finite list of qualities. Nothing is queued.
pub async fn inspect_via(
    value: &str,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Video> {
    use crate::diagnostics::{self, Details, Event, Level};
    let started = std::time::Instant::now();
    diagnostics::record(Level::Info, Event::VideoLookupStarted, Details::default());
    let result = lookup(value, cancel, route).await;
    diagnostics::record(
        if result.is_ok() {
            Level::Info
        } else {
            Level::Warn
        },
        if result.is_ok() {
            Event::VideoLookupSucceeded
        } else {
            Event::VideoLookupFailed
        },
        Details {
            elapsed_ms: Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64),
            failure: result.as_ref().err().map(diagnostics::Failure::error),
            ..Default::default()
        },
    );
    result
}

async fn lookup(
    value: &str,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Video> {
    let id = video_id(value)?;
    let read = async {
        let reply = client::query(&id, cancel, &route).await?;
        let access = access(&reply, &id)?;
        let source = source_url(&id)?;
        // With the page as the root, Twitch's playlist servers must be public.
        let mut network = hls::network::Network::via(&source, route.clone()).await?;
        let (final_url, text) = network
            .playlist(usher_url(&id, &access)?.as_str(), cancel)
            .await
            .context("Twitch did not open this video. Subscriber-only videos are not supported")?;
        Ok(Video {
            title: access.title,
            inspection: hls::inspection_from(source, &final_url, &text)?,
        })
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => bail!("Twitch lookup cancelled"),
        result = tokio::time::timeout(Duration::from_secs(60), read) => result.context("Twitch lookup timed out")?,
    }
}

#[cfg(test)]
mod tests;
