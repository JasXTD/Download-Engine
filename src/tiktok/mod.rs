//! TikTok video pages, read as the web page itself does: the page carries the
//! video's qualities and sets anonymous visitor cookies, which TikTok's video
//! servers require together with a TikTok Referer. Each quality is one ordinary
//! MP4 with sound, fetched in byte-range parts and joined unchanged. No account
//! cookies are used; private and sign-in-only videos are not supported.
mod client;

use crate::hls::{self, Format, Inspection, Part, Playlist, Range, Variant};
use anyhow::{Context, Result, ensure};
use reqwest::{Url, header};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Byte-range size of the resumable parts of one MP4 file.
const PART_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const REFERER: &str = "https://www.tiktok.com/";

pub struct Video {
    pub title: String,
    pub inspection: Inspection,
}

/// The qualities a TikTok page offered, in the order of `Inspection::variants`.
#[derive(Clone, Debug)]
pub struct Listing {
    video_id: String,
    author: String,
    duration_ms: u64,
    qualities: Vec<Quality>,
}
#[derive(Clone, Debug)]
struct Quality {
    gear: String,
    url: String,
    size: u64,
    hash: String,
    codec: String,
    bitrate: u64,
    resolution: Option<(u32, u32)>,
}

/// A reviewed choice that survives expiring links: the video and the exact file,
/// never a signed URL or a cookie.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub video_id: String,
    pub author: String,
    pub gear: String,
    pub size: u64,
    pub hash: String,
}

pub struct Preview {
    pub playlist: Playlist,
    pub selection: Selection,
}

struct Reference {
    author: String,
    video_id: String,
}

pub fn is_url(value: &str) -> bool {
    reference(value).is_ok()
}

fn reference(value: &str) -> Result<Reference> {
    let url = crate::model::validate_url(value)?;
    ensure!(
        matches!(
            url.host_str(),
            Some("www.tiktok.com" | "tiktok.com" | "m.tiktok.com")
        ) && url.port().is_none(),
        "Use a TikTok video link (tiktok.com/@user/video/…)"
    );
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    let [user, "video", id] = parts.as_slice() else {
        anyhow::bail!("This TikTok link does not identify one video");
    };
    let author = user
        .strip_prefix('@')
        .filter(|a| valid_author(a))
        .context("This TikTok link does not identify one video")?;
    ensure!(valid_id(id), "This TikTok link does not identify one video");
    Ok(Reference {
        author: author.into(),
        video_id: (*id).into(),
    })
}

fn valid_id(id: &str) -> bool {
    (1..=25).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}
/// TikTok usernames; an empty one still finds the video, so it is allowed.
fn valid_author(author: &str) -> bool {
    author.len() <= 64
        && author
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_')
}

/// The stable page address saved with a download.
pub fn source_url(author: &str, video_id: &str) -> Result<String> {
    ensure!(
        valid_author(author) && valid_id(video_id),
        "Invalid TikTok video ID"
    );
    Ok(format!("https://www.tiktok.com/@{author}/video/{video_id}"))
}

/// Video addresses must stay on TikTok's own HTTPS servers: the page's cookies
/// are sent only there.
fn media_url(value: &str) -> Result<Url> {
    let url = crate::model::validate_url(value)?;
    ensure!(
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|host| { host == "tiktok.com" || host.ends_with(".tiktok.com") }),
        "TikTok returned a video address outside its own servers"
    );
    Ok(url)
}

/// TikTok sends some numbers as strings (for example "DataSize": "4429719").
fn number(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

fn listing(item: &Value, video_id: &str) -> Result<(String, Listing)> {
    let text = |pointer: &str| item.pointer(pointer).and_then(Value::as_str).unwrap_or("");
    ensure!(text("/id") == video_id, "TikTok returned a different video");
    ensure!(
        item.get("imagePost").is_none_or(Value::is_null),
        "TikTok photo posts are not supported yet. Choose a video."
    );
    let author = text("/author/uniqueId");
    ensure!(
        valid_author(author),
        "TikTok returned an invalid author name"
    );
    let duration_ms = item
        .pointer("/video/duration")
        .and_then(Value::as_u64)
        .filter(|&s| s > 0 && s <= 86400)
        .context("TikTok did not report the video length")?
        * 1000;
    let mut qualities = Vec::new();
    for rate in item
        .pointer("/video/bitrateInfo")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(16)
    {
        let play = &rate["PlayAddr"];
        let Some(url) = play["UrlList"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .find(|url| media_url(url).is_ok())
        else {
            continue;
        };
        let size = number(&play["DataSize"]).unwrap_or(0);
        let gear = rate["GearName"].as_str().unwrap_or("");
        let hash = play["FileHash"].as_str().unwrap_or("");
        if !(1..=MAX_FILE_BYTES).contains(&size)
            || gear.is_empty()
            || gear.len() > 128
            || hash.len() > 128
            || qualities.iter().any(|q: &Quality| q.gear == gear)
        {
            continue;
        }
        let codec = match rate["CodecType"].as_str().unwrap_or("") {
            "h264" => "avc1",
            "h265" | "h265_hvc1" | "bytevc1" => "hvc1",
            other => other,
        };
        let resolution = match (number(&play["Width"]), number(&play["Height"])) {
            (Some(w @ 1..=16384), Some(h @ 1..=16384)) => Some((w as u32, h as u32)),
            _ => None,
        };
        qualities.push(Quality {
            gear: gear.into(),
            url: url.into(),
            size,
            hash: hash.into(),
            codec: codec.chars().take(32).collect(),
            bitrate: number(&rate["Bitrate"]).unwrap_or(0),
            resolution,
        });
    }
    ensure!(
        !qualities.is_empty(),
        "TikTok did not offer a downloadable quality for this video"
    );
    // Largest picture first, then the higher bitrate; H.264 plays almost anywhere.
    qualities.sort_by_key(|q| {
        (
            std::cmp::Reverse(q.resolution.map_or(0, |(w, h)| u64::from(w) * u64::from(h))),
            q.codec != "avc1",
            std::cmp::Reverse(q.bitrate),
        )
    });
    let description = text("/desc").trim();
    let title = if description.is_empty() {
        format!("TikTok @{author} {video_id}")
    } else {
        description.into()
    };
    Ok((
        title
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect(),
        Listing {
            video_id: video_id.into(),
            author: author.into(),
            duration_ms,
            qualities,
        },
    ))
}

/// The qualities as choices for the shared quality dialog.
fn inspection(listing: Listing) -> Result<Inspection> {
    Ok(Inspection {
        public_only: false,
        source: source_url(&listing.author, &listing.video_id)?,
        variants: listing
            .qualities
            .iter()
            .map(|q| Variant {
                url: q.url.clone(),
                bandwidth: q.bitrate,
                resolution: q.resolution,
                codecs: q.codec.clone(),
                unsupported: None,
                audio: Vec::new(),
            })
            .collect(),
        playlist: None,
        dash: None,
        tiktok: Some(listing),
        page: None,
    })
}

/// Fetch the page and list its qualities. Nothing is queued.
pub async fn inspect_via(
    value: &str,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Video> {
    use crate::diagnostics::{self, Details, Event, Level};
    let started = std::time::Instant::now();
    diagnostics::record(Level::Info, Event::VideoLookupStarted, Details::default());
    let result = lookup(value, cancel, route).await.map(|(video, _)| video);
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
) -> Result<(Video, String)> {
    let reference = reference(value)?;
    let read = async {
        let page = client::page(
            &source_url(&reference.author, &reference.video_id)?,
            cancel,
            &route,
        )
        .await?;
        let (title, listing) = listing(&page.item, &reference.video_id)?;
        Ok((
            Video {
                title,
                inspection: inspection(listing)?,
            },
            page.cookies,
        ))
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => anyhow::bail!("TikTok lookup cancelled"),
        result = tokio::time::timeout(Duration::from_secs(60), read) => result.context("TikTok lookup timed out")?,
    }
}

/// Build the download from a listed quality. No network request is needed.
pub fn preview(inspection: &Inspection, variant: usize) -> Result<Preview> {
    let listing = inspection
        .tiktok
        .as_ref()
        .context("Load the TikTok qualities first")?;
    let quality = listing
        .qualities
        .get(variant)
        .context("Choose a listed TikTok quality")?;
    let label = inspection
        .variants
        .get(variant)
        .map(Variant::label)
        .context("Choose a listed TikTok quality")?;
    let parts = (0..quality.size.div_ceil(PART_BYTES))
        .map(|i| {
            let start = i * PART_BYTES;
            Part {
                url: quality.url.clone(),
                range: Some(Range {
                    start,
                    length: PART_BYTES.min(quality.size - start),
                }),
                initialization: false,
            }
        })
        .collect();
    let playlist = Playlist {
        public_only: false,
        source: inspection.source.clone(),
        url: quality.url.clone(),
        quality: label,
        bandwidth: Some(quality.bitrate).filter(|&b| b > 0),
        duration_ms: listing.duration_ms,
        sequence: 0,
        format: Format::Progressive,
        parts,
        audio: None,
        dash: None,
    };
    let selection = Selection {
        video_id: listing.video_id.clone(),
        author: listing.author.clone(),
        gear: quality.gear.clone(),
        size: quality.size,
        hash: quality.hash.clone(),
    };
    playlist.validate()?;
    selection.validate(&playlist)?;
    Ok(Preview {
        playlist,
        selection,
    })
}

impl Selection {
    pub(crate) fn validate(&self, playlist: &Playlist) -> Result<()> {
        let total: u64 = playlist
            .parts
            .iter()
            .filter_map(|p| p.range.as_ref().map(|r| r.length))
            .sum();
        ensure!(
            playlist.source == source_url(&self.author, &self.video_id)?
                && playlist.format == Format::Progressive
                && total == self.size
                && !self.gear.is_empty()
                && self.gear.len() <= 128
                && self.hash.len() <= 128,
            "Invalid saved TikTok selection"
        );
        for part in &playlist.parts {
            media_url(&part.url)?;
        }
        Ok(())
    }
}

/// Re-read the page before every start: TikTok's links and visitor cookies expire.
/// Completed parts are kept only when it is still the very same file.
pub(crate) async fn refresh(
    download: &hls::Download,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<(Playlist, hls::SiteHeaders)> {
    let selection = download
        .tiktok
        .as_ref()
        .context("Missing TikTok selection")?;
    let (video, cookies) = lookup(
        &source_url(&selection.author, &selection.video_id)?,
        cancel,
        route,
    )
    .await?;
    let listing = video
        .inspection
        .tiktok
        .as_ref()
        .context("Missing TikTok qualities")?;
    let index = listing
        .qualities
        .iter()
        .position(|q| {
            q.gear == selection.gear && q.size == selection.size && q.hash == selection.hash
        })
        .context(
            "This TikTok video changed or that quality is no longer offered. Preview it again.",
        )?;
    let mut current = preview(&video.inspection, index)?;
    // The author's name may have changed; the saved page address still works.
    current
        .playlist
        .source
        .clone_from(&download.playlist.source);
    current.selection.author.clone_from(&selection.author);
    ensure!(
        current.selection == *selection,
        "TikTok selection changed. Preview this video again."
    );
    let ranges = |p: &Playlist| {
        p.parts
            .iter()
            .map(|part| part.range.clone())
            .collect::<Vec<_>>()
    };
    ensure!(
        download.completed.is_empty() || ranges(&download.playlist) == ranges(&current.playlist),
        "This TikTok video changed. Preview it again before resuming."
    );
    current.selection.validate(&current.playlist)?;
    Ok((current.playlist, site_headers(&cookies)?))
}

pub(crate) fn site_headers(cookies: &str) -> Result<hls::SiteHeaders> {
    let mut headers = header::HeaderMap::new();
    headers.insert(header::REFERER, header::HeaderValue::from_static(REFERER));
    if !cookies.is_empty() {
        headers.insert(header::COOKIE, header::HeaderValue::from_str(cookies)?);
    }
    Ok(hls::SiteHeaders {
        domain: "tiktok.com",
        headers,
    })
}

#[cfg(test)]
mod tests;
