//! Bounded flat SIDX indexes. All offsets are relative to the containing resource,
//! never to the beginning of the HTTP response. No external or nested indexes.
use crate::{
    engine,
    hls::{MAX_PART_BYTES, MAX_PARTS, Part, Range, network::Network},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

pub(super) const MAX_INDEX_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct Address {
    pub url: String,
    pub range: Range,
    pub initialization: Part,
    pub offset_us: u64,
    pub exact: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexIdentity {
    url: String,
    range: Range,
    sha256: String,
    total: u64,
    etag: Option<String>,
    last_modified: Option<String>,
}
impl IndexIdentity {
    /// Same media bytes behind a possibly different (re-signed or mirrored) URL.
    pub(crate) fn same_media(&self, other: &Self) -> bool {
        self.range == other.range && self.sha256 == other.sha256 && self.total == other.total
    }
    pub(super) fn validate(&self, parts: &[Part]) -> Result<()> {
        crate::model::validate_url(&self.url)?;
        ensure!(
            self.total > 0
                && self.total <= i64::MAX as u64
                && self.range.length > 0
                && self.range.length <= MAX_INDEX_BYTES
                && self
                    .range
                    .start
                    .checked_add(self.range.length)
                    .is_some_and(|end| end <= self.total)
                && self.sha256.len() == 64
                && self.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                && [&self.etag, &self.last_modified].iter().all(|v| v
                    .as_ref()
                    .is_none_or(
                        |s| s.len() <= 1024 && s.bytes().all(|b| (0x20..=0x7e).contains(&b))
                    ))
                && parts.iter().all(|p| p.url == self.url
                    && p.range.as_ref().is_some_and(|r| r
                        .start
                        .checked_add(r.length)
                        .is_some_and(|end| end <= self.total))),
            "Invalid saved DASH index identity"
        );
        Ok(())
    }
    pub(crate) fn check_response(&self, part: &Part, response: &reqwest::Response) -> Result<()> {
        if part.url == self.url {
            ensure!(
                engine::range(response)?.2 == self.total
                    && self
                        .etag
                        .as_ref()
                        .is_none_or(|v| engine::strong_etag(response).as_ref() == Some(v))
                    && self.last_modified.as_ref().is_none_or(|v| response
                        .headers()
                        .get(reqwest::header::LAST_MODIFIED)
                        .and_then(|h| h.to_str().ok())
                        == Some(v.as_str())),
                "DASH media resource changed. Preview this video again before downloading."
            );
        }
        Ok(())
    }
}
impl Address {
    pub(super) async fn resolve(
        &self,
        network: &mut Network,
        duration_ms: u64,
        cancel: &CancellationToken,
    ) -> Result<(Vec<Part>, IndexIdentity)> {
        let (bytes, response) = network.index(&self.url, &self.range, cancel).await?;
        let total = engine::range(&response)?.2;
        let identity = IndexIdentity {
            url: self.url.clone(),
            range: self.range.clone(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            total,
            etag: engine::strong_etag(&response).filter(|s| s.len() <= 1024),
            last_modified: response
                .headers()
                .get(reqwest::header::LAST_MODIFIED)
                .and_then(|h| h.to_str().ok())
                .filter(|s| s.len() <= 1024)
                .map(str::to_owned),
        };
        let ranges = parse(
            &bytes,
            &self.range,
            total,
            self.offset_us,
            duration_ms,
            self.exact,
        )?;
        ensure!(
            (ranges.len() + 1).saturating_mul(self.url.len() + 64) <= 4 * 1024 * 1024,
            "DASH index expands to too much segment data"
        );
        let mut parts = vec![self.initialization.clone()];
        parts.extend(ranges.into_iter().map(|range| Part {
            url: self.url.clone(),
            range: Some(range),
            initialization: false,
        }));
        identity.validate(&parts)?;
        Ok((parts, identity))
    }
}

fn parse(
    bytes: &[u8],
    requested: &Range,
    total: u64,
    offset_us: u64,
    duration_ms: u64,
    exact: bool,
) -> Result<Vec<Range>> {
    ensure!(
        bytes.len() as u64 == requested.length && requested.length <= MAX_INDEX_BYTES,
        "Incomplete or oversized DASH index"
    );
    let mut position = 0;
    let mut found = None;
    while position < bytes.len() {
        let mut reader = Reader::new(&bytes[position..]);
        let size = reader.u32()? as u64;
        let kind = reader.take(4)?;
        let size = if size == 1 { reader.u64()? } else { size };
        ensure!(
            size >= reader.position as u64 && size <= (bytes.len() - position) as u64,
            "Invalid DASH index box size"
        );
        if kind == b"sidx" {
            ensure!(
                found.is_none() && (!exact || (position == 0 && size == bytes.len() as u64)),
                "DASH index must contain one SIDX box"
            );
            let end = requested
                .start
                .checked_add(position as u64)
                .and_then(|p| p.checked_add(size))
                .context("DASH index offset overflow")?;
            found = Some(references(
                &bytes[position + reader.position..position + size as usize],
                end,
                total,
                offset_us,
                duration_ms,
            )?);
        } else {
            ensure!(
                !exact && matches!(kind, b"free" | b"skip" | b"styp" | b"ftyp"),
                "DASH index contains unsupported boxes"
            );
        }
        position += size as usize;
    }
    found.context("DASH index has no SIDX box")
}
fn references(
    bytes: &[u8],
    index_end: u64,
    total: u64,
    offset_us: u64,
    duration_ms: u64,
) -> Result<Vec<Range>> {
    let mut reader = Reader::new(bytes);
    let full = reader.u32()?;
    ensure!(
        full == 0 || full == 1 << 24,
        "Unsupported SIDX version or flags"
    );
    let _track_id = reader.u32()?;
    let scale = reader.u32()? as u64;
    ensure!(scale > 0, "SIDX timescale must be positive");
    let (mut time, first_offset) = if full == 0 {
        (reader.u32()? as u64, reader.u32()? as u64)
    } else {
        (reader.u64()?, reader.u64()?)
    };
    ensure!(reader.u16()? == 0, "Invalid SIDX reserved bits");
    let count = reader.u16()? as usize;
    ensure!(
        count > 0 && count < MAX_PARTS && bytes.len() - reader.position == count * 12,
        "Invalid or excessive SIDX reference count"
    );
    let mut start = index_end
        .checked_add(first_offset)
        .context("SIDX byte offset overflow")?;
    let period_end = u128::from(offset_us) + u128::from(duration_ms) * 1000;
    let mut ranges = Vec::new();
    for _ in 0..count {
        let reference = reader.u32()?;
        ensure!(
            reference >> 31 == 0,
            "Hierarchical SIDX indexes are not supported"
        );
        let length = reference as u64;
        let duration = reader.u32()? as u64;
        let _sap = reader.u32()?;
        ensure!(
            length > 0 && length <= MAX_PART_BYTES && duration > 0,
            "Invalid SIDX segment size or duration"
        );
        let end = start
            .checked_add(length)
            .context("SIDX byte offset overflow")?;
        ensure!(
            end <= total && total <= i64::MAX as u64,
            "SIDX segment exceeds its media resource"
        );
        let next = time
            .checked_add(duration)
            .context("SIDX timestamp overflow")?;
        let start_us = u128::from(time) * 1_000_000 / u128::from(scale);
        let end_us = u128::from(next) * 1_000_000 / u128::from(scale);
        if end_us > u128::from(offset_us) && start_us < period_end {
            ensure!(
                !ranges.is_empty() || start_us >= u128::from(offset_us),
                "SIDX segments crossing the period start are not supported"
            );
            ranges.push(Range { start, length });
        }
        time = next;
        start = end;
    }
    ensure!(!ranges.is_empty(), "SIDX has no segments in this period");
    Ok(ranges)
}
struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    fn take(&mut self, size: usize) -> Result<&'a [u8]> {
        let value = self
            .bytes
            .get(self.position..self.position + size)
            .context("Truncated SIDX box")?;
        self.position += size;
        Ok(value)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into()?))
    }
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
