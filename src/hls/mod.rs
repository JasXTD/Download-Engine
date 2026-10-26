//! Bounded, anonymous HLS and DASH previews and downloads. Never fetch encryption keys.
mod aac;
mod merge;
mod mp4;
pub(crate) mod network;
mod parser;
mod store;
mod transfer;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
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
    pub version: u8,
    pub playlist: Playlist,
    #[serde(default)]
    pub completed: Vec<CompletedPart>,
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
            youtube: None,
            bilibili: None,
            tiktok: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
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
            self.completed.len() <= self.playlist.part_count(),
            "Invalid saved HLS progress"
        );
        for part in &self.completed {
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
    let source = crate::model::validate_url(url)?.to_string();
    let page = page
        .map(|page| crate::model::validate_url(page).map(|page| page.to_string()))
        .transpose()?;
    let mut network = network::Network::via(&source, route)
        .await?
        .with_page(page.as_deref())
        .with_public_only(public_only)?;
    let (final_url, text) = network.playlist(&source, cancel).await?;
    let mut inspection = inspection_from(source, &final_url, &text)?;
    inspection.page = page;
    inspection.public_only = public_only;
    if let Some(playlist) = &mut inspection.playlist {
        playlist.public_only = public_only;
    }
    Ok(inspection)
}

/// Parse a playlist or DASH manifest that was already fetched from `final_url`.
pub(crate) fn inspection_from(source: String, final_url: &str, text: &str) -> Result<Inspection> {
    if crate::dash::is_manifest_url(final_url)
        || text
            .trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with('<')
    {
        return crate::dash::parse(&source, final_url, text);
    }
    match parser::parse(final_url, text)? {
        parser::Parsed::Master(variants) => Ok(Inspection {
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
        .with_public_only(inspection.public_only)?;
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
