//! YouTube's anonymous VISIONOS player flow, adapted from yt-dlp 2026.08.19.
//! See docs/youtube-port.md for the source map and the deliberately supported subset.
//! No browser cookies, remote JavaScript execution, or Python subprocesses.
mod client;

use crate::hls::{self, Inspection, Playlist};
use anyhow::{Context, Result, ensure};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};
use tokio_util::sync::CancellationToken;

pub const UPSTREAM_VERSION: &str = "2026.08.19";

pub struct Video {
    pub title: String,
    pub inspection: Inspection,
}

/// Stable choices survive expiring CDN links. The signed URLs are never used
/// as a video/quality identity and never decide which quality to select next.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub video_id: String,
    pub video_itag: u32,
    pub audio_itag: Option<u32>,
    pub audio_name: Option<String>,
    pub audio_language: Option<String>,
    pub resolution: Option<(u32, u32)>,
    pub codecs: String,
}

pub struct Preview {
    pub playlist: Playlist,
    pub selection: Selection,
}

pub fn is_url(value: &str) -> bool {
    crate::model::validate_url(value).is_ok_and(|url| official_host(&url))
}

fn official_host(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some(
            "youtube.com"
                | "www.youtube.com"
                | "m.youtube.com"
                | "music.youtube.com"
                | "youtu.be"
                | "www.youtu.be"
                | "youtube-nocookie.com"
                | "www.youtube-nocookie.com"
        )
    )
}

fn valid_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn video_id(value: &str) -> Result<String> {
    let url = crate::model::validate_url(value)?;
    ensure!(
        official_host(&url) && url.port().is_none(),
        "Use a YouTube watch, Shorts, embed, or youtu.be video link"
    );
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    let id = if matches!(url.host_str(), Some("youtu.be" | "www.youtu.be")) && parts.len() == 1 {
        Some(parts[0].to_owned())
    } else if matches!(parts.as_slice(), ["watch"]) {
        // Like yt-dlp, use the first v parameter. A playlist never changes it.
        url.query_pairs()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.into_owned())
    } else if let ["shorts" | "embed" | "v" | "live", id] = parts.as_slice() {
        Some((*id).to_owned())
    } else {
        None
    };
    let id = id
        .filter(|id| valid_id(id))
        .context("This YouTube link does not identify one video")?;
    Ok(id)
}

pub fn watch_url(id: &str) -> Result<String> {
    ensure!(valid_id(id), "Invalid YouTube video ID");
    Ok(format!("https://www.youtube.com/watch?v={id}"))
}

pub(crate) fn media_url(value: &str) -> Result<Url> {
    let url = crate::model::validate_url(value)?;
    ensure!(
        url.scheme() == "https"
            && url.port().is_none()
            && url
                .host_str()
                .is_some_and(|h| h == "googlevideo.com" || h.ends_with(".googlevideo.com")),
        "YouTube returned a video address outside its HTTPS media servers"
    );
    ensure!(
        !url.query_pairs().any(|(key, _)| key == "n")
            && !url
                .path_segments()
                .is_some_and(|mut p| p.any(|part| part == "n")),
        "This YouTube format needs a JavaScript player challenge that this Rust extractor does not support yet"
    );
    Ok(url)
}

fn path_fields(value: &str) -> Result<BTreeMap<String, String>> {
    let url = media_url(value)?;
    let parts: Vec<_> = url
        .path_segments()
        .context("Invalid YouTube media path")?
        .collect();
    let start = parts
        .iter()
        .position(|p| *p == "id")
        .context("YouTube media identity is missing")?;
    let mut fields = BTreeMap::new();
    let (pairs, remainder) = parts[start..].as_chunks::<2>();
    ensure!(remainder.is_empty(), "Unpaired YouTube media field");
    for pair in pairs {
        ensure!(
            fields
                .insert(
                    pair[0].to_owned(),
                    percent_encoding::percent_decode_str(pair[1])
                        .decode_utf8()?
                        .into_owned()
                )
                .is_none(),
            "Duplicate YouTube media field"
        );
    }
    Ok(fields)
}

fn itag(value: &str) -> Result<u32> {
    let fields = path_fields(value)?;
    let tag = fields
        .get("itag")
        .context("YouTube quality identifier is missing")?
        .parse::<u32>()?;
    ensure!(tag > 0, "Invalid YouTube quality identifier");
    Ok(tag)
}

/// Fetch metadata and a finite, unencrypted HLS quality list. Loading this list
/// cannot enqueue a job. All network and body reads have cancellation and limits.
pub async fn inspect(value: &str, cancel: &CancellationToken) -> Result<Video> {
    inspect_via(value, cancel, crate::proxy::Route::direct()).await
}
pub async fn inspect_via(
    value: &str,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Video> {
    use crate::diagnostics::{self, Details, Event, Level};
    let started = std::time::Instant::now();
    diagnostics::record(Level::Info, Event::VideoLookupStarted, Details::default());
    let result = inspect_inner(value, cancel, route).await;
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

async fn inspect_inner(
    value: &str,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Video> {
    let id = video_id(value)?;
    let source = watch_url(&id)?;
    let read = async {
        let player = client::extract(&id, cancel, &route).await?;
        let mut inspection = hls::inspect_via(&player.manifest, cancel, route).await?;
        ensure!(
            inspection.dash.is_none()
                && inspection.playlist.is_none()
                && !inspection.variants.is_empty(),
            "YouTube did not return a video quality list"
        );
        for variant in &inspection.variants {
            itag(&variant.url)?;
            for audio in &variant.audio {
                if let Some(url) = &audio.url {
                    itag(url)?;
                }
            }
        }
        inspection.source = source;
        Ok(Video {
            title: player.title,
            inspection,
        })
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => anyhow::bail!("YouTube lookup cancelled"),
        result = tokio::time::timeout(Duration::from_secs(60), read) => result.context("YouTube lookup timed out")?,
    }
}

pub async fn preview(
    inspection: &Inspection,
    variant: usize,
    audio: usize,
    cancel: &CancellationToken,
) -> Result<Preview> {
    preview_via(
        inspection,
        variant,
        audio,
        cancel,
        crate::proxy::Route::direct(),
    )
    .await
}
pub async fn preview_via(
    inspection: &Inspection,
    variant: usize,
    audio: usize,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Preview> {
    let choice = inspection
        .variants
        .get(variant)
        .context("Choose a listed YouTube quality")?;
    let track = choice.audio.get(audio);
    ensure!(
        track.is_some() || (choice.audio.is_empty() && audio == 0),
        "Choose a listed audio track"
    );
    let selection = Selection {
        video_id: video_id(&inspection.source)?,
        video_itag: itag(&choice.url)?,
        audio_itag: track.and_then(|a| a.url.as_deref()).map(itag).transpose()?,
        audio_name: track.map(|a| a.name.clone()),
        audio_language: track.and_then(|a| a.language.clone()),
        resolution: choice.resolution,
        codecs: choice.codecs.clone(),
    };
    let mut playlist =
        hls::preview_with_audio_via(inspection, variant, audio, cancel, route).await?;
    // yt-dlp's HLS formats 233/234 carry ADTS AAC with ID3 transport timestamps,
    // although their playlist has no EXT-X-MAP or container declaration.
    if matches!(selection.audio_itag, Some(233 | 234))
        && let Some(audio) = &mut playlist.audio
    {
        ensure!(
            audio.format == hls::Format::TransportStream,
            "Unexpected YouTube AAC container declaration"
        );
        audio.format = hls::Format::PackedAac;
    }
    selection.validate(&playlist)?;
    Ok(Preview {
        playlist,
        selection,
    })
}

impl Selection {
    pub(crate) fn validate(&self, playlist: &Playlist) -> Result<()> {
        ensure!(
            playlist.source == watch_url(&self.video_id)?
                && playlist.dash.is_none()
                && self.video_itag > 0
                && self.video_itag == itag(&playlist.url)?
                && self.codecs.len() <= 200
                && self.audio_name.as_ref().is_none_or(|s| s.len() <= 200)
                && self.audio_language.as_ref().is_none_or(|s| s.len() <= 64)
                && self
                    .resolution
                    .is_none_or(|(w, h)| w > 0 && h > 0 && w <= 65536 && h <= 65536),
            "Invalid saved YouTube selection"
        );
        ensure!(
            self.audio_itag == playlist.audio.as_ref().map(|a| itag(&a.url)).transpose()?,
            "Saved YouTube audio selection changed"
        );
        if let Some(audio) = &playlist.audio {
            ensure!(
                self.audio_name.as_deref() == Some(audio.name.as_str())
                    && self.audio_language == audio.language,
                "Saved YouTube audio description changed"
            );
        }
        for part in playlist
            .parts
            .iter()
            .chain(playlist.audio.iter().flat_map(|a| &a.parts))
        {
            media_url(&part.url)?;
        }
        Ok(())
    }

    fn indices(&self, inspection: &Inspection) -> Result<(usize, usize)> {
        let mut matches = Vec::new();
        for (index, variant) in inspection.variants.iter().enumerate() {
            if itag(&variant.url)? != self.video_itag
                || variant.resolution != self.resolution
                || variant.codecs != self.codecs
            {
                continue;
            }
            if variant.audio.is_empty() && self.audio_name.is_none() && self.audio_itag.is_none() {
                matches.push((index, 0));
            }
            for (audio_index, audio) in variant.audio.iter().enumerate() {
                if audio.url.as_deref().map(itag).transpose()? == self.audio_itag
                    && self.audio_name.as_deref() == Some(audio.name.as_str())
                    && self.audio_language == audio.language
                {
                    matches.push((index, audio_index));
                }
            }
        }
        ensure!(
            matches.len() == 1,
            "The selected YouTube quality or audio changed. Preview this video again."
        );
        Ok(matches[0])
    }
}

/// Signed links can change, but the encoding version and exact segment mapping
/// must remain identical before any completed bytes may be reused.
fn content_identity(url: &str) -> Result<BTreeMap<String, String>> {
    let fields = path_fields(url)?;
    let versioned = ["sgovp", "sgoap"].iter().any(|key| {
        fields
            .get(*key)
            .is_some_and(|v| v.split(';').any(|p| p.starts_with("lmt=") && p.len() > 4))
    }) || fields.get("lmt").is_some_and(|v| !v.is_empty());
    ensure!(
        versioned
            && fields.contains_key("id")
            && fields.contains_key("itag")
            && (fields.contains_key("gosq") || fields.contains_key("sq")),
        "YouTube did not provide a stable segment version. Start this download again to refresh it safely."
    );
    Ok(fields
        .into_iter()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                "id" | "itag"
                    | "sgovp"
                    | "sgoap"
                    | "govp"
                    | "goap"
                    | "gosq"
                    | "sq"
                    | "file"
                    | "lmt"
                    | "clen"
                    | "dur"
                    | "mime"
                    | "gir"
                    | "range"
                    | "xtags"
            )
        })
        .collect())
}

pub(crate) fn check_refresh(previous: &Playlist, current: &Playlist) -> Result<()> {
    ensure!(
        previous.source == current.source
            && previous.format == current.format
            && previous.duration_ms == current.duration_ms
            && previous.sequence == current.sequence
            && previous.parts.len() == current.parts.len()
            && previous.audio.is_some() == current.audio.is_some(),
        "YouTube video changed. Preview this video again before resuming."
    );
    if let (Some(old), Some(new)) = (&previous.audio, &current.audio) {
        ensure!(
            old.format == new.format
                && old.duration_ms == new.duration_ms
                && old.sequence == new.sequence
                && old.parts.len() == new.parts.len()
                && old.name == new.name
                && old.language == new.language,
            "YouTube audio changed. Preview this video again before resuming."
        );
    }
    let parts = |p: &Playlist| {
        p.parts
            .iter()
            .chain(p.audio.iter().flat_map(|a| &a.parts))
            .cloned()
            .collect::<Vec<_>>()
    };
    for (old, new) in parts(previous).iter().zip(parts(current)) {
        ensure!(
            old.range == new.range
                && old.initialization == new.initialization
                && content_identity(&old.url)? == content_identity(&new.url)?,
            "YouTube segments changed. Preview this video again before resuming."
        );
    }
    Ok(())
}

pub(crate) async fn refresh(
    download: &hls::Download,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Playlist> {
    let selection = download
        .youtube
        .as_ref()
        .context("Missing YouTube selection")?;
    let video = inspect_via(&watch_url(&selection.video_id)?, cancel, route.clone()).await?;
    let (variant, audio) = selection.indices(&video.inspection)?;
    let current = preview_via(&video.inspection, variant, audio, cancel, route).await?;
    ensure!(
        current.selection == *selection,
        "YouTube selection changed. Preview this video again."
    );
    if download.completed_count() != 0 {
        check_refresh(&download.playlist, &current.playlist)?;
    }
    Ok(current.playlist)
}

#[cfg(test)]
mod tests;
