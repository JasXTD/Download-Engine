use super::xml::Node;
use crate::hls::{MAX_PART_BYTES, MAX_PARTS, Part, Range};
use anyhow::{Context, Result, bail, ensure};

pub(super) struct Track {
    pub parts: Vec<Part>,
    pub offset_us: u64,
    pub index: Option<super::index::Address>,
}

pub(super) fn resolve(base: &str, value: &str) -> Result<String> {
    ensure!(!value.trim().is_empty(), "Empty DASH resource URL");
    let base = crate::model::validate_url(base)?;
    let url = crate::model::validate_url(base.join(value.trim())?.as_str())?;
    ensure!(
        base.scheme() != "https" || url.scheme() == "https",
        "DASH cannot downgrade HTTPS to HTTP"
    );
    Ok(url.to_string())
}
pub(super) fn base(parent: &str, node: &Node) -> Result<String> {
    node.one("BaseURL")?
        .map_or_else(|| Ok(parent.into()), |n| resolve(parent, &n.text))
}
fn number(node: &Node, key: &str, default: Option<u64>) -> Result<u64> {
    let value = node
        .attr(key)
        .map(str::parse)
        .transpose()
        .with_context(|| format!("Invalid DASH {key}"))?
        .or(default)
        .with_context(|| format!("Missing DASH {key}"))?;
    ensure!(value <= i64::MAX as u64, "DASH {key} is too large");
    Ok(value)
}
fn inherited(nodes: &[&Node]) -> Result<Node> {
    let mut output = Node::default();
    for node in nodes {
        for kind in ["SegmentTemplate", "SegmentList", "SegmentBase"] {
            if let Some(value) = node.one(kind)? {
                ensure!(
                    output.name.is_empty() || output.name == kind,
                    "Conflicting DASH segment addressing"
                );
                output.name = kind.into();
                output.attrs.extend(value.attrs.clone());
                for child_kind in [
                    "Initialization",
                    "SegmentTimeline",
                    "SegmentURL",
                    "RepresentationIndex",
                ] {
                    if value.children(child_kind).next().is_some() {
                        output.children.retain(|child| child.name != child_kind);
                        output.children.extend(value.children(child_kind).cloned());
                    }
                }
            }
        }
    }
    ensure!(
        !output.name.is_empty(),
        "DASH needs a segment template, segment list or indexed SegmentBase"
    );
    Ok(output)
}
fn range(value: Option<&str>) -> Result<Option<Range>> {
    value
        .map(|value| {
            let (start, end) = value.split_once('-').context("Invalid DASH byte range")?;
            let start: u64 = start.parse()?;
            let end: u64 = end.parse()?;
            ensure!(
                start <= end && end < i64::MAX as u64 && end - start < MAX_PART_BYTES,
                "Invalid DASH byte range"
            );
            Ok(Range {
                start,
                length: end - start + 1,
            })
        })
        .transpose()
}
pub(super) fn parts(
    base: &str,
    nodes: &[&Node],
    id: &str,
    bandwidth: u64,
    duration_ms: u64,
) -> Result<Track> {
    let node = inherited(nodes)?;
    let scale = number(&node, "timescale", Some(1))?;
    ensure!(
        scale > 0 && scale <= 1_000_000_000,
        "Invalid DASH timescale"
    );
    let offset = number(&node, "presentationTimeOffset", Some(0))?;
    let offset_us = u64::try_from(u128::from(offset) * 1_000_000 / u128::from(scale))?;
    ensure!(
        offset_us <= i64::MAX as u64 / 2,
        "DASH presentation offset is too large"
    );
    if node.name == "SegmentBase" {
        ensure!(
            node.one("RepresentationIndex")?.is_none(),
            "External DASH representation indexes are not supported"
        );
        let index =
            range(node.attr("indexRange"))?.context("DASH SegmentBase needs an indexRange")?;
        ensure!(
            index.length <= super::index::MAX_INDEX_BYTES,
            "DASH index exceeds 1 MiB"
        );
        let exact = match node.attr("indexRangeExact").unwrap_or("false") {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => bail!("Invalid DASH indexRangeExact value"),
        };
        let init = node
            .one("Initialization")?
            .context("DASH SegmentBase needs an initialization range")?;
        let init_url = init
            .attr("sourceURL")
            .map_or_else(|| Ok(base.to_owned()), |url| resolve(base, url))?;
        let init_range = range(init.attr("range"))?
            .context("DASH SegmentBase needs an initialization byte range")?;
        ensure!(
            init_url == base
                && init_range.length <= 8 * 1024 * 1024
                && init_range
                    .start
                    .checked_add(init_range.length)
                    .is_some_and(|end| end <= index.start),
            "DASH initialization must precede the index in the same media file (at most 8 MiB)"
        );
        let initialization = Part {
            url: init_url,
            range: Some(init_range),
            initialization: true,
        };
        return Ok(Track {
            parts: vec![initialization.clone()],
            offset_us,
            index: Some(super::index::Address {
                url: base.into(),
                range: index,
                initialization,
                offset_us,
                exact,
            }),
        });
    }
    let end = offset
        .checked_add(u64::try_from(
            (u128::from(duration_ms) * u128::from(scale)).div_ceil(1000),
        )?)
        .context("DASH timeline overflow")?;
    let start_number = number(&node, "startNumber", Some(1))?;
    let times = if node.name == "SegmentList"
        && node.one("SegmentTimeline")?.is_none()
        && node.attr("duration").is_none()
    {
        let count = node.children("SegmentURL").count();
        ensure!(
            count > 0 && count < MAX_PARTS,
            "DASH segment list is empty or too large"
        );
        (0..count)
            .map(|i| {
                start_number
                    .checked_add(i as u64)
                    .map(|n| (n, 0))
                    .context("DASH segment number overflow")
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        timeline(&node, offset, end, start_number)?
    };
    let mut parts = Vec::new();
    let mut bytes = 0;
    let mut push = |part: Part| -> Result<()> {
        bytes += part.url.len() + 64;
        ensure!(
            bytes <= 4 * 1024 * 1024,
            "DASH track expands to too much segment data"
        );
        parts.push(part);
        Ok(())
    };
    if node.name == "SegmentTemplate" {
        let init = node
            .attr("initialization")
            .context("DASH template needs an initialization segment")?;
        push(Part {
            url: resolve(base, &substitute(init, id, bandwidth, None, None)?)?,
            range: None,
            initialization: true,
        })?;
        let media = node
            .attr("media")
            .context("DASH template has no media URL")?;
        for (number, time) in times {
            push(Part {
                url: resolve(
                    base,
                    &substitute(media, id, bandwidth, Some(number), Some(time))?,
                )?,
                range: None,
                initialization: false,
            })?;
        }
    } else {
        let init = node
            .one("Initialization")?
            .context("DASH list needs an initialization segment")?;
        push(Part {
            url: init
                .attr("sourceURL")
                .map_or_else(|| Ok(base.into()), |url| resolve(base, url))?,
            range: range(init.attr("range"))?,
            initialization: true,
        })?;
        let urls: Vec<_> = node.children("SegmentURL").collect();
        ensure!(
            !urls.is_empty() && urls.len() < MAX_PARTS,
            "DASH segment list is empty or too large"
        );
        for (number, _) in times {
            let index = usize::try_from(
                number
                    .checked_sub(start_number)
                    .context("Invalid DASH segment number")?,
            )?;
            let segment = urls
                .get(index)
                .context("DASH timeline exceeds its segment list")?;
            ensure!(
                segment.attr("index").is_none() && segment.attr("indexRange").is_none(),
                "Indexed DASH lists are not supported"
            );
            push(Part {
                url: segment
                    .attr("media")
                    .map_or_else(|| Ok(base.into()), |url| resolve(base, url))?,
                range: range(segment.attr("mediaRange"))?,
                initialization: false,
            })?;
        }
    }
    ensure!(
        parts.len() > 1 && parts.len() <= MAX_PARTS,
        "DASH has too many segments"
    );
    Ok(Track {
        parts,
        offset_us,
        index: None,
    })
}
fn timeline(node: &Node, offset: u64, end: u64, start_number: u64) -> Result<Vec<(u64, u64)>> {
    let mut result = Vec::new();
    let mut push = |number: u64, time: u64, duration: u64| -> Result<()> {
        let next = time
            .checked_add(duration)
            .context("DASH timeline overflow")?;
        if next > offset && time < end {
            ensure!(
                !result.is_empty() || time == offset,
                "DASH segments crossing the period start are not supported"
            );
            ensure!(result.len() + 1 < MAX_PARTS, "DASH has too many segments");
            result.push((number, time));
        }
        Ok(())
    };
    if let Some(timeline) = node.one("SegmentTimeline")? {
        let entries: Vec<_> = timeline.children("S").collect();
        ensure!(
            !entries.is_empty() && entries.len() < MAX_PARTS,
            "DASH timeline is empty or too large"
        );
        let mut next = 0;
        let mut number = start_number;
        let mut expanded = 0;
        for (index, entry) in entries.iter().enumerate() {
            let time = number_fn(entry, "t", Some(next))?;
            ensure!(
                index == 0 || time == next,
                "DASH timeline gaps and overlaps are not supported"
            );
            let duration = number_fn(entry, "d", None)?;
            ensure!(duration > 0, "Invalid DASH segment duration");
            let repeat = entry.attr("r").unwrap_or("0").parse::<i64>()?;
            let count = if repeat == -1 {
                let limit = entries
                    .get(index + 1)
                    .map(|n| number_fn(n, "t", None))
                    .transpose()?
                    .unwrap_or(end);
                ensure!(limit > time, "Invalid DASH repeated segment");
                let span = limit - time;
                ensure!(
                    index + 1 == entries.len() || span.is_multiple_of(duration),
                    "DASH repeated segments overlap the next entry"
                );
                span.div_ceil(duration)
            } else {
                ensure!(repeat >= 0, "Invalid DASH repetition count");
                (repeat as u64)
                    .checked_add(1)
                    .context("DASH repetition overflow")?
            };
            expanded += count;
            ensure!(
                expanded < MAX_PARTS as u64,
                "DASH has too many repeated segments"
            );
            next = time;
            for _ in 0..count {
                push(number, next, duration)?;
                next = next
                    .checked_add(duration)
                    .context("DASH timeline overflow")?;
                number = number
                    .checked_add(1)
                    .context("DASH segment number overflow")?;
            }
        }
        ensure!(next >= end, "DASH timeline ends before the video duration");
    } else {
        let duration = number(node, "duration", None)?;
        ensure!(duration > 0, "Invalid DASH segment duration");
        let first = offset / duration;
        let last = end.div_ceil(duration);
        ensure!(
            last - first < MAX_PARTS as u64,
            "DASH has too many segments"
        );
        for i in first..last {
            push(
                start_number
                    .checked_add(i)
                    .context("DASH number overflow")?,
                i.checked_mul(duration).context("DASH timeline overflow")?,
                duration,
            )?;
        }
    }
    ensure!(
        !result.is_empty(),
        "DASH has no video segments in this period"
    );
    Ok(result)
}
// Keep the function available inside the timeline's segment-number scope.
fn number_fn(node: &Node, key: &str, default: Option<u64>) -> Result<u64> {
    number(node, key, default)
}
fn substitute(
    template: &str,
    id: &str,
    bandwidth: u64,
    number: Option<u64>,
    time: Option<u64>,
) -> Result<String> {
    let mut output = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('$') {
        output.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        if let Some(tail) = rest.strip_prefix('$') {
            output.push('$');
            rest = tail;
            continue;
        }
        let (token, tail) = rest.split_once('$').context("Unclosed DASH URL template")?;
        rest = tail;
        let (name, format) = token
            .split_once('%')
            .map_or((token, None), |(a, b)| (a, Some(b)));
        if name == "RepresentationID" {
            ensure!(
                format.is_none(),
                "DASH representation IDs cannot use numeric formatting"
            );
            output.push_str(id);
        } else {
            let value = match name {
                "Bandwidth" => bandwidth,
                "Number" => {
                    number.context("Segment number is invalid in this DASH initialization URL")?
                }
                "Time" => {
                    time.context("Segment time is invalid in this DASH initialization URL")?
                }
                _ => bail!("Unknown DASH URL template token"),
            };
            let width = if let Some(format) = format {
                let digits = format
                    .strip_prefix('0')
                    .and_then(|v| v.strip_suffix('d'))
                    .context("Unsupported DASH number format")?;
                let n: usize = digits.parse()?;
                ensure!((1..=20).contains(&n), "Invalid DASH number width");
                n
            } else {
                0
            };
            output.push_str(&format!("{value:0width$}"));
        }
        ensure!(output.len() <= 16384, "DASH URL is too long");
    }
    output.push_str(rest);
    ensure!(output.len() <= 16384, "DASH URL is too long");
    Ok(output)
}
