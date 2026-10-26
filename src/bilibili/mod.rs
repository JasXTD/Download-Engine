//! Bilibili video pages through Bilibili's own web API, as its player reads them:
//! WBI-signed view and playurl replies list separate MP4 video and audio streams,
//! which become an indexed DASH presentation for the shared media pipeline.
//! Anonymous only (Bilibili then offers up to 480p). The request flow follows
//! j4rviscmd/bilibili-downloader-gui (MIT); no code was copied from it.
//! Bangumi episode pages (bangumi/play/ep… and ss…) use the PGC season and
//! playurl replies, which list the same DASH streams.
mod client;

use crate::hls::{self, Inspection, Playlist};
use anyhow::{Context, Result, ensure};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct Video {
    pub title: String,
    pub inspection: Inspection,
}

/// A reviewed choice that survives expiring stream links: the page and the
/// representation keys, never a signed CDN URL. A Bangumi episode is saved by
/// its episode ID, with an empty `bvid` and page 1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub bvid: String,
    pub page: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<u64>,
    pub video: String,
    pub audio: Option<String>,
}

pub struct Preview {
    pub playlist: Playlist,
    pub selection: Selection,
}

enum Id {
    Bv(String),
    Av(u64),
    Episode(u64),
    Season(u64),
}
struct Reference {
    id: Id,
    page: u32,
}

pub fn is_url(value: &str) -> bool {
    reference(value).is_ok()
}

fn reference(value: &str) -> Result<Reference> {
    let url = crate::model::validate_url(value)?;
    ensure!(
        matches!(
            url.host_str(),
            Some("www.bilibili.com" | "bilibili.com" | "m.bilibili.com")
        ) && url.port().is_none(),
        "Use a Bilibili video or Bangumi link (bilibili.com/video/BV… or bilibili.com/bangumi/play/ep…)"
    );
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    if let ["bangumi", "play", id] = parts.as_slice() {
        let number = |prefix: &str| {
            id.strip_prefix(prefix)
                .filter(|n| (1..=12).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|n| n.parse::<u64>().ok())
                .filter(|n| *n > 0)
        };
        let id = if let Some(ep) = number("ep") {
            Id::Episode(ep)
        } else if let Some(ss) = number("ss") {
            Id::Season(ss)
        } else {
            anyhow::bail!("This Bilibili link does not identify one episode");
        };
        return Ok(Reference { id, page: 1 });
    }
    let ["video", id] = parts.as_slice() else {
        anyhow::bail!("This Bilibili link does not identify one video");
    };
    let id = if valid_bvid(id) {
        Id::Bv((*id).to_owned())
    } else if let Some(number) = id.strip_prefix("av").or_else(|| id.strip_prefix("AV")) {
        let aid = number
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0 && number.len() <= 12 && number.bytes().all(|b| b.is_ascii_digit()))
            .context("This Bilibili link does not identify one video")?;
        Id::Av(aid)
    } else {
        anyhow::bail!("This Bilibili link does not identify one video");
    };
    let page = match url.query_pairs().find(|(k, _)| k == "p") {
        Some((_, p)) => p
            .parse::<u32>()
            .ok()
            .filter(|p| (1..=1000).contains(p))
            .context("This Bilibili link names an invalid video part")?,
        None => 1,
    };
    Ok(Reference { id, page })
}

fn valid_bvid(id: &str) -> bool {
    id.len() == 12 && id.starts_with("BV") && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The stable page address saved with a download; ?p= only for later parts.
pub fn source_url(bvid: &str, page: u32) -> Result<String> {
    ensure!(
        valid_bvid(bvid) && (1..=1000).contains(&page),
        "Invalid Bilibili video ID"
    );
    Ok(if page == 1 {
        format!("https://www.bilibili.com/video/{bvid}/")
    } else {
        format!("https://www.bilibili.com/video/{bvid}/?p={page}")
    })
}

/// The stable page address saved with a Bangumi episode download.
pub fn episode_url(ep: u64) -> Result<String> {
    ensure!(
        ep > 0 && ep < 1_000_000_000_000,
        "Invalid Bilibili episode ID"
    );
    Ok(format!("https://www.bilibili.com/bangumi/play/ep{ep}"))
}

fn clean_title(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn media_url(value: &str) -> Result<Url> {
    let url = crate::model::validate_url(value)?;
    ensure!(
        url.scheme() == "https",
        "Bilibili returned a stream address outside HTTPS"
    );
    Ok(url)
}

/// Fetch metadata and the finite list of DASH qualities. Nothing is queued.
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
    let reference = reference(value)?;
    let read = async {
        let api = client::Api::open(cancel, &route).await?;
        let id = match &reference.id {
            Id::Bv(bvid) => client::Id::Bv(bvid),
            Id::Av(aid) => client::Id::Av(*aid),
            Id::Episode(ep) => {
                return episode(&api, client::SeasonId::Episode(*ep), cancel).await;
            }
            Id::Season(ss) => {
                return episode(&api, client::SeasonId::Season(*ss), cancel).await;
            }
        };
        let view = api.view(id, cancel).await?;
        ensure!(
            valid_bvid(&view.bvid),
            "Bilibili returned an invalid video ID"
        );
        let part = view
            .pages
            .iter()
            .find(|p| p.page == reference.page)
            .with_context(|| format!("This Bilibili video has no part {}", reference.page))?;
        let play = api.play(&view.bvid, part.cid, cancel).await?;
        let inspection = inspection(source_url(&view.bvid, reference.page)?, &play)?;
        let title = if view.pages.len() > 1 {
            format!("{} - P{} {}", view.title, part.page, part.part)
        } else {
            view.title
        };
        Ok(Video {
            title: clean_title(&title),
            inspection,
        })
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => anyhow::bail!("Bilibili lookup cancelled"),
        result = tokio::time::timeout(Duration::from_secs(60), read) => result.context("Bilibili lookup timed out")?,
    }
}

/// A Bangumi episode; a season link (ss…) opens its first main episode, as the
/// web player does for visitors.
async fn episode(
    api: &client::Api,
    id: client::SeasonId,
    cancel: &CancellationToken,
) -> Result<Video> {
    let wanted = match id {
        client::SeasonId::Episode(ep) => Some(ep),
        client::SeasonId::Season(_) => None,
    };
    let season = api.season(id, cancel).await?;
    let episode = match wanted {
        Some(ep) => season.all_episodes().find(|e| e.id == ep),
        None => season.episodes.iter().flatten().next(),
    }
    .context("This Bilibili series does not list that episode")?;
    let play = api.episode_play(episode.id, episode.cid, cancel).await?;
    Ok(Video {
        title: episode_title(&season.title, episode),
        inspection: inspection(episode_url(episode.id)?, &play)?,
    })
}

/// The DASH choices of one page. Bilibili's video servers expect that page as
/// the Referer, as its own player sends it; some mirrors refuse requests without.
fn inspection(source: String, play: &client::Play) -> Result<Inspection> {
    let mut inspection = crate::dash::parse(&source, &source, &manifest(play)?)?;
    inspection.page = Some(source);
    Ok(inspection)
}

/// "Series - 第1话 Name", from the episode's display title when Bilibili sends one.
fn episode_title(series: &str, episode: &client::Episode) -> String {
    let name = episode
        .show_title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "{} {}",
                episode.title.as_deref().unwrap_or_default(),
                episode.long_title.as_deref().unwrap_or_default()
            )
        });
    let name = clean_title(&name);
    clean_title(&if name.is_empty() {
        series.to_owned()
    } else {
        format!("{series} - {name}")
    })
}

/// Bilibili's DASH JSON as an MPD for the shared parser: one indexed SegmentBase
/// per stream, video before audio so representation keys stay stable.
fn manifest(play: &client::Play) -> Result<String> {
    let dash = play.dash.as_ref().context(
        "Bilibili offered this video only in its old single-file format, which is not supported",
    )?;
    let duration_ms = play
        .timelength
        .filter(|&ms| ms > 0)
        .or(dash.duration.map(|s| s.saturating_mul(1000)))
        .filter(|&ms| ms > 0 && ms <= 7 * 86400 * 1000)
        .context("Bilibili did not report the video length")?;
    let mut xml = format!(
        r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT{}.{:03}S"><Period>"#,
        duration_ms / 1000,
        duration_ms % 1000
    );
    let audio = dash.audio.as_deref().unwrap_or_default();
    for (kind, streams) in [("video", dash.video.as_slice()), ("audio", audio)] {
        let mut ids = Vec::new();
        let mut set = String::new();
        for stream in streams.iter().take(32) {
            let Some(segment) = &stream.segment_base else {
                continue;
            };
            if stream.mime_type != format!("{kind}/mp4") {
                continue;
            }
            let id = if kind == "video" {
                format!("{}-{}", stream.id, stream.codecid)
            } else {
                stream.id.to_string()
            };
            if ids.contains(&id) {
                continue;
            }
            let size = if kind == "video" && stream.width > 0 && stream.height > 0 {
                format!(r#" width="{}" height="{}""#, stream.width, stream.height)
            } else {
                String::new()
            };
            let label = if kind == "audio" {
                format!("<Label>{}</Label>", audio_name(&stream.codecs))
            } else {
                String::new()
            };
            set += &format!(
                r#"<Representation id="{id}" bandwidth="{}" codecs="{}"{size}>{label}<BaseURL>{}</BaseURL><SegmentBase indexRange="{}"><Initialization range="{}"/></SegmentBase></Representation>"#,
                stream.bandwidth,
                escape(&stream.codecs),
                escape(stream_url(stream)?),
                escape(&segment.index_range),
                escape(&segment.initialization),
            );
            ids.push(id);
        }
        if !set.is_empty() {
            xml += &format!(
                r#"<AdaptationSet contentType="{kind}" mimeType="{kind}/mp4">{set}</AdaptationSet>"#
            );
        }
    }
    xml += "</Period></MPD>";
    Ok(xml)
}

/// Prefer Bilibili's own CDN over peer-assisted mirrors (PCDN), which are often
/// slow or on unusual ports.
fn stream_url(stream: &client::Stream) -> Result<&str> {
    let peer = |url: &str| {
        Url::parse(url).is_ok_and(|u| {
            u.host_str()
                .is_some_and(|h| h.ends_with(".mcdn.bilivideo.cn") || h.ends_with(".szbdyd.com"))
        })
    };
    let urls: Vec<&str> = std::iter::once(stream.base_url.as_str())
        .chain(stream.backup_url.iter().flatten().map(String::as_str))
        .take(8)
        .collect();
    let url = urls
        .iter()
        .find(|u| !peer(u))
        .or(urls.first())
        .context("Bilibili returned a stream without an address")?;
    media_url(url)?;
    Ok(*url)
}

fn audio_name(codecs: &str) -> &'static str {
    match codecs.split('.').next() {
        Some("mp4a") => "AAC",
        Some("ec-3") => "Dolby Digital Plus",
        Some("fLaC") => "FLAC",
        _ => "Audio",
    }
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub async fn preview_via(
    inspection: &Inspection,
    variant: usize,
    audio: usize,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Preview> {
    let reference = reference(&inspection.source)?;
    let (bvid, page, episode) = match reference.id {
        Id::Bv(bvid) => (bvid, reference.page, None),
        Id::Episode(ep) => (String::new(), 1, Some(ep)),
        Id::Av(_) | Id::Season(_) => anyhow::bail!("Invalid Bilibili preview source"),
    };
    let playlist = hls::preview_with_audio_via(inspection, variant, audio, cancel, route).await?;
    let dash = playlist
        .dash
        .as_ref()
        .context("Missing Bilibili stream choice")?;
    let selection = Selection {
        bvid,
        page,
        episode,
        video: dash.video.clone(),
        audio: dash.audio.clone(),
    };
    selection.validate(&playlist)?;
    Ok(Preview {
        playlist,
        selection,
    })
}

impl Selection {
    /// The page this choice was read from: a video part or a Bangumi episode.
    pub fn source(&self) -> Result<String> {
        match self.episode {
            Some(ep) => {
                ensure!(
                    self.bvid.is_empty() && self.page == 1,
                    "Invalid saved Bilibili selection"
                );
                episode_url(ep)
            }
            None => source_url(&self.bvid, self.page),
        }
    }
    pub(crate) fn validate(&self, playlist: &Playlist) -> Result<()> {
        let dash = playlist
            .dash
            .as_ref()
            .context("Invalid saved Bilibili selection")?;
        ensure!(
            playlist.source == self.source()?
                && dash.video == self.video
                && dash.audio == self.audio
                && dash.has_index()
                && self.video.len() <= 64
                && self.audio.as_ref().is_none_or(|a| a.len() <= 64),
            "Invalid saved Bilibili selection"
        );
        for part in playlist
            .parts
            .iter()
            .chain(playlist.audio.iter().flat_map(|a| &a.parts))
        {
            media_url(&part.url)?;
        }
        Ok(())
    }
}

/// Stream links expire, but completed bytes may be reused only when the media
/// itself is unchanged: same length, byte ranges and SIDX index contents.
pub(crate) fn check_refresh(previous: &Playlist, current: &Playlist) -> Result<()> {
    let ranges = |p: &Playlist| {
        p.parts
            .iter()
            .chain(p.audio.iter().flat_map(|a| &a.parts))
            .map(|part| (part.range.clone(), part.initialization))
            .collect::<Vec<_>>()
    };
    let (old, new) = (
        previous.dash.as_ref().context("Missing DASH selection")?,
        current.dash.as_ref().context("Missing DASH selection")?,
    );
    let same_index = |a: &Option<crate::dash::IndexIdentity>,
                      b: &Option<crate::dash::IndexIdentity>| match (a, b) {
        (Some(a), Some(b)) => a.same_media(b),
        (None, None) => true,
        _ => false,
    };
    ensure!(
        previous.source == current.source
            && previous.duration_ms == current.duration_ms
            && ranges(previous) == ranges(current)
            && same_index(&old.video_index, &new.video_index)
            && same_index(&old.audio_index, &new.audio_index),
        "This Bilibili video changed. Preview it again before resuming."
    );
    Ok(())
}

/// Re-read fresh stream links from the saved page and choice before every start.
pub(crate) async fn refresh(
    download: &hls::Download,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Playlist> {
    let selection = download
        .bilibili
        .as_ref()
        .context("Missing Bilibili selection")?;
    let video = inspect_via(&selection.source()?, cancel, route.clone()).await?;
    let (variant, audio) = video
        .inspection
        .dash
        .as_ref()
        .and_then(|d| d.position(&selection.video, selection.audio.as_deref()))
        .context("The chosen Bilibili quality is no longer offered. Preview this video again.")?;
    let current = preview_via(&video.inspection, variant, audio, cancel, route).await?;
    ensure!(
        current.selection == *selection,
        "Bilibili selection changed. Preview this video again."
    );
    if !download.completed.is_empty() {
        check_refresh(&download.playlist, &current.playlist)?;
    }
    Ok(current.playlist)
}

#[cfg(test)]
mod tests;
