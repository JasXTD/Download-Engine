//! Check initialization metadata for protection before handing media to a helper.
use anyhow::{Context, Result, ensure};

pub(super) fn clear_initialization(bytes: &[u8]) -> Result<()> {
    boxes(bytes, 0)
}
fn boxes(mut data: &[u8], depth: usize) -> Result<()> {
    ensure!(depth <= 12, "MP4 initialization nesting is too deep");
    while !data.is_empty() {
        ensure!(data.len() >= 8, "Truncated MP4 metadata");
        let size = u32::from_be_bytes(data[..4].try_into().unwrap()) as u64;
        let kind = &data[4..8];
        let (size, header) = if size == 1 {
            ensure!(data.len() >= 16, "Truncated MP4 metadata");
            (u64::from_be_bytes(data[8..16].try_into().unwrap()), 16)
        } else if size == 0 {
            (data.len() as u64, 8)
        } else {
            (size, 8)
        };
        let size = usize::try_from(size).context("Invalid MP4 metadata size")?;
        ensure!(
            size >= header && size <= data.len(),
            "Invalid MP4 metadata size"
        );
        ensure!(
            !matches!(kind, b"pssh" | b"sinf" | b"tenc" | b"encv" | b"enca"),
            "Encrypted / DRM media is not supported"
        );
        let body = &data[header..size];
        if matches!(kind, b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl") {
            boxes(body, depth + 1)?;
        }
        if kind == b"stsd" {
            ensure!(body.len() >= 8, "Truncated MP4 sample descriptions");
            // Sample entries are boxes; protected audio/video entries are enca/encv.
            boxes(&body[8..], depth + 1)?;
        }
        if matches!(
            kind,
            b"avc1" | b"avc3" | b"hvc1" | b"hev1" | b"av01" | b"vp09"
        ) {
            ensure!(body.len() >= 78, "Truncated MP4 video sample entry");
            boxes(&body[78..], depth + 1)?;
        }
        if matches!(kind, b"mp4a" | b"Opus" | b"fLaC" | b"ac-3" | b"ec-3") {
            ensure!(body.len() >= 28, "Truncated MP4 audio sample entry");
            let extra = match u16::from_be_bytes(body[8..10].try_into().unwrap()) {
                0 => 0,
                1 => 16,
                2 => 36,
                _ => anyhow::bail!("Unsupported MP4 audio sample version"),
            };
            ensure!(body.len() >= 28 + extra, "Truncated MP4 audio sample entry");
            boxes(&body[28 + extra..], depth + 1)?;
        }
        data = &data[size..];
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [(body.len() as u32 + 8).to_be_bytes().as_slice(), kind, body].concat()
    }
    #[test]
    fn protected_sample_entries_and_protection_headers_are_rejected() {
        for kind in [b"encv", b"enca"] {
            let sample = bx(kind, &[]);
            let stsd = bx(
                b"stsd",
                &[[0, 0, 0, 0, 0, 0, 0, 1].as_slice(), &sample].concat(),
            );
            let moov = bx(
                b"moov",
                &bx(b"trak", &bx(b"mdia", &bx(b"minf", &bx(b"stbl", &stsd)))),
            );
            assert!(
                clear_initialization(&moov)
                    .unwrap_err()
                    .to_string()
                    .contains("Encrypted")
            );
        }
        assert!(clear_initialization(&bx(b"pssh", &[])).is_err());
        assert!(clear_initialization(&bx(b"moov", &bx(b"stsd", &[0, 0]))).is_err());
        assert!(
            clear_initialization(&bx(
                b"moov",
                &bx(
                    b"stsd",
                    &[[0, 0, 0, 0, 0, 0, 0, 1].as_slice(), &bx(b"avc1", &[0; 78])].concat()
                )
            ))
            .is_ok()
        );
        let disguised = bx(b"avc1", &[vec![0; 78], bx(b"sinf", &[])].concat());
        assert!(
            clear_initialization(&disguised)
                .unwrap_err()
                .to_string()
                .contains("Encrypted")
        );
    }
}
