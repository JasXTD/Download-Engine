//! Unencrypted, finite DASH presentations, using the shared guarded media pipeline.
mod addressing;
mod index;
mod xml;

use crate::hls::network::Network;
use crate::hls::{AudioRendition, AudioTrack, Format, Inspection, Part, Playlist, Variant};
use anyhow::{Context, Result, ensure};
pub use index::IndexIdentity;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub video: String,
    pub audio: Option<String>,
    pub video_offset_us: u64,
    pub audio_offset_us: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_index: Option<IndexIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_index: Option<IndexIdentity>,
}
impl Selection {
    pub(crate) fn has_index(&self) -> bool {
        self.video_index.is_some() || self.audio_index.is_some()
    }
    pub(crate) fn validate(&self, playlist: &Playlist) -> Result<()> {
        ensure!(
            !self.video.is_empty()
                && self.video.len() <= 512
                && self
                    .audio
                    .as_ref()
                    .is_none_or(|id| !id.is_empty() && id.len() <= 512)
                && self.audio.is_some() == playlist.audio.is_some()
                && self.video_offset_us <= i64::MAX as u64 / 2
                && self.audio_offset_us <= i64::MAX as u64 / 2
                && playlist.format == Format::FragmentedMp4
                && playlist
                    .audio
                    .as_ref()
                    .is_none_or(|a| a.format == Format::FragmentedMp4),
            "Invalid saved DASH selection"
        );
        if let Some(index) = &self.video_index {
            index.validate(&playlist.parts)?;
        }
        if let Some(index) = &self.audio_index {
            index.validate(
                &playlist
                    .audio
                    .as_ref()
                    .context("DASH index has no audio track")?
                    .parts,
            )?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
struct Representation {
    key: String,
    name: String,
    language: Option<String>,
    bandwidth: u64,
    resolution: Option<(u32, u32)>,
    codecs: String,
    offset_us: u64,
    parts: Vec<Part>,
    index: Option<index::Address>,
    is_default: bool,
}
#[derive(Clone, Debug)]
pub struct Manifest {
    url: String,
    duration_ms: u64,
    videos: Vec<Representation>,
    audio: Vec<Representation>,
}
pub fn is_manifest_url(value: &str) -> bool {
    crate::model::validate_url(value)
        .is_ok_and(|url| url.path().to_ascii_lowercase().ends_with(".mpd"))
}
pub(crate) fn parse(source: &str, url: &str, text: &str) -> Result<Inspection> {
    let root = xml::parse(text)?;
    ensure!(
        root.attr("type").unwrap_or("static") == "static",
        "Live DASH recording is not supported. Choose a finished video."
    );
    let period = root.one("Period")?.context("DASH manifest has no Period")?;
    ensure!(
        period.attr("start").map(duration).transpose()?.unwrap_or(0) == 0,
        "DASH periods starting after zero are not supported"
    );
    let duration_ms = period
        .attr("duration")
        .or(root.attr("mediaPresentationDuration"))
        .map(duration)
        .transpose()?
        .context("DASH needs a finite video duration")?;
    ensure!(
        duration_ms > 0 && duration_ms <= 7 * 86400 * 1000,
        "DASH duration must be between zero and seven days"
    );
    let base = addressing::base(&addressing::base(url, &root)?, period)?;
    let mut manifest = Manifest {
        url: url.into(),
        duration_ms,
        videos: Vec::new(),
        audio: Vec::new(),
    };
    let mut size = 0usize;
    let mut parts = 0usize;
    for (a, adaptation) in period.children("AdaptationSet").enumerate() {
        ensure!(a < 64, "DASH has too many adaptation sets");
        let parent_base = addressing::base(&base, adaptation)?;
        for (r, rep) in adaptation.children("Representation").enumerate() {
            ensure!(
                manifest.videos.len() + manifest.audio.len() < 64 && r < 64,
                "DASH has too many representations"
            );
            let attr = |key| rep.attr(key).or(adaptation.attr(key));
            let mime = attr("mimeType").unwrap_or("");
            let kind = attr("contentType")
                .or_else(|| mime.split_once('/').map(|(kind, _)| kind))
                .unwrap_or("");
            if !matches!(kind, "video" | "audio") {
                continue;
            }
            ensure!(
                rep.attr("dependencyId").is_none()
                    && rep.children("EssentialProperty").next().is_none()
                    && adaptation.children("EssentialProperty").next().is_none(),
                "Dependent or specialized DASH representations are not supported"
            );
            ensure!(
                mime.is_empty() || matches!(mime, "video/mp4" | "audio/mp4" | "application/mp4"),
                "DASH currently supports MP4 representations only"
            );
            ensure!(
                rep.one("SubRepresentation")?.is_none()
                    && adaptation.one("ContentComponent")?.is_none(),
                "Multiplexed DASH representations are not supported"
            );
            let id = rep.attr("id").context("DASH representation has no ID")?;
            ensure!(
                !id.is_empty() && id.len() <= 200,
                "Invalid DASH representation ID"
            );
            let key = format!("{a}/{id}");
            ensure!(
                !manifest
                    .videos
                    .iter()
                    .chain(&manifest.audio)
                    .any(|v| v.key == key),
                "Duplicate DASH representation ID"
            );
            let bandwidth = attr("bandwidth")
                .context("DASH representation has no bandwidth")?
                .parse::<u64>()?;
            ensure!(
                bandwidth > 0 && bandwidth <= 10_000_000_000,
                "Invalid DASH bandwidth"
            );
            let codecs = attr("codecs").unwrap_or("").to_owned();
            ensure!(codecs.len() <= 200, "DASH codec description is too long");
            let language = attr("lang").map(str::to_owned);
            ensure!(
                language.as_ref().is_none_or(|s| s.len() <= 64),
                "DASH language is too long"
            );
            let name = rep
                .one("Label")?
                .or(adaptation.one("Label")?)
                .map(|n| n.text.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or(id)
                .chars()
                .take(140)
                .collect::<String>();
            let resolution = match (attr("width"), attr("height")) {
                (Some(w), Some(h)) => Some((w.parse()?, h.parse()?)),
                _ => None,
            };
            let track = addressing::parts(
                &addressing::base(&parent_base, rep)?,
                &[period, adaptation, rep],
                id,
                bandwidth,
                duration_ms,
            )?;
            size += track.parts.iter().map(|p| p.url.len() + 64).sum::<usize>();
            parts += track.parts.len();
            ensure!(
                size <= 8 * 1024 * 1024 && parts <= 50_000,
                "DASH manifest expands to too many segments"
            );
            let value = Representation {
                key,
                name,
                language,
                bandwidth,
                resolution,
                codecs,
                offset_us: track.offset_us,
                parts: track.parts,
                index: track.index,
                is_default: adaptation
                    .children("Role")
                    .any(|role| role.attr("value") == Some("main")),
            };
            if kind == "video" {
                manifest.videos.push(value);
            } else {
                manifest.audio.push(value);
            }
        }
    }
    ensure!(
        !manifest.videos.is_empty(),
        "DASH manifest has no supported MP4 video representations"
    );
    manifest
        .videos
        .sort_by_key(|v| std::cmp::Reverse(v.bandwidth));
    let variants = manifest
        .videos
        .iter()
        .map(|video| Variant {
            url: manifest.url.clone(),
            bandwidth: video.bandwidth,
            resolution: video.resolution,
            codecs: video.codecs.clone(),
            unsupported: None,
            audio: manifest
                .audio
                .iter()
                .map(|audio| AudioRendition {
                    name: format!("{} · {} kb/s", audio.name, audio.bandwidth / 1000),
                    language: audio.language.clone(),
                    url: Some(manifest.url.clone()),
                    is_default: audio.is_default,
                })
                .collect(),
        })
        .collect();
    Ok(Inspection {
        public_only: false,
        source: source.into(),
        variants,
        playlist: None,
        dash: Some(manifest),
        tiktok: None,
        page: None,
    })
}
impl Manifest {
    #[cfg(test)]
    pub(crate) async fn preview(
        &self,
        source: &str,
        page: Option<&str>,
        video: usize,
        audio: usize,
        cancel: &CancellationToken,
        route: crate::proxy::Route,
    ) -> Result<Playlist> {
        let mut network = Network::via(source, route).await?.with_page(page);
        self.preview_using(source, video, audio, &mut network, cancel)
            .await
    }
    pub(crate) async fn preview_using(
        &self,
        source: &str,
        video: usize,
        audio: usize,
        network: &mut Network,
        cancel: &CancellationToken,
    ) -> Result<Playlist> {
        ensure!(!cancel.is_cancelled(), "DASH preview cancelled");
        let video = self
            .videos
            .get(video)
            .context("Choose a listed video quality")?;
        let selected_audio = if self.audio.is_empty() {
            ensure!(audio == 0, "This video has no audio tracks");
            None
        } else {
            Some(
                self.audio
                    .get(audio)
                    .context("Choose a listed audio track")?,
            )
        };
        let (parts, video_index) = self.resolve(video, network, cancel).await?;
        let (audio_parts, audio_index) = if let Some(audio) = selected_audio {
            self.resolve(audio, network, cancel).await?
        } else {
            (Vec::new(), None)
        };
        let quality = Variant {
            url: self.url.clone(),
            bandwidth: video.bandwidth,
            resolution: video.resolution,
            codecs: video.codecs.clone(),
            unsupported: None,
            audio: Vec::new(),
        }
        .label();
        let playlist = Playlist {
            // Local policy is retained when a saved manifest is rechecked.
            public_only: network.public_only(),
            source: source.into(),
            url: self.url.clone(),
            quality,
            bandwidth: Some(video.bandwidth + selected_audio.map_or(0, |a| a.bandwidth)),
            duration_ms: self.duration_ms,
            sequence: 0,
            format: Format::FragmentedMp4,
            parts,
            audio: selected_audio.map(|a| AudioTrack {
                name: a.name.clone(),
                language: a.language.clone(),
                url: self.url.clone(),
                duration_ms: self.duration_ms,
                sequence: 0,
                format: Format::FragmentedMp4,
                parts: audio_parts,
            }),
            dash: Some(Selection {
                video: video.key.clone(),
                audio: selected_audio.map(|a| a.key.clone()),
                video_offset_us: video.offset_us,
                audio_offset_us: selected_audio.map_or(0, |a| a.offset_us),
                video_index,
                audio_index,
            }),
        };
        playlist.validate()?;
        Ok(playlist)
    }
    async fn resolve(
        &self,
        track: &Representation,
        network: &mut Network,
        cancel: &CancellationToken,
    ) -> Result<(Vec<Part>, Option<IndexIdentity>)> {
        if let Some(index) = &track.index {
            let (parts, identity) = index.resolve(network, self.duration_ms, cancel).await?;
            Ok((parts, Some(identity)))
        } else {
            Ok((track.parts.clone(), None))
        }
    }
    /// Choice positions of saved representation keys, as listed in the inspection.
    pub(crate) fn position(&self, video: &str, audio: Option<&str>) -> Option<(usize, usize)> {
        let v = self.videos.iter().position(|r| r.key == video)?;
        let a = match audio {
            Some(key) => self.audio.iter().position(|r| r.key == key)?,
            None if self.audio.is_empty() => 0,
            None => return None,
        };
        Some((v, a))
    }
    pub(crate) async fn recheck(
        &self,
        playlist: &Playlist,
        network: &mut Network,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let selected = playlist.dash.as_ref().context("Missing DASH selection")?;
        let v = self
            .videos
            .iter()
            .position(|v| v.key == selected.video)
            .context("Selected DASH quality disappeared. Preview again.")?;
        let a = if let Some(id) = &selected.audio {
            self.audio
                .iter()
                .position(|a| &a.key == id)
                .context("Selected DASH audio disappeared. Preview again.")?
        } else {
            0
        };
        ensure!(
            &self
                .preview_using(&playlist.source, v, a, network, cancel)
                .await?
                == playlist,
            "DASH manifest or index changed. Preview the video again before downloading."
        );
        Ok(())
    }
}
fn duration(value: &str) -> Result<u64> {
    let mut rest = value.strip_prefix('P').context("Invalid DASH duration")?;
    let mut milliseconds = 0u64;
    let mut time = false;
    let mut last = 0;
    let mut any = false;
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix('T') {
            ensure!(!time, "Invalid DASH duration");
            time = true;
            rest = tail;
            ensure!(!rest.is_empty(), "Invalid DASH duration");
            continue;
        }
        let end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .context("Invalid DASH duration")?;
        let value: f64 = rest[..end].parse()?;
        let unit = rest.as_bytes()[end];
        let (order, scale) = match (time, unit) {
            (false, b'D') => (1, 86400000.),
            (true, b'H') => (2, 3600000.),
            (true, b'M') => (3, 60000.),
            (true, b'S') => (4, 1000.),
            _ => anyhow::bail!("DASH duration uses unsupported units"),
        };
        ensure!(
            order > last && value.is_finite() && value >= 0. && value * scale <= 7. * 86400000.,
            "Invalid DASH duration"
        );
        milliseconds = milliseconds
            .checked_add((value * scale).round() as u64)
            .context("DASH duration overflow")?;
        last = order;
        any = true;
        rest = &rest[end + 1..];
    }
    ensure!(any, "Invalid DASH duration");
    Ok(milliseconds)
}

#[cfg(test)]
mod tests;
