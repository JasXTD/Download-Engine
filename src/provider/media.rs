//! Declarative, finite media plans. URLs exist only in a resolver's live reply;
//! persisted content and processor messages contain neither URLs nor credentials.
mod mux;
#[cfg(test)]
mod tests;
mod transfer;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub(crate) use transfer::download;
pub const API: u32 = 1;
pub const MAX_PLAN_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SEGMENTS: usize = 16384;
pub const MAX_SEGMENT_INPUT: usize = 8 * 1024 * 1024;
pub const MAX_SEGMENT_OUTPUT: usize = 16 * 1024 * 1024;
pub const MAX_STREAM_OUTPUT: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Media,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    Mkv,
    Mp4,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
    Subtitle,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Mp4,
    Mpegts,
    Aac,
    Srt,
    Webvtt,
    Ass,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Processing {
    #[default]
    Copy,
    Lua,
    LuaSegment,
    LuaStream,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Segment {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Zero (or omitted) means the core measures this segment while streaming it.
    #[serde(default)]
    pub size: u64,
    /// A non-secret name handed to the downloader with xd.handoff_session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// A segment may be a byte range of a larger resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_start: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_sha256: Option<String>,
    #[serde(default)]
    pub processing: Processing,
}
impl Segment {
    pub(crate) fn output_limit(&self) -> u64 {
        match self.processing {
            Processing::LuaSegment => MAX_SEGMENT_OUTPUT as u64,
            Processing::LuaStream => MAX_STREAM_OUTPUT,
            _ if self.size == 0 => crate::hls::MAX_PART_BYTES,
            _ => self.size,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub id: String,
    pub kind: TrackKind,
    pub format: Format,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub default: bool,
    /// Replay the verified original initialization segment before every processor run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init_segment: Option<String>,
    pub segments: Vec<Segment>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub kind: Kind,
    pub media_api: u32,
    pub content_id: String,
    pub container: Container,
    pub tracks: Vec<Track>,
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
impl Plan {
    pub fn validate(&self, resolved: bool) -> Result<()> {
        ensure!(self.media_api == API, "Unsupported media provider API");
        ensure!(identifier(&self.content_id), "Invalid media content_id");
        ensure!(
            !self.tracks.is_empty() && self.tracks.len() <= 16,
            "Media plans need 1 to 16 tracks"
        );
        ensure!(
            self.tracks.iter().map(|t| t.segments.len()).sum::<usize>() <= MAX_SEGMENTS,
            "Media plan exceeds 16384 segments"
        );
        let mut ids = HashSet::new();
        let mut defaults = HashSet::new();
        for track in &self.tracks {
            ensure!(
                identifier(&track.id) && ids.insert(&track.id),
                "Duplicate or invalid media track ID"
            );
            ensure!(!track.segments.is_empty(), "Media track has no segments");
            if let Some(id) = &track.init_segment {
                ensure!(
                    track
                        .segments
                        .first()
                        .is_some_and(|s| &s.id == id && s.size <= MAX_SEGMENT_INPUT as u64),
                    "The initialization segment must be first and at most 8 MiB"
                );
            }
            ensure!(
                track.language.len() <= 32
                    && track
                        .language
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
                "Invalid track language"
            );
            ensure!(
                track.title.len() <= 128 && !track.title.chars().any(char::is_control),
                "Invalid track title"
            );
            ensure!(
                !track.default || defaults.insert(track.kind as u8),
                "Only one default track per kind is allowed"
            );
            ensure!(
                matches!(
                    (track.kind, track.format),
                    (TrackKind::Video, Format::Mp4 | Format::Mpegts)
                        | (TrackKind::Audio, Format::Mp4 | Format::Mpegts | Format::Aac)
                        | (
                            TrackKind::Subtitle,
                            Format::Srt | Format::Webvtt | Format::Ass
                        )
                ),
                "Track kind and format do not match"
            );
            let mut segments = HashSet::new();
            for segment in &track.segments {
                if let Some(name) = &segment.session {
                    super::handoff::validate_name(name)?;
                }
                ensure!(
                    identifier(&segment.id) && segments.insert(&segment.id),
                    "Duplicate or invalid segment ID"
                );
                ensure!(
                    segment.size <= crate::hls::MAX_PART_BYTES,
                    "Media segments must contain at most 256 MiB; omit size if unknown"
                );
                ensure!(
                    segment.range_start.is_none() || segment.size > 0,
                    "Byte ranges require a known, positive size"
                );
                ensure!(
                    segment.processing != Processing::LuaSegment
                        || segment.size <= MAX_SEGMENT_INPUT as u64,
                    "Whole-segment Lua input exceeds 8 MiB"
                );
                ensure!(
                    segment
                        .range_start
                        .unwrap_or(0)
                        .checked_add(segment.size)
                        .is_some_and(|end| end <= i64::MAX as u64),
                    "Invalid media byte range"
                );
                for hash in [&segment.input_sha256, &segment.output_sha256]
                    .into_iter()
                    .flatten()
                {
                    ensure!(
                        hash.len() == 64
                            && hash
                                .bytes()
                                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
                        "Invalid media SHA-256"
                    );
                }
                if resolved {
                    crate::model::validate_url(&segment.url)?;
                } else {
                    ensure!(
                        segment.url.is_empty(),
                        "Saved media content must not contain URLs"
                    );
                }
            }
        }
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_PLAN_BYTES,
            "Media plan exceeds 4 MiB"
        );
        Ok(())
    }
    pub fn content(&self) -> Self {
        let mut result = self.clone();
        for track in &mut result.tracks {
            for segment in &mut track.segments {
                segment.url.clear();
            }
        }
        result
    }
    pub fn size(&self) -> u64 {
        self.tracks
            .iter()
            .flat_map(|t| &t.segments)
            .map(|s| s.size)
            .sum()
    }
    pub fn uses_segment_callbacks(&self) -> bool {
        self.tracks.iter().any(|t| {
            t.init_segment.is_some()
                || t.segments
                    .iter()
                    .any(|s| matches!(s.processing, Processing::LuaSegment | Processing::LuaStream))
        })
    }
    pub fn has_unknown_sizes(&self) -> bool {
        self.tracks
            .iter()
            .flat_map(|t| &t.segments)
            .any(|s| s.size == 0)
    }
    pub fn needs_output_sizes(&self) -> bool {
        self.uses_segment_callbacks() || self.has_unknown_sizes()
    }
    pub fn needs_current_writer(&self) -> bool {
        self.has_unknown_sizes()
            || self
                .tracks
                .iter()
                .flat_map(|t| &t.segments)
                .any(|s| s.session.is_some() || s.processing == Processing::LuaStream)
            || self.tracks.iter().any(|t| t.format == Format::Ass)
            || self.tracks.iter().map(|t| t.segments.len()).sum::<usize>() > 2048
            || serde_json::to_vec(self).map_or(true, |bytes| bytes.len() > 512 * 1024)
    }
    pub fn extension(&self) -> &'static str {
        match self.container {
            Container::Mkv => "mkv",
            Container::Mp4 => "mp4",
        }
    }
}

/// The provider script gets identifiers and offsets, never filesystem paths.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Chunk {
    pub content_id: String,
    pub track_id: String,
    pub kind: TrackKind,
    pub segment_id: String,
    pub segment_index: usize,
    pub track_offset: u64,
    /// Bytes already written to this track, including reused saved segments.
    #[serde(default)]
    pub output_offset: u64,
    pub size: u64,
}
impl Chunk {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            identifier(&self.content_id)
                && identifier(&self.track_id)
                && identifier(&self.segment_id),
            "Invalid media chunk identity"
        );
        ensure!(
            self.segment_index < MAX_SEGMENTS
                && self.size > 0
                && self.size <= crate::hls::MAX_PART_BYTES
                && self.track_offset.checked_add(self.size).is_some(),
            "Invalid media chunk bounds"
        );
        ensure!(
            self.output_offset
                .checked_add(MAX_SEGMENT_OUTPUT as u64)
                .is_some(),
            "Invalid media output offset"
        );
        Ok(())
    }
}
