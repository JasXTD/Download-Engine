use super::{
    CompletedPart, Format, MAX_PART_BYTES, Playlist,
    network::Network,
    parser::{self, Parsed},
};
use crate::{
    bandwidth::Limiter,
    download_file, engine,
    model::Job,
    store::Store,
    synchronization::FinishedFile,
    transfer_activity::{MediaSnapshot, SegmentState},
};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

fn part_path(root: &Path, index: usize) -> PathBuf {
    root.join(format!("hls-{index}.part"))
}

pub(crate) async fn transfer(
    store: &Store,
    job: &mut Job,
    root: &Path,
    cancel: &CancellationToken,
    limiter: &Limiter,
) -> Result<Option<FinishedFile>> {
    let route = crate::proxy::Route::load(store)?;
    let download = job.hls.as_ref().context("Missing HLS download")?;
    download.validate()?;
    if download.youtube.is_some() {
        // Re-resolve from the stable watch URL before every start/resume. The
        // resolver verifies encoding versions before retaining completed parts.
        let refreshed = crate::youtube::refresh(download, cancel, route.clone()).await?;
        job.hls.as_mut().unwrap().playlist = refreshed;
        store.save(job)?;
    } else if download.bilibili.is_some() {
        // Bilibili's signed stream links expire; re-read them from the saved page.
        // The resolver checks the media is unchanged before keeping completed parts.
        let refreshed = crate::bilibili::refresh(download, cancel, route.clone()).await?;
        job.hls.as_mut().unwrap().playlist = refreshed;
        store.save(job)?;
    }
    // TikTok's video servers need the page's fresh visitor cookies; they stay in memory.
    let mut site = None;
    let download = job.hls.as_ref().unwrap();
    if download.tiktok.is_some() {
        let (refreshed, headers) = crate::tiktok::refresh(download, cancel, route.clone()).await?;
        job.hls.as_mut().unwrap().playlist = refreshed;
        store.save(job)?;
        site = Some(headers);
    }
    let download = job.hls.as_ref().unwrap();
    // These playlists were just rebuilt from the site's own reply; there is no
    // manifest or playlist URL to re-read.
    let site_built = download.bilibili.is_some() || download.tiktok.is_some();
    let playlist = download.playlist.clone();
    // Bilibili's video servers expect its own page as the Referer (saved downloads
    // from older versions have no source page); browser-found streams send theirs.
    let page = if download.bilibili.is_some() {
        Some(playlist.source.clone())
    } else {
        job.source_page.clone()
    };
    let merger = if playlist.needs_merge() {
        Some(super::merge::Helper::checked(cancel).await?)
    } else {
        None
    };
    let mut network = Network::via(&playlist.source, route)
        .await?
        .with_page(page.as_deref())
        .with_public_only(job.public_only || playlist.public_only)?;
    if let Some(site) = site {
        network = network.with_site(site);
    }
    // Recheck encryption, segment order and the complete finite playlist before
    // reusing any bytes. A changed playlist needs a fresh user preview. Site
    // playlists have no manifest URL; the refresh above already rebuilt and checked them.
    if site_built {
        // Rebuilt and checked by the site refresh above.
    } else if playlist.dash.is_some() {
        let (url, text) = network.playlist(&playlist.url, cancel).await?;
        crate::dash::parse(&playlist.source, &url, &text)?
            .dash
            .context("Missing DASH manifest")?
            .recheck(&playlist, &mut network, cancel)
            .await?;
    } else {
        recheck(&mut network, &playlist, cancel).await?;
        if let Some(audio) = &playlist.audio {
            recheck(&mut network, &audio.playlist(&playlist.source), cancel).await?;
        }
    }
    let parts: Vec<_> = playlist
        .parts
        .iter()
        .map(|p| (p, playlist.format))
        .chain(
            playlist
                .audio
                .iter()
                .flat_map(|a| a.parts.iter().map(|p| (p, a.format))),
        )
        .collect();
    job.downloaded = 0;
    store
        .activity
        .set(&job.id, crate::transfer_activity::Phase::CheckingSaved);
    job.total = None;
    for (index, completed) in job.hls.as_ref().unwrap().completed.iter().enumerate() {
        let mut file = download_file::read_part(&part_path(root, index))
            .await
            .context("Saved HLS segment is missing. Restart this download.")?;
        ensure!(
            file.metadata().await?.len() == completed.size,
            "Saved HLS segment changed. Restart this download."
        );
        let mut hash = Sha256::new();
        let mut buffer = vec![0; 256 * 1024];
        loop {
            if cancel.is_cancelled() {
                return Ok(None);
            }
            let count = file.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        ensure!(
            format!("{:x}", hash.finalize()) == completed.sha256,
            "Saved HLS segment changed. Restart this download."
        );
        job.downloaded += completed.size;
    }
    store.save(job)?;
    let start = job.hls.as_ref().unwrap().completed.len();
    store
        .activity
        .set(&job.id, crate::transfer_activity::Phase::Downloading);
    for (index, (part, format)) in parts.iter().enumerate().skip(start) {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        limiter.check_quota().await?;
        let mut activity = MediaSnapshot {
            index,
            downloaded: 0,
            total: part.range.as_ref().map(|range| range.length),
            state: SegmentState::SendingGet,
        };
        store.activity.set_media(&job.id, activity);
        let mut response = network.get(&part.url, part.range.as_ref(), cancel).await?;
        if let Some(dash) = &playlist.dash {
            for index in dash.video_index.iter().chain(&dash.audio_index) {
                index.check_response(part, &response)?;
            }
        }
        let expected = part
            .range
            .as_ref()
            .map(|r| r.length)
            .or(response.content_length());
        ensure!(
            expected.is_none_or(|size| size > 0 && size <= MAX_PART_BYTES),
            "HLS segment exceeds 256 MiB or is empty"
        );
        activity.total = expected;
        activity.state = SegmentState::ReceivingData;
        store.activity.set_media(&job.id, activity);
        let path = part_path(root, index);
        let mut file = download_file::fresh(&path).await?;
        let mut received = 0u64;
        let mut hash = Sha256::new();
        let mut checkpoint = Instant::now();
        let result: Result<bool> = async {
            loop {
                if expected != Some(received) { limiter.check_quota().await?; }
                let chunk = tokio::select! { _=cancel.cancelled()=>return Ok(false), result=engine::network_wait(response.chunk())=>result? };
                let Some(chunk) = chunk else { break; };
                limiter.record_payload(chunk.len()).await?;
                received += chunk.len() as u64;
                ensure!(received <= expected.unwrap_or(MAX_PART_BYTES), "HLS segment is larger than declared or exceeds 256 MiB");
                let mut remaining = chunk.as_ref();
                while !remaining.is_empty() {
                    let Some(n) = limiter.acquire(&job.id,remaining.len(),cancel).await else { return Ok(false); };
                    file.write_all(&remaining[..n]).await?;
                    hash.update(&remaining[..n]);
                    job.downloaded += n as u64;
                    activity.downloaded += n as u64;
                    store.activity.set_media(&job.id, activity);
                    remaining = &remaining[n..];
                    if checkpoint.elapsed() >= Duration::from_millis(500) { store.save(job)?; checkpoint = Instant::now(); }
                }
            }
            if expected.is_some_and(|size| size != received) { return Err(crate::retry::incomplete("HLS segment ended before all bytes arrived.")); }
            ensure!(received > 0, "HLS segment is empty");
            Ok(true)
        }.await;
        // Finish buffered writes on cancellation or failure before another
        // worker can restart this segment. Only complete segments are resumable.
        file.sync_all().await?;
        drop(file);
        if !result? {
            return Ok(None);
        }
        activity.state = SegmentState::Verifying;
        store.activity.set_media(&job.id, activity);
        validate_media(&path, *format, part.initialization, index == 0, cancel).await?;
        job.hls.as_mut().unwrap().completed.push(CompletedPart {
            size: received,
            sha256: format!("{:x}", hash.finalize()),
        });
        store.save(job)?;
        activity.state = SegmentState::Completed;
        store.activity.set_media(&job.id, activity);
    }
    job.total = Some(job.downloaded);
    store.save(job)?;
    if let Some(merger) = merger {
        return super::merge::finish(store, job, root, cancel, merger).await;
    }
    engine::finalize_sources(
        store,
        job,
        cancel,
        (0..playlist.parts.len())
            .map(|i| part_path(root, i))
            .collect(),
    )
    .await
}

async fn recheck(
    network: &mut Network,
    playlist: &Playlist,
    cancel: &CancellationToken,
) -> Result<()> {
    let (url, text) = network.playlist(&playlist.url, cancel).await?;
    let Parsed::Media(current) = parser::parse(&url, &text)? else {
        anyhow::bail!("HLS playlist changed. Preview the video again.");
    };
    ensure!(
        current.url == playlist.url
            && current.parts == playlist.parts
            && current.duration_ms == playlist.duration_ms
            && current.sequence == playlist.sequence
            && (current.format == playlist.format
                || (playlist.format == Format::PackedAac
                    && current.format == Format::TransportStream)),
        "HLS playlist changed. Preview the video again before downloading."
    );
    Ok(())
}

/// Catch HTML/login responses and unsupported packed audio before publishing a
/// video extension. This is container validation, not a codec/security verdict.
async fn validate_media(
    path: &Path,
    format: Format,
    init: bool,
    first: bool,
    cancel: &CancellationToken,
) -> Result<()> {
    let mut file = download_file::read_part(path).await?;
    let size = file.metadata().await?.len();
    match format {
        Format::Progressive => {
            // Later parts are arbitrary slices of the file; the first must open an MP4.
            if first {
                let mut header = [0; 8];
                ensure!(
                    size >= 8
                        && file.read_exact(&mut header).await.is_ok()
                        && &header[4..8] == b"ftyp",
                    "The video server did not return an MP4 file"
                );
            }
        }
        Format::PackedAac => {
            super::aac::validate(path, cancel).await?;
        }
        Format::TransportStream => {
            ensure!(
                size >= 188 && size % 188 == 0,
                "HLS segment is not MPEG-TS. Packed audio and other containers are not supported."
            );
            let mut buffer = vec![0; 188 * 256];
            loop {
                ensure!(!cancel.is_cancelled(), "HLS download cancelled");
                let count =
                    (size - file.stream_position().await?).min(buffer.len() as u64) as usize;
                if count == 0 {
                    break;
                }
                file.read_exact(&mut buffer[..count]).await?;
                ensure!(
                    buffer[..count].chunks(188).all(|packet| packet[0] == 0x47),
                    "Invalid MPEG-TS segment"
                );
            }
        }
        Format::FragmentedMp4 => {
            if init {
                ensure!(
                    size <= 8 * 1024 * 1024,
                    "MP4 initialization segment exceeds 8 MiB"
                );
                let mut bytes = Vec::with_capacity(size as usize);
                file.read_to_end(&mut bytes).await?;
                super::mp4::clear_initialization(&bytes)?;
                file.rewind().await?;
            }
            let mut offset = 0;
            let mut types = Vec::new();
            while offset < size {
                ensure!(!cancel.is_cancelled(), "HLS download cancelled");
                ensure!(
                    size - offset >= 8 && types.len() < 10_000,
                    "Invalid fragmented MP4 segment"
                );
                let mut header = [0; 8];
                file.read_exact(&mut header).await?;
                let length = u32::from_be_bytes(header[..4].try_into().unwrap()) as u64;
                let (length, header_length) = if length == 1 {
                    (file.read_u64().await?, 16)
                } else if length == 0 {
                    (size - offset, 8)
                } else {
                    (length, 8)
                };
                ensure!(
                    length >= header_length && length <= size - offset,
                    "Invalid MP4 box length"
                );
                types.push(header[4..8].to_vec());
                offset += length;
                file.seek(std::io::SeekFrom::Start(offset)).await?;
            }
            let has = |name: &[u8]| types.iter().any(|kind| kind == name);
            ensure!(
                if init {
                    has(b"ftyp") && has(b"moov")
                } else {
                    has(b"moof") && has(b"mdat") && !has(b"moov")
                },
                "HLS did not return the expected fragmented MP4 container"
            );
        }
    }
    Ok(())
}
