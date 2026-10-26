use super::{MAX_PART_BYTES, MAX_PARTS, MAX_PLAYLIST_BYTES};
use anyhow::{Context, Result, bail, ensure};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    TransportStream,
    FragmentedMp4,
    PackedAac,
    /// One ordinary MP4 file with sound, fetched in byte-range parts and joined
    /// unchanged. Only reviewed site videos (TikTok) use it.
    Progressive,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::TransportStream => "ts",
            Self::FragmentedMp4 | Self::Progressive => "mp4",
            Self::PackedAac => "aac",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::TransportStream => "MPEG-TS (.ts)",
            Self::FragmentedMp4 => "Fragmented MP4 (.mp4)",
            Self::PackedAac => "AAC audio (.aac)",
            Self::Progressive => "MP4 (.mp4) · video + audio",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: u64,
    pub length: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    pub url: String,
    pub range: Option<Range>,
    pub initialization: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    /// A reader's public-only boundary also applies after restart or resume.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub public_only: bool,
    pub source: String,
    pub url: String,
    pub quality: String,
    pub bandwidth: Option<u64>,
    pub duration_ms: u64,
    pub sequence: u64,
    pub format: Format,
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash: Option<crate::dash::Selection>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioTrack {
    pub name: String,
    pub language: Option<String>,
    pub url: String,
    pub duration_ms: u64,
    pub sequence: u64,
    pub format: Format,
    pub parts: Vec<Part>,
}
impl Playlist {
    pub fn validate(&self) -> Result<()> {
        if let Some(selection) = &self.dash {
            selection.validate(self)?;
        }
        ensure!(
            self.format != Format::Progressive || (self.audio.is_none() && self.dash.is_none()),
            "A single MP4 file has no separate audio or DASH tracks"
        );
        crate::model::validate_url(&self.source)?;
        self.validate_track()?;
        if let Some(audio) = &self.audio {
            ensure!(
                !audio.name.is_empty()
                    && audio.name.len() <= 200
                    && audio.language.as_ref().is_none_or(|v| v.len() <= 64),
                "Invalid HLS audio description"
            );
            audio.playlist(&self.source).validate_track()?;
        }
        ensure!(
            self.part_count() <= MAX_PARTS
                && serde_json::to_vec(self)?.len() <= 4 * MAX_PLAYLIST_BYTES,
            "HLS playlist is too large"
        );
        Ok(())
    }
    fn validate_track(&self) -> Result<()> {
        crate::model::validate_url(&self.url)?;
        ensure!(
            !self.parts.is_empty()
                && self.parts.len() <= MAX_PARTS
                && self.duration_ms > 0
                && self.duration_ms <= 7 * 24 * 3600 * 1000,
            "Invalid saved HLS playlist"
        );
        ensure!(
            self.quality.len() <= 512 && serde_json::to_vec(self)?.len() <= 4 * MAX_PLAYLIST_BYTES,
            "HLS playlist is too large"
        );
        let has_init = self.parts[0].initialization;
        ensure!(
            has_init == (self.format == Format::FragmentedMp4)
                && self.parts.len() > usize::from(has_init),
            "Invalid HLS initialization segment"
        );
        for (i, part) in self.parts.iter().enumerate() {
            let url = crate::model::validate_url(&part.url)?;
            ensure!(
                self.url.starts_with("http:") || url.scheme() == "https",
                "HLS cannot downgrade HTTPS to HTTP"
            );
            ensure!(
                i == 0 || !part.initialization,
                "Changing HLS initialization segments is not supported"
            );
            if let Some(range) = &part.range {
                ensure!(
                    range.length > 0
                        && range.length <= MAX_PART_BYTES
                        && range
                            .start
                            .checked_add(range.length)
                            .is_some_and(|end| end <= i64::MAX as u64),
                    "Invalid HLS byte range"
                );
            }
        }
        if self.format == Format::Progressive {
            // Consecutive ranges of one file from its first byte, joined unchanged.
            let mut next = 0u64;
            for part in &self.parts {
                let range = part
                    .range
                    .as_ref()
                    .context("Every part of a single MP4 file needs a byte range")?;
                ensure!(
                    range.start == next && part.url == self.parts[0].url,
                    "Single MP4 file parts must be consecutive ranges of one file"
                );
                next = range.start + range.length;
            }
        }
        Ok(())
    }
    pub fn media_count(&self) -> usize {
        self.parts.iter().filter(|p| !p.initialization).count()
            + self
                .audio
                .as_ref()
                .map_or(0, |a| a.parts.iter().filter(|p| !p.initialization).count())
    }
    pub fn part_count(&self) -> usize {
        self.parts.len() + self.audio.as_ref().map_or(0, |a| a.parts.len())
    }
    pub fn output_extension(&self) -> &'static str {
        if self.audio.is_some() {
            "mp4"
        } else {
            self.format.extension()
        }
    }
    pub fn output_label(&self) -> &'static str {
        if self.audio.is_some() {
            "MP4 (.mp4) · video + audio"
        } else if self.dash.is_some() {
            "MP4 (.mp4) · video only"
        } else {
            self.format.label()
        }
    }
    pub fn estimated_size(&self) -> Option<u64> {
        self.bandwidth?
            .checked_mul(self.duration_ms)
            .map(|bits| bits / 8000)
    }
    pub fn needs_merge(&self) -> bool {
        self.audio.is_some() || self.dash.is_some()
    }
}
impl AudioTrack {
    pub(super) fn playlist(&self, source: &str) -> Playlist {
        Playlist {
            public_only: false,
            source: source.into(),
            url: self.url.clone(),
            quality: self.name.clone(),
            bandwidth: None,
            duration_ms: self.duration_ms,
            sequence: self.sequence,
            format: self.format,
            parts: self.parts.clone(),
            audio: None,
            dash: None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct AudioRendition {
    pub name: String,
    pub language: Option<String>,
    pub url: Option<String>,
    pub is_default: bool,
}
impl AudioRendition {
    pub fn label(&self) -> String {
        format!(
            "{}{}{}",
            self.name,
            self.language
                .as_ref()
                .map(|v| format!(" · {v}"))
                .unwrap_or_default(),
            if self.url.is_none() {
                " · included in video"
            } else {
                ""
            }
        )
    }
}
#[derive(Clone, Debug)]
pub struct Variant {
    pub url: String,
    pub bandwidth: u64,
    pub resolution: Option<(u32, u32)>,
    pub codecs: String,
    pub unsupported: Option<String>,
    pub audio: Vec<AudioRendition>,
}
impl Variant {
    pub fn default_audio(&self) -> usize {
        self.audio.iter().position(|a| a.is_default).unwrap_or(0)
    }
    pub fn label(&self) -> String {
        let resolution = self
            .resolution
            .map(|(w, h)| format!("{w} × {h} · "))
            .unwrap_or_default();
        format!(
            "{resolution}{:.1} Mb/s{}",
            self.bandwidth as f64 / 1_000_000.0,
            if self.codecs.is_empty() {
                String::new()
            } else {
                format!(" · {}", self.codecs)
            }
        )
    }
}
pub(super) enum Parsed {
    Master(Vec<Variant>),
    Media(Box<Playlist>),
}

fn attributes(mut value: &str) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    while !value.is_empty() {
        let (name, rest) = value.split_once('=').context("Invalid HLS attributes")?;
        ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-'),
            "Invalid HLS attribute name"
        );
        let (item, tail) = if let Some(rest) = rest.strip_prefix('"') {
            let (item, tail) = rest.split_once('"').context("Unterminated HLS attribute")?;
            (
                item,
                if tail.is_empty() {
                    ""
                } else {
                    tail.strip_prefix(',')
                        .context("Invalid HLS attribute separator")?
                },
            )
        } else {
            rest.split_once(',').unwrap_or((rest, ""))
        };
        ensure!(
            !item.is_empty() && result.insert(name.to_owned(), item.to_owned()).is_none(),
            "Empty or duplicate HLS attribute"
        );
        value = tail;
    }
    Ok(result)
}
fn required<'a>(attrs: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str> {
    attrs
        .get(key)
        .map(String::as_str)
        .with_context(|| format!("HLS is missing {key}"))
}
fn resolve(base: &Url, uri: &str) -> Result<String> {
    ensure!(
        !uri.contains("{$"),
        "HLS variable substitution is not supported"
    );
    let url = base.join(uri).context("Invalid HLS resource URL")?;
    let url = crate::model::validate_url(url.as_str())?;
    ensure!(
        base.scheme() != "https" || url.scheme() == "https",
        "HLS cannot downgrade HTTPS to HTTP"
    );
    Ok(url.to_string())
}
fn byte_range(value: &str, url: &str, previous: Option<&Part>) -> Result<Range> {
    let (len, offset) = value
        .split_once('@')
        .map_or((value, None), |(l, o)| (l, Some(o)));
    let length: u64 = len.parse().context("Invalid HLS byte range length")?;
    let start = if let Some(offset) = offset {
        offset.parse().context("Invalid HLS byte range offset")?
    } else {
        let previous = previous
            .filter(|part| part.url == url)
            .and_then(|part| part.range.as_ref())
            .context("Implicit HLS byte range has no preceding range of the same resource")?;
        previous
            .start
            .checked_add(previous.length)
            .context("HLS range overflow")?
    };
    ensure!(
        length > 0
            && length <= MAX_PART_BYTES
            && start
                .checked_add(length)
                .is_some_and(|v| v <= i64::MAX as u64),
        "Invalid HLS byte range"
    );
    Ok(Range { start, length })
}

pub(super) fn parse(url: &str, text: &str) -> Result<Parsed> {
    ensure!(
        text.len() <= MAX_PLAYLIST_BYTES
            && !text.starts_with('\u{feff}')
            && !text
                .chars()
                .any(|c| c.is_control() && c != '\r' && c != '\n'),
        "Invalid or oversized HLS playlist"
    );
    let base = crate::model::validate_url(url)?;
    let mut lines = text.lines();
    ensure!(
        lines.next() == Some("#EXTM3U"),
        "This URL did not return an HLS playlist"
    );
    let lines: Vec<_> = lines.filter(|line| !line.is_empty()).collect();
    // Check the entire manifest before following any URI, including keys on
    // later segments and session keys inherited by variants.
    for line in &lines {
        if line.starts_with("#EXT-X-SESSION-KEY:") {
            bail!("Encrypted HLS streams are not supported");
        }
        if let Some(value) = line.strip_prefix("#EXT-X-KEY:") {
            let attrs = attributes(value)?;
            ensure!(
                required(&attrs, "METHOD")? == "NONE" && attrs.len() == 1,
                "Encrypted HLS streams are not supported"
            );
        }
        ensure!(
            ![
                "#EXT-X-DEFINE:",
                "#EXT-X-CONTENT-STEERING:",
                "#EXT-X-SKIP:",
                "#EXT-X-PART:",
                "#EXT-X-PRELOAD-HINT:",
                "#EXT-X-GAP",
                "#EXT-X-I-FRAMES-ONLY",
                "#EXT-X-DISCONTINUITY"
            ]
            .iter()
            .any(|tag| line.starts_with(tag)),
            "This HLS playlist needs live, discontinuity, gap, or advanced rendition handling that is not supported yet"
        );
    }
    if lines
        .iter()
        .any(|line| line.starts_with("#EXT-X-STREAM-INF:"))
    {
        ensure!(
            !lines
                .iter()
                .any(|line| line.starts_with("#EXTINF:") || line.starts_with("#EXT-X-MAP:")),
            "Mixed master and media HLS playlist"
        );
        let mut external_groups = HashSet::new();
        let mut groups = HashSet::new();
        let mut audio_groups: BTreeMap<String, Vec<AudioRendition>> = BTreeMap::new();
        for line in &lines {
            if let Some(value) = line.strip_prefix("#EXT-X-MEDIA:") {
                let attrs = attributes(value)?;
                groups.insert((
                    required(&attrs, "TYPE")?.to_owned(),
                    required(&attrs, "GROUP-ID")?.to_owned(),
                ));
                if matches!(required(&attrs, "TYPE")?, "AUDIO" | "VIDEO")
                    && attrs.contains_key("URI")
                {
                    external_groups.insert((
                        required(&attrs, "TYPE")?.to_owned(),
                        required(&attrs, "GROUP-ID")?.to_owned(),
                    ));
                }
                if required(&attrs, "TYPE")? == "AUDIO" {
                    let name = required(&attrs, "NAME")?.to_owned();
                    let language = attrs.get("LANGUAGE").cloned();
                    ensure!(
                        name.len() <= 200 && language.as_ref().is_none_or(|v| v.len() <= 64),
                        "HLS audio description is too long"
                    );
                    let is_default = match attrs.get("DEFAULT").map(String::as_str) {
                        None | Some("NO") => false,
                        Some("YES") => true,
                        _ => bail!("Invalid default HLS audio rendition"),
                    };
                    let group = audio_groups
                        .entry(required(&attrs, "GROUP-ID")?.into())
                        .or_default();
                    ensure!(
                        group.len() < 32
                            && !group
                                .iter()
                                .any(|a| a.name == name || (is_default && a.is_default)),
                        "Duplicate or excessive HLS audio renditions"
                    );
                    group.push(AudioRendition {
                        name,
                        language,
                        url: attrs.get("URI").map(|v| resolve(&base, v)).transpose()?,
                        is_default,
                    });
                }
            }
        }
        let mut variants = Vec::new();
        let mut pending = None;
        for line in &lines {
            if let Some(value) = line.strip_prefix("#EXT-X-STREAM-INF:") {
                ensure!(pending.is_none(), "HLS quality is missing its playlist URL");
                pending = Some(attributes(value)?);
            } else if !line.starts_with('#') {
                let attrs = pending
                    .take()
                    .context("Unexpected URL in HLS master playlist")?;
                let bandwidth: u64 = required(&attrs, "BANDWIDTH")?
                    .parse()
                    .context("Invalid HLS bandwidth")?;
                ensure!(
                    bandwidth > 0 && bandwidth <= 10_000_000_000,
                    "Invalid HLS bandwidth"
                );
                let resolution = attrs
                    .get("RESOLUTION")
                    .map(|value| -> Result<_> {
                        let (w, h) = value.split_once('x').context("Invalid HLS resolution")?;
                        let (w, h): (u32, u32) = (w.parse()?, h.parse()?);
                        ensure!(
                            w > 0 && h > 0 && w <= 65536 && h <= 65536,
                            "Invalid HLS resolution"
                        );
                        Ok((w, h))
                    })
                    .transpose()?;
                let external = attrs
                    .get("VIDEO")
                    .is_some_and(|id| external_groups.contains(&("VIDEO".into(), id.clone())));
                ensure!(
                    ["AUDIO", "VIDEO"].iter().all(|kind| attrs
                        .get(*kind)
                        .is_none_or(|id| groups.contains(&(kind.to_string(), id.clone())))),
                    "HLS quality references a missing audio/video group"
                );
                variants.push(Variant {
                    url: resolve(&base, line)?,
                    bandwidth,
                    resolution,
                    codecs: attrs.get("CODECS").cloned().unwrap_or_default(),
                    audio: attrs
                        .get("AUDIO")
                        .and_then(|id| audio_groups.get(id))
                        .cloned()
                        .unwrap_or_default(),
                    unsupported: external
                        .then(|| "Alternate video rendition groups are not supported yet".into()),
                });
                ensure!(
                    variants.len() <= 64 && variants.last().unwrap().codecs.len() <= 200,
                    "Too many HLS qualities or oversized codec description"
                );
            }
        }
        ensure!(
            pending.is_none() && !variants.is_empty(),
            "Incomplete HLS master playlist"
        );
        variants.sort_by_key(|v| std::cmp::Reverse(v.bandwidth));
        return Ok(Parsed::Master(variants));
    }
    let mut parts: Vec<Part> = Vec::new();
    let mut duration_ms = 0u64;
    let mut duration = None;
    let mut range = None;
    let mut map: Option<Part> = None;
    let mut ended = false;
    let mut target = None;
    let mut sequence = 0;
    let mut sequence_seen = false;
    for line in &lines {
        if let Some(value) = line.strip_prefix("#EXTINF:") {
            ensure!(duration.is_none() && !ended, "Invalid HLS segment order");
            let seconds: f64 = value
                .split_once(',')
                .map_or(value, |(v, _)| v)
                .parse()
                .context("Invalid HLS segment duration")?;
            ensure!(
                seconds.is_finite() && seconds > 0.0 && seconds <= 86400.0,
                "Invalid HLS segment duration"
            );
            duration = Some((seconds * 1000.0).round().max(1.0) as u64);
        } else if let Some(value) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            ensure!(range.replace(value).is_none(), "Duplicate HLS byte range");
        } else if let Some(value) = line.strip_prefix("#EXT-X-MAP:") {
            let attrs = attributes(value)?;
            let url = resolve(&base, required(&attrs, "URI")?)?;
            let init = Part {
                range: attrs
                    .get("BYTERANGE")
                    .map(|value| byte_range(value, &url, None))
                    .transpose()?,
                url,
                initialization: true,
            };
            ensure!(
                map.as_ref().is_none_or(|v| v == &init) && (map.is_some() || parts.is_empty()),
                "Changing HLS initialization segments is not supported"
            );
            if map.is_none() {
                parts.push(init.clone());
                map = Some(init);
            }
        } else if let Some(value) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            ensure!(target.is_none(), "Duplicate HLS target duration");
            let seconds: u64 = value.parse().context("Invalid HLS target duration")?;
            ensure!(
                seconds > 0 && seconds <= 86400,
                "Invalid HLS target duration"
            );
            target = Some(seconds);
        } else if let Some(value) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            ensure!(
                !sequence_seen && duration_ms == 0,
                "Invalid HLS media sequence"
            );
            sequence = value.parse().context("Invalid HLS media sequence")?;
            sequence_seen = true;
        } else if *line == "#EXT-X-ENDLIST" {
            ensure!(!ended && duration.is_none(), "Invalid HLS end marker");
            ended = true;
        } else if !line.starts_with('#') {
            ensure!(!ended, "HLS segment appears after the end marker");
            let millis = duration.take().context("HLS segment has no duration")?;
            duration_ms = duration_ms
                .checked_add(millis)
                .context("HLS duration overflow")?;
            let url = resolve(&base, line)?;
            let range = range
                .take()
                .map(|value| byte_range(value, &url, parts.last().filter(|p| !p.initialization)))
                .transpose()?;
            parts.push(Part {
                url,
                range,
                initialization: false,
            });
            ensure!(
                parts.len() <= MAX_PARTS,
                "HLS exceeds the 10,000 segment limit"
            );
        }
    }
    ensure!(
        ended,
        "Live HLS recording is not supported. Choose a finished video (EXT-X-ENDLIST)."
    );
    ensure!(
        target.is_some() && duration.is_none() && range.is_none(),
        "Incomplete HLS media playlist"
    );
    let playlist = Playlist {
        public_only: false,
        source: base.to_string(),
        url: base.to_string(),
        quality: "Original quality".into(),
        bandwidth: None,
        duration_ms,
        sequence,
        format: if map.is_some() {
            Format::FragmentedMp4
        } else {
            Format::TransportStream
        },
        parts,
        audio: None,
        dash: None,
    };
    playlist.validate()?;
    Ok(Parsed::Media(Box::new(playlist)))
}
