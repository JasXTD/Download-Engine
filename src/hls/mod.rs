//! Bounded, anonymous HLS and DASH previews and downloads. Never fetch encryption keys.
mod aac;
pub(crate) mod merge;
mod mp4;
pub(crate) mod network;
mod parser;
mod store;
mod transfer;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio_util::sync::CancellationToken;

pub(crate) use network::SiteHeaders;
pub use parser::{AudioRendition, AudioTrack, Format, Part, Playlist, Range, Variant};
pub(crate) use store::initialize;
pub(crate) use transfer::transfer;

pub const MAX_PARTS: usize = 10_000;
pub const MAX_PLAYLIST_BYTES: usize = 1024 * 1024;
pub const MAX_PART_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletedPart {
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Download {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_relay: Option<crate::browser_relay::Session>,
    /// Keep the reviewed browser identity for playlist checks and resumed parts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    pub version: u8,
    pub playlist: Playlist,
    #[serde(default)]
    pub completed: Vec<CompletedPart>,
    /// Verified parts beyond the contiguous prefix. Older saved jobs omit this.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub completed_ahead: BTreeMap<usize, CompletedPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub youtube: Option<crate::youtube::Selection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bilibili: Option<crate::bilibili::Selection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiktok: Option<crate::tiktok::Selection>,
}
impl Download {
    pub fn new(playlist: Playlist) -> Self {
        Self {
            user_agent: None,
            browser_relay: None,
            version: if playlist.dash.as_ref().is_some_and(|d| d.has_index()) {
                4
            } else if playlist.dash.is_some() {
                3
            } else if playlist.audio.is_some() {
                2
            } else {
                1
            },
            playlist,
            completed: Vec::new(),
            completed_ahead: BTreeMap::new(),
            youtube: None,
            bilibili: None,
            tiktok: None,
        }
    }
    pub fn completed_count(&self) -> usize {
        self.completed.len() + self.completed_ahead.len()
    }
    pub fn saved_part(&self, index: usize) -> Option<&CompletedPart> {
        self.completed
            .get(index)
            .or_else(|| self.completed_ahead.get(&index))
    }
    pub fn saved_bytes(&self) -> u64 {
        self.completed
            .iter()
            .chain(self.completed_ahead.values())
            .map(|part| part.size)
            .sum()
    }
    pub(crate) fn remember_part(&mut self, index: usize, part: CompletedPart) -> Result<()> {
        ensure!(
            index < self.playlist.part_count() && self.saved_part(index).is_none(),
            "Invalid completed video part"
        );
        if index == self.completed.len() {
            self.completed.push(part);
            while let Some(next) = self.completed_ahead.remove(&self.completed.len()) {
                self.completed.push(next);
            }
        } else {
            self.completed_ahead.insert(index, part);
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        crate::model::validate_browser_user_agent(self.user_agent.as_deref())?;
        if let Some(session) = &self.browser_relay {
            session.validate()?;
            ensure!(
                !self.playlist.public_only
                    && self.youtube.is_none()
                    && self.bilibili.is_none()
                    && self.tiktok.is_none(),
                "Invalid browser video transport context"
            );
        }
        // Version 6 is an indexed DASH download re-resolved from a Bilibili page;
        // version 7 is a single MP4 file re-resolved from a TikTok page.
        ensure!(
            matches!(self.version, 1..=7)
                && (self.version >= 2 || self.playlist.audio.is_none())
                && matches!(self.version, 3 | 4 | 6) == self.playlist.dash.is_some()
                && matches!(self.version, 4 | 6)
                    == self.playlist.dash.as_ref().is_some_and(|d| d.has_index())
                && (self.version == 5) == self.youtube.is_some()
                && (self.version == 6) == self.bilibili.is_some()
                && (self.version == 7) == self.tiktok.is_some()
                && (self.version == 7) == (self.playlist.format == Format::Progressive),
            "Unsupported saved HLS download version"
        );
        self.playlist.validate()?;
        if let Some(youtube) = &self.youtube {
            youtube.validate(&self.playlist)?;
        }
        if let Some(bilibili) = &self.bilibili {
            bilibili.validate(&self.playlist)?;
        }
        if let Some(tiktok) = &self.tiktok {
            tiktok.validate(&self.playlist)?;
        }
        ensure!(
            self.playlist.format != Format::PackedAac
                && (self.youtube.is_some()
                    || self
                        .playlist
                        .audio
                        .as_ref()
                        .is_none_or(|a| a.format != Format::PackedAac)),
            "Packed AAC is only supported for reviewed YouTube audio"
        );
        ensure!(
            self.completed.len() <= self.playlist.part_count()
                && self
                    .completed_ahead
                    .keys()
                    .all(|index| *index > self.completed.len()
                        && *index < self.playlist.part_count()),
            "Invalid saved HLS progress"
        );
        for part in self.completed.iter().chain(self.completed_ahead.values()) {
            ensure!(
                part.size > 0
                    && part.size <= MAX_PART_BYTES
                    && part.sha256.len() == 64
                    && part.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid saved HLS segment"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Inspection {
    pub browser_relay: Option<crate::browser_relay::Session>,
    pub user_agent: Option<String>,
    /// Retained through quality selection and saved downloads for site readers.
    pub public_only: bool,
    pub source: String,
    pub variants: Vec<Variant>,
    pub playlist: Option<Playlist>,
    pub dash: Option<crate::dash::Manifest>,
    /// A TikTok page's qualities, for building its single-file download.
    pub tiktok: Option<crate::tiktok::Listing>,
    /// The browser page a detected stream came from; its origin is the Referer.
    pub page: Option<String>,
}

/// Video pages that a built-in site reader turns into stream choices.
pub fn is_site_video(value: &str) -> bool {
    crate::youtube::is_url(value)
        || crate::bilibili::is_url(value)
        || crate::tiktok::is_url(value)
        || crate::twitch::is_url(value)
}

pub fn is_playlist_url(value: &str) -> bool {
    crate::model::validate_url(value).is_ok_and(|url| {
        let path = url.path().to_ascii_lowercase();
        path.ends_with(".m3u8") || path.ends_with(".mpd")
    })
}

pub async fn inspect(url: &str, cancel: &CancellationToken) -> Result<Inspection> {
    inspect_via(url, cancel, crate::proxy::Route::direct()).await
}
pub async fn inspect_via(
    url: &str,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Inspection> {
    inspect_page_via(url, None, cancel, route).await
}
/// Inspect a stream the browser found on `page`. The page's origin is sent as
/// the Referer, here and when previewing and downloading the chosen quality.
pub async fn inspect_page_via(
    url: &str,
    page: Option<&str>,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Inspection> {
    inspect_with_policy(url, page, cancel, route, false).await
}

/// Reader-returned URLs are untrusted even when they are the stream's root.
pub async fn inspect_reader_via(
    url: &str,
    page: &str,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Inspection> {
    inspect_with_policy(url, Some(page), cancel, route, true).await
}

async fn inspect_with_policy(
    url: &str,
    page: Option<&str>,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
    public_only: bool,
) -> Result<Inspection> {
    inspect_candidates_via(url, &[], page, public_only, cancel, route).await
}

/// Try bounded entry points retained by the browser for one selected video.
/// Access rejections and responses that are not playlists permit another entry
/// point. Errors inside an identified playlist, cancellation and network-scope
/// failures must remain visible. All attempts use
/// the original root's network policy, including redirect/DNS/proxy checks.
pub async fn inspect_candidates_via(
    url: &str,
    fallbacks: &[String],
    page: Option<&str>,
    public_only: bool,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Inspection> {
    inspect_browser_via(url, fallbacks, page, None, public_only, cancel, route).await
}

/// Preserve the browser's non-secret identity without importing login cookies
/// or changing the existing network, redirect, proxy or public-only policy.
pub async fn inspect_browser_via(
    url: &str,
    fallbacks: &[String],
    page: Option<&str>,
    user_agent: Option<&str>,
    public_only: bool,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Inspection> {
    inspect_browser_relay_via(
        url,
        fallbacks,
        BrowserContext {
            page,
            user_agent,
            relay: None,
        },
        public_only,
        cancel,
        route,
    )
    .await
}

/// Context captured by the extension for one explicit video review.
pub struct BrowserContext<'a> {
    pub page: Option<&'a str>,
    pub user_agent: Option<&'a str>,
    pub relay: Option<&'a crate::browser_relay::Session>,
}

pub async fn inspect_browser_relay_via(
    url: &str,
    fallbacks: &[String],
    context: BrowserContext<'_>,
    public_only: bool,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Inspection> {
    use crate::diagnostics::{self, Details, Event, Failure, Level};
    let BrowserContext {
        page,
        user_agent,
        relay: browser_relay,
    } = context;
    crate::model::validate_browser_user_agent(user_agent)?;
    ensure!(fallbacks.len() <= 4, "Too many video alternatives");
    let source = crate::model::validate_url(url)?.to_string();
    let mut candidates = vec![source.clone()];
    for fallback in fallbacks {
        ensure!(fallback.len() <= 4096, "Video alternative URL is too long");
        let candidate = crate::model::validate_url(fallback)?.to_string();
        ensure!(
            !candidates.contains(&candidate),
            "Duplicate video alternative"
        );
        candidates.push(candidate);
    }
    let page = page
        .map(|page| crate::model::validate_url(page).map(|page| page.to_string()))
        .transpose()?;
    let mut network = network::Network::via(&source, route)
        .await?
        .with_page(page.as_deref())
        .with_user_agent(user_agent)?
        .with_public_only(public_only)?
        .with_browser_relay(browser_relay)?;
    for (index, candidate) in candidates.iter().enumerate() {
        ensure!(!cancel.is_cancelled(), "Video stream preview cancelled");
        let attempt = Some((index + 1) as u8);
        diagnostics::record(
            Level::Info,
            Event::VideoLookupStarted,
            Details {
                attempt,
                ..Default::default()
            },
        );
        let result = async {
            let (final_url, text) = network.playlist(candidate, cancel).await?;
            inspection_from(candidate.clone(), &final_url, &text)
        }
        .await;
        match result {
            Ok(mut inspection) => {
                inspection.page = page;
                inspection.user_agent = user_agent.map(str::to_owned);
                inspection.browser_relay = network.browser_relay().cloned();
                inspection.public_only = public_only;
                if let Some(playlist) = &mut inspection.playlist {
                    playlist.public_only = public_only;
                }
                diagnostics::record(
                    Level::Info,
                    Event::VideoLookupSucceeded,
                    Details {
                        attempt,
                        ..Default::default()
                    },
                );
                return Ok(inspection);
            }
            Err(error) => {
                diagnostics::record(
                    Level::Warn,
                    Event::VideoLookupFailed,
                    Details {
                        attempt,
                        failure: Some(Failure::error(&error)),
                        ..Default::default()
                    },
                );
                if cancel.is_cancelled() {
                    anyhow::bail!("Video stream preview cancelled");
                }
                let can_try_alternative =
                    error.is::<network::AccessDenied>() || error.is::<NotPlaylistResponse>();
                if !can_try_alternative || index + 1 == candidates.len() {
                    return Err(if index > 0 {
                        error.context(format!(
                            "No usable stream after {} same-video attempts",
                            index + 1
                        ))
                    } else {
                        error
                    });
                }
            }
        }
    }
    unreachable!("the primary candidate is always present")
}

/// A signed entry point can return an error page with a successful HTTP status.
/// Keep this distinct from invalid/unsupported content inside a real playlist:
/// only the former allows another browser-supplied entry point to be tried.
#[derive(Debug)]
struct NotPlaylistResponse;
impl std::fmt::Display for NotPlaylistResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("This link did not return a video playlist. Reload the video page, start playback, and try again.")
    }
}
impl std::error::Error for NotPlaylistResponse {}

/// Parse a playlist or DASH manifest that was already fetched from `final_url`.
pub(crate) fn inspection_from(source: String, final_url: &str, text: &str) -> Result<Inspection> {
    if crate::dash::is_manifest_document(text) {
        return crate::dash::parse(&source, final_url, text);
    }
    // Signed stream URLs can expire into HTML/XML error responses with HTTP 200.
    // A leading '<' or .mpd suffix alone does not identify a DASH presentation.
    if !text.starts_with("#EXTM3U") {
        return Err(NotPlaylistResponse.into());
    }
    match parser::parse(final_url, text)? {
        parser::Parsed::Master(variants) => Ok(Inspection {
            user_agent: None,
            browser_relay: None,
            public_only: false,
            source,
            variants,
            playlist: None,
            dash: None,
            tiktok: None,
            page: None,
        }),
        parser::Parsed::Media(mut playlist) => {
            playlist.source = source.clone();
            Ok(Inspection {
                user_agent: None,
                browser_relay: None,
                public_only: false,
                source,
                variants: Vec::new(),
                playlist: Some(*playlist),
                dash: None,
                tiktok: None,
                page: None,
            })
        }
    }
}

pub async fn preview(
    inspection: &Inspection,
    variant: usize,
    cancel: &CancellationToken,
) -> Result<Playlist> {
    let audio = inspection
        .variants
        .get(variant)
        .map_or(0, Variant::default_audio);
    preview_with_audio(inspection, variant, audio, cancel).await
}

pub async fn preview_with_audio(
    inspection: &Inspection,
    variant: usize,
    audio: usize,
    cancel: &CancellationToken,
) -> Result<Playlist> {
    preview_with_audio_via(
        inspection,
        variant,
        audio,
        cancel,
        crate::proxy::Route::direct(),
    )
    .await
}
pub async fn preview_with_audio_via(
    inspection: &Inspection,
    variant: usize,
    audio: usize,
    cancel: &CancellationToken,
    route: crate::proxy::Route,
) -> Result<Playlist> {
    let mut network = network::Network::via(&inspection.source, route)
        .await?
        .with_page(inspection.page.as_deref())
        .with_user_agent(inspection.user_agent.as_deref())?
        .with_public_only(inspection.public_only)?
        .with_browser_relay(inspection.browser_relay.as_ref())?;
    if let Some(dash) = &inspection.dash {
        ensure!(!cancel.is_cancelled(), "Video preview cancelled");
        return dash
            .preview_using(&inspection.source, variant, audio, &mut network, cancel)
            .await;
    }
    let choice = inspection
        .variants
        .get(variant)
        .ok_or_else(|| anyhow::anyhow!("Choose a listed quality"))?;
    ensure!(
        choice.unsupported.is_none(),
        "{}",
        choice
            .unsupported
            .as_deref()
            .unwrap_or("Unsupported quality")
    );
    let (url, text) = network.playlist(&choice.url, cancel).await?;
    let parser::Parsed::Media(mut playlist) = parser::parse(&url, &text)? else {
        anyhow::bail!("Nested master playlists are not supported. Use a media playlist URL.");
    };
    playlist.source.clone_from(&inspection.source);
    playlist.public_only = inspection.public_only;
    playlist.quality = choice.label();
    playlist.bandwidth = Some(choice.bandwidth);
    if !choice.audio.is_empty() {
        let track = choice
            .audio
            .get(audio)
            .ok_or_else(|| anyhow::anyhow!("Choose a listed audio track"))?;
        if let Some(url) = &track.url {
            let (url, text) = network.playlist(url, cancel).await?;
            let parser::Parsed::Media(audio) = parser::parse(&url, &text)? else {
                anyhow::bail!("The audio URL must be a finished media playlist");
            };
            playlist.audio = Some(AudioTrack {
                name: track.name.clone(),
                language: track.language.clone(),
                url: audio.url,
                duration_ms: audio.duration_ms,
                sequence: audio.sequence,
                format: audio.format,
                parts: audio.parts,
            });
        }
    } else {
        ensure!(audio == 0, "This quality has no separate audio choices");
    }
    playlist.validate()?;
    Ok(*playlist)
}

#[cfg(test)]
mod tests;
