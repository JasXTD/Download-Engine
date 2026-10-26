//! PKWARE APPNOTE central-directory metadata. No local file paths are created.
use super::{Entry, MAX_DIRECTORY_BYTES, MAX_ENTRIES};
use anyhow::{Context, Result, ensure};

fn u16_at(b: &[u8], p: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(p..p + 2)
            .context("Truncated ZIP metadata")?
            .try_into()?,
    ))
}
fn u32_at(b: &[u8], p: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(p..p + 4)
            .context("Truncated ZIP metadata")?
            .try_into()?,
    ))
}
fn u64_at(b: &[u8], p: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        b.get(p..p + 8)
            .context("Truncated ZIP64 metadata")?
            .try_into()?,
    ))
}

pub(super) struct Directory {
    pub offset: u64,
    pub length: u64,
    count: u64,
    boundary: u64,
}
impl Directory {
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            self.count <= MAX_ENTRIES as u64 && self.length <= MAX_DIRECTORY_BYTES,
            "ZIP preview is limited to 10,000 entries and 4 MiB of directory metadata"
        );
        ensure!(
            self.offset.checked_add(self.length) == Some(self.boundary),
            "Invalid ZIP directory bounds or unsupported archive layout"
        );
        ensure!(
            self.count <= self.length / 46,
            "ZIP entry count exceeds its directory size"
        );
        Ok(())
    }
}
pub(super) struct End {
    pub directory: Directory,
    pub zip64_offset: Option<u64>,
}
pub(super) fn end(tail: &[u8], start: u64) -> Result<End> {
    ensure!(tail.len() >= 22, "Not a complete ZIP archive");
    let at = (0..=tail.len() - 22)
        .rev()
        .find(|&p| {
            tail[p..].starts_with(b"PK\x05\x06")
                && u16_at(tail, p + 20).is_ok_and(|n| p + 22 + usize::from(n) == tail.len())
        })
        .context(
            "No ZIP directory found. This may not be a ZIP file, or its directory is encrypted.",
        )?;
    let b = &tail[at..];
    ensure!(
        u16_at(b, 4)? == 0 && u16_at(b, 6)? == 0 && u16_at(b, 8)? == u16_at(b, 10)?,
        "Split ZIP archives are not supported by preview"
    );
    let count = u64::from(u16_at(b, 10)?);
    let length = u64::from(u32_at(b, 12)?);
    let offset = u64::from(u32_at(b, 16)?);
    let sentinel =
        count == u16::MAX as u64 || length == u32::MAX as u64 || offset == u32::MAX as u64;
    let locator = at
        .checked_sub(20)
        .filter(|&p| tail[p..].starts_with(b"PK\x06\x07"));
    ensure!(
        !sentinel || locator.is_some(),
        "Missing ZIP64 directory locator"
    );
    let zip64_offset = if let Some(at) = locator {
        ensure!(
            u32_at(tail, at + 4)? == 0 && u32_at(tail, at + 16)? == 1,
            "Split ZIP64 archives are not supported by preview"
        );
        let offset = u64_at(tail, at + 8)?;
        ensure!(
            offset.checked_add(56) == Some(start + at as u64),
            "Unsupported ZIP64 directory extension or invalid offset"
        );
        Some(offset)
    } else {
        None
    };
    Ok(End {
        directory: Directory {
            offset,
            length,
            count,
            boundary: start + at as u64,
        },
        zip64_offset,
    })
}
pub(super) fn zip64(bytes: &[u8], offset: u64, end: &End) -> Result<Directory> {
    ensure!(
        bytes.starts_with(b"PK\x06\x06") && u64_at(bytes, 4)? == 44,
        "Unsupported or invalid ZIP64 directory record"
    );
    ensure!(
        u32_at(bytes, 16)? == 0
            && u32_at(bytes, 20)? == 0
            && u64_at(bytes, 24)? == u64_at(bytes, 32)?,
        "Split ZIP64 archives are not supported by preview"
    );
    let result = Directory {
        count: u64_at(bytes, 32)?,
        length: u64_at(bytes, 40)?,
        offset: u64_at(bytes, 48)?,
        boundary: offset,
    };
    for (old, sentinel, actual) in [
        (end.directory.count, u16::MAX as u64, result.count),
        (end.directory.length, u32::MAX as u64, result.length),
        (end.directory.offset, u32::MAX as u64, result.offset),
    ] {
        ensure!(
            old == sentinel || old == actual,
            "ZIP and ZIP64 directories disagree"
        );
    }
    Ok(result)
}

pub(super) fn parse(bytes: &[u8], directory: &Directory) -> Result<(Vec<Entry>, u64)> {
    directory.validate()?;
    ensure!(
        bytes.len() as u64 == directory.length,
        "ZIP directory length changed"
    );
    let cp437: Vec<char> = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■ ".chars().collect();
    let mut entries = Vec::with_capacity(directory.count as usize);
    let mut at = 0;
    let mut expanded_size = 0u64;
    for _ in 0..directory.count {
        let b = bytes.get(at..).context("Truncated ZIP directory")?;
        ensure!(
            b.len() >= 46 && b.starts_with(b"PK\x01\x02"),
            "Invalid, compressed or encrypted ZIP directory"
        );
        let flags = u16_at(b, 8)?;
        ensure!(
            flags & 0x2000 == 0,
            "Encrypted ZIP directories are not supported by preview"
        );
        let name_len = usize::from(u16_at(b, 28)?);
        let extra_len = usize::from(u16_at(b, 30)?);
        let comment_len = usize::from(u16_at(b, 32)?);
        ensure!(
            (1..=4096).contains(&name_len),
            "ZIP entry name exceeds the preview limit or is empty"
        );
        let record_len = 46 + name_len + extra_len + comment_len;
        ensure!(record_len <= b.len(), "Truncated ZIP directory entry");
        let raw_name = &b[46..46 + name_len];
        let mut name = if flags & 0x800 != 0 {
            std::str::from_utf8(raw_name)
                .context("Invalid UTF-8 ZIP filename")?
                .to_owned()
        } else {
            raw_name
                .iter()
                .map(|&b| {
                    if b < 128 {
                        char::from(b)
                    } else {
                        cp437[(b - 128) as usize]
                    }
                })
                .collect()
        };
        let mut size = u64::from(u32_at(b, 24)?);
        let mut compressed = u64::from(u32_at(b, 20)?);
        let mut local = u64::from(u32_at(b, 42)?);
        let mut disk = u32::from(u16_at(b, 34)?);
        let mut zip64_seen = false;
        let mut unicode_seen = false;
        let mut extra = &b[46 + name_len..46 + name_len + extra_len];
        while !extra.is_empty() {
            let tag = u16_at(extra, 0)?;
            let len = usize::from(u16_at(extra, 2)?);
            let value = extra.get(4..4 + len).context("Truncated ZIP extra field")?;
            if tag == 1 {
                ensure!(!zip64_seen, "Duplicate ZIP64 extra field");
                zip64_seen = true;
                let mut p = 0;
                for number in [&mut size, &mut compressed, &mut local] {
                    if *number == u32::MAX as u64 {
                        *number = u64_at(value, p)?;
                        p += 8;
                    }
                }
                if disk == u16::MAX as u32 {
                    disk = u32_at(value, p)?;
                }
            } else if tag == 0x7075 {
                ensure!(!unicode_seen, "Duplicate Unicode filename field");
                unicode_seen = true;
                if value.first() == Some(&1)
                    && value.len() >= 5
                    && u32_at(value, 1)? == crc32(raw_name)
                {
                    let unicode =
                        std::str::from_utf8(&value[5..]).context("Invalid Unicode ZIP filename")?;
                    ensure!(
                        !unicode.is_empty() && unicode.len() <= 4096,
                        "Invalid Unicode ZIP filename length"
                    );
                    if flags & 0x800 != 0 {
                        ensure!(unicode == name, "Conflicting ZIP filenames");
                    }
                    name = unicode.to_owned();
                }
            }
            extra = &extra[4 + len..];
        }
        ensure!(
            zip64_seen
                || (size != u32::MAX as u64
                    && compressed != u32::MAX as u64
                    && local != u32::MAX as u64),
            "Missing ZIP64 entry metadata"
        );
        ensure!(disk == 0, "Split ZIP entries are not supported by preview");
        ensure!(
            local
                .checked_add(30)
                .and_then(|n| n.checked_add(compressed))
                .is_some_and(|n| n <= directory.offset),
            "ZIP entry points outside its file data"
        );
        expanded_size = expanded_size
            .checked_add(size)
            .context("ZIP expanded size overflow")?;
        let is_directory = name.ends_with('/') || name.ends_with('\\');
        let unusual_path = name.starts_with(['/', '\\'])
            || name.contains(':')
            || name.split(['/', '\\']).any(|p| p == "..")
            || name.chars().any(hidden);
        let name = name
            .chars()
            .map(|c| {
                if hidden(c) {
                    c.escape_default().to_string()
                } else {
                    c.to_string()
                }
            })
            .collect();
        entries.push(Entry {
            name,
            size,
            compressed_size: compressed,
            directory: is_directory,
            encrypted: flags & 1 != 0,
            unusual_path,
        });
        at += record_len;
    }
    // Optional central directory digital signature (not a trust assertion).
    if bytes
        .get(at..)
        .is_some_and(|b| b.starts_with(b"PK\x05\x05"))
    {
        at += 6 + usize::from(u16_at(bytes, at + 4)?);
    }
    ensure!(
        at == bytes.len(),
        "ZIP directory contains unexpected or missing entries"
    );
    Ok((entries, expanded_size))
}
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    fn preview(bytes: &[u8]) -> Result<(Vec<Entry>, u64)> {
        let end = end(bytes, 0)?;
        let directory = if let Some(at) = end.zip64_offset {
            zip64(
                bytes
                    .get(at as usize..at as usize + 56)
                    .context("Missing ZIP64 record")?,
                at,
                &end,
            )?
        } else {
            end.directory
        };
        directory.validate()?;
        let data = bytes
            .get(directory.offset as usize..(directory.offset + directory.length) as usize)
            .context("Missing directory")?;
        parse(data, &directory)
    }
    #[test]
    fn truncated_and_mutated_archive_corpus_never_panics() {
        let bytes = include_bytes!("../../tests/fixtures/zip/dotnet.zip");
        for length in 0..bytes.len() {
            assert!(preview(&bytes[..length]).is_err());
        }
        for index in 0..bytes.len() {
            for byte in [0, 0xff, b'P', b'K'] {
                let mut changed = bytes.to_vec();
                changed[index] = byte;
                let _ = preview(&changed); // Metadata changes may remain valid; none may panic.
            }
        }
    }
    #[test]
    fn cp437_names_and_unicode_path_crc_are_respected() {
        fn record(name: &[u8], extra: &[u8]) -> Vec<u8> {
            let mut b = vec![0; 46];
            b[..4].copy_from_slice(b"PK\x01\x02");
            b[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
            b[30..32].copy_from_slice(&(extra.len() as u16).to_le_bytes());
            b.extend(name);
            b.extend(extra);
            b
        }
        let raw = b"\x82.txt";
        let unicode = "日本語.txt".as_bytes();
        let mut extra = vec![0x75, 0x70];
        extra.extend(((unicode.len() + 5) as u16).to_le_bytes());
        extra.push(1);
        extra.extend(crc32(raw).to_le_bytes());
        extra.extend(unicode);
        for case in 0..3 {
            let extra = match case {
                0 => Vec::new(),
                1 => extra.clone(),
                _ => {
                    let mut bad = extra.clone();
                    bad[5] ^= 1;
                    bad
                }
            };
            let bytes = record(raw, &extra);
            let dir = Directory {
                offset: 100,
                length: bytes.len() as u64,
                count: 1,
                boundary: 100 + bytes.len() as u64,
            };
            let (entries, _) = parse(&bytes, &dir).unwrap();
            assert_eq!(
                entries[0].name,
                if case == 1 { "日本語.txt" } else { "é.txt" }
            );
        }
    }
}
