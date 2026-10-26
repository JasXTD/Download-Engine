//! Validate packed ADTS AAC and Apple's ID3 transport timestamps before remuxing.
//! No metadata is interpreted as a URL or passed to an external program.
use anyhow::{Context, Result, ensure};
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::sync::CancellationToken;

fn syncsafe(bytes: &[u8]) -> Result<u32> {
    ensure!(
        bytes.len() == 4 && bytes.iter().all(|b| b & 0x80 == 0),
        "Invalid AAC ID3 size"
    );
    Ok(bytes.iter().fold(0, |size, b| (size << 7) | u32::from(*b)))
}

fn timestamp(version: u8, tag: &[u8]) -> Result<u64> {
    let mut offset = 0;
    let mut timestamp = None;
    while offset + 10 <= tag.len() && tag[offset] != 0 {
        let head = &tag[offset..offset + 10];
        ensure!(
            head[..4].iter().all(u8::is_ascii_alphanumeric) && head[8..] == [0, 0],
            "Unsupported AAC ID3 frame"
        );
        let len = if version == 4 {
            syncsafe(&head[4..8])?
        } else {
            u32::from_be_bytes(head[4..8].try_into()?)
        } as usize;
        offset += 10;
        let frame = tag
            .get(offset..offset + len)
            .context("Truncated AAC ID3 frame")?;
        if &head[..4] == b"PRIV"
            && let Some(value) =
                frame.strip_prefix(b"com.apple.streaming.transportStreamTimestamp\0")
        {
            ensure!(
                timestamp.is_none() && value.len() == 8,
                "Invalid AAC transport timestamp"
            );
            let ticks = u64::from_be_bytes(value.try_into()?);
            ensure!(ticks < 1 << 33, "AAC transport timestamp exceeds 33 bits");
            timestamp = Some(ticks);
        }
        offset += len;
    }
    ensure!(
        tag[offset..].iter().all(|b| *b == 0),
        "Invalid AAC ID3 padding"
    );
    timestamp.context("Packed AAC has no transport timestamp")
}

/// Works on one segment or a concatenated local track. Every ID3 boundary must
/// agree with the frame clock; silently removing an audio gap would desync video.
pub(super) async fn validate(path: &Path, cancel: &CancellationToken) -> Result<u64> {
    let mut file = crate::download_file::read_part(path).await?;
    validate_handle(&mut file, cancel).await
}

pub(super) async fn validate_handle(
    file: &mut tokio::fs::File,
    cancel: &CancellationToken,
) -> Result<u64> {
    file.rewind().await?;
    let size = file.metadata().await?.len();
    let mut offset = 0u64;
    let (mut first, mut frames, mut rate, mut config) = (None::<u64>, 0u64, 0u64, None);
    while offset < size {
        ensure!(!cancel.is_cancelled(), "Packed AAC validation cancelled");
        ensure!(size - offset >= 7, "Truncated packed AAC header");
        let mut head = [0u8; 7];
        file.read_exact(&mut head).await?;
        if &head[..3] == b"ID3" {
            ensure!(
                matches!(head[3], 3 | 4) && head[4] == 0 && head[5] == 0 && size - offset >= 10,
                "Unsupported packed AAC ID3 version or flags"
            );
            let mut rest = [0; 3];
            file.read_exact(&mut rest).await?;
            let len = syncsafe(&[head[6], rest[0], rest[1], rest[2]])? as u64;
            ensure!(
                len <= 1024 * 1024 && len + 10 <= size - offset,
                "Oversized or truncated AAC metadata"
            );
            let mut tag = vec![0; len as usize];
            file.read_exact(&mut tag).await?;
            let ticks = timestamp(head[3], &tag)?;
            if let Some(start) = first {
                ensure!(
                    frames > 0 && rate > 0,
                    "AAC metadata has no intervening frames"
                );
                let expected = (start + frames * 1024 * 90_000 / rate) % (1 << 33);
                let difference = expected.abs_diff(ticks);
                ensure!(
                    difference.min((1 << 33) - difference) <= 90,
                    "Packed AAC timeline has a gap or overlap; cannot safely merge it"
                );
            } else {
                first = Some(ticks);
            }
            offset += len + 10;
        } else {
            ensure!(
                first.is_some() && head[0] == 0xff && head[1] & 0xf6 == 0xf0 && head[6] & 3 == 0,
                "Invalid packed AAC frame"
            );
            let index = usize::from((head[2] >> 2) & 15);
            let sample_rate = *[
                96000u64, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025,
                8000, 7350,
            ]
            .get(index)
            .context("Invalid packed AAC sample rate")?;
            let current = (head[2] & 0xfd, head[3] & 0xc0);
            ensure!(
                config.is_none_or(|c| c == current),
                "Packed AAC encoding changed"
            );
            config = Some(current);
            rate = sample_rate;
            let len = (u64::from(head[3] & 3) << 11)
                | (u64::from(head[4]) << 3)
                | u64::from(head[5] >> 5);
            let minimum = if head[1] & 1 == 0 { 9 } else { 7 };
            ensure!(
                len > minimum && len <= size - offset,
                "Invalid or truncated packed AAC frame length"
            );
            offset += len;
            frames += 1;
            file.seek(std::io::SeekFrom::Start(offset)).await?;
        }
    }
    ensure!(frames > 0, "Packed AAC contains no audio frames");
    first.context("Packed AAC has no timestamp")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn assembled_audio_can_be_validated_without_reopening_the_windows_write_guard() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("assembled.part");
        let bytes = [
            tag(28800),
            vec![0xff, 0xf1, 0x4c, 0x80, 0x01, 0x1f, 0xfc, 0],
        ]
        .concat();
        let mut writer = crate::download_file::fresh_read_write(&path).await.unwrap();
        writer.write_all(&bytes).await.unwrap();
        writer.sync_all().await.unwrap();
        let guard = writer.into_std().await;
        let mut reader = tokio::fs::File::from_std(guard.try_clone().unwrap());
        assert_eq!(
            validate_handle(&mut reader, &CancellationToken::new())
                .await
                .unwrap(),
            28800
        );
    }
    fn tag(ticks: u64) -> Vec<u8> {
        let mut payload = b"com.apple.streaming.transportStreamTimestamp\0".to_vec();
        payload.extend_from_slice(&ticks.to_be_bytes());
        let mut frame = b"PRIV".to_vec();
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&[0, 0]);
        frame.extend(payload);
        let mut tag = b"ID3\x03\x00\x00\x00\x00\x00".to_vec();
        tag.push(frame.len() as u8);
        tag.extend(frame);
        tag
    }
    #[tokio::test]
    async fn packed_aac_requires_valid_frames_timestamps_and_a_continuous_clock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.part");
        // AAC LC, 48 kHz, stereo, 8 byte ADTS frame (payload is inert fixture data).
        let frame = [0xff, 0xf1, 0x4c, 0x80, 0x01, 0x1f, 0xfc, 0];
        let good = [tag(28800), frame.to_vec(), tag(30720), frame.to_vec()].concat();
        tokio::fs::write(&path, &good).await.unwrap();
        assert_eq!(
            validate(&path, &CancellationToken::new()).await.unwrap(),
            28800
        );
        for bad in [
            b"<html>login</html>".to_vec(),
            tag(0),
            [tag(0), frame.to_vec(), tag(50000), frame.to_vec()].concat(),
            good[..good.len() - 1].to_vec(),
        ] {
            tokio::fs::write(&path, bad).await.unwrap();
            assert!(validate(&path, &CancellationToken::new()).await.is_err());
        }
        let cancel = CancellationToken::new();
        cancel.cancel();
        tokio::fs::write(&path, good).await.unwrap();
        assert!(
            validate(&path, &cancel)
                .await
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
    }
}
