//! Remux only locally downloaded, verified tracks. Never give FFmpeg a URL,
//! playlist, user-provided option, or permission to open external MP4 references.
use super::{CompletedPart, Download, Format};
use crate::{
    child_process, download_file, engine, model::Job, store::Store, synchronization::FinishedFile,
};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

const FFMPEG_SHA256: &str = "f1f9097330cc07678bebe983e1e5854bacce5f36cd851e234cdcf074ea54d577";
const MERGE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub(super) struct Helper {
    path: PathBuf,
    _folders: Vec<File>,
    _program: File,
}
impl Helper {
    pub(super) async fn checked(cancel: &CancellationToken) -> Result<Self> {
        let cancel = cancel.clone();
        tokio::task::spawn_blocking(move || {
            #[allow(unused_mut)]
            let mut path = std::env::current_exe()?
                .parent()
                .context("Missing application folder")?
                .join("tools/ffmpeg/ffmpeg.exe");
            // Development binaries use the exact same pinned helper as the package.
            #[cfg(debug_assertions)]
            if !path.exists() {
                path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/ffmpeg/ffmpeg.exe");
            }
            Self::at(&path, &cancel)
        })
        .await?
    }
    fn at(path: &Path, cancel: &CancellationToken) -> Result<Self> {
        let folders = download_file::guard_parents(path).context(
            "The bundled media merger is missing. Repair or re-extract Download Engine.",
        )?;
        let mut program = download_file::read_completed(path).context(
            "The bundled media merger is missing. Repair or re-extract Download Engine.",
        )?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0; 256 * 1024];
        loop {
            ensure!(!cancel.is_cancelled(), "HLS merge cancelled");
            let n = program.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        ensure!(
            format!("{:x}", hash.finalize()) == FFMPEG_SHA256,
            "The bundled media merger changed. Repair or re-extract Download Engine before downloading separate tracks."
        );
        Ok(Self {
            path: path.to_owned(),
            _folders: folders,
            _program: program,
        })
    }
}

pub(super) async fn finish(
    store: &Store,
    job: &mut Job,
    root: &Path,
    cancel: &CancellationToken,
    helper: Helper,
) -> Result<Option<FinishedFile>> {
    let video = root.join("merge-video.part");
    let audio = root.join("merge-audio.part");
    let staging = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
    // Keep the destination and cache ancestors fixed until publication/cleanup.
    let _destination = download_file::guard_parents(&staging)?;
    let _cache = download_file::guard_parents(&video)?;
    let result = finish_inner(store, job, root, cancel, helper, [&video, &audio, &staging]).await;
    for path in [&video, &audio] {
        let _ = tokio::fs::remove_file(path).await;
    }
    if !matches!(result, Ok(Some(_))) {
        let _ = tokio::fs::remove_file(&staging).await;
    }
    result
}

async fn finish_inner(
    store: &Store,
    job: &mut Job,
    root: &Path,
    cancel: &CancellationToken,
    helper: Helper,
    paths: [&Path; 3],
) -> Result<Option<FinishedFile>> {
    let download = job.hls.as_ref().context("Missing HLS download")?;
    ensure!(
        download.completed.len() == download.playlist.part_count(),
        "Incomplete HLS download"
    );
    let split = download.playlist.parts.len();
    store
        .activity
        .set(&job.id, crate::transfer_activity::Phase::Assembling);
    let video_file =
        assemble_track(root, paths[0], &download.completed[..split], 0, cancel).await?;
    let audio_file = if download.playlist.audio.is_some() {
        Some(assemble_track(root, paths[1], &download.completed[split..], split, cancel).await?)
    } else {
        None
    };
    let output = download_file::fresh_read_write(paths[2])
        .await?
        .into_std()
        .await;
    // Remuxing may add MP4 tables/headers, but cannot expand unboundedly.
    let input_size: u64 = download.completed.iter().map(|part| part.size).sum();
    let max_output = input_size + input_size / 2 + 8 * 1024 * 1024;
    let aac_start = if download
        .playlist
        .audio
        .as_ref()
        .is_some_and(|a| a.format == Format::PackedAac)
    {
        // Assembly retains its write guard on Windows. Validate that same file
        // handle; reopening with read-only sharing would conflict with the guard.
        let mut track = tokio::fs::File::from_std(audio_file.as_ref().unwrap().try_clone()?);
        Some(super::aac::validate_handle(&mut track, cancel).await?)
    } else {
        None
    };
    let mut command = command(
        &helper.path,
        paths[0],
        paths[1],
        download,
        max_output,
        aac_start,
    );
    let cancel_copy = cancel.clone();
    store
        .activity
        .set(&job.id, crate::transfer_activity::Phase::Merging);
    let mut output = tokio::task::spawn_blocking(move || {
        // These handles prevent replacement/writes until the child has exited.
        let _guards = (helper, video_file, audio_file);
        run(
            &mut command,
            &output,
            &cancel_copy,
            MERGE_TIMEOUT,
            max_output,
        )?;
        Ok::<_, anyhow::Error>(tokio::fs::File::from_std(output))
    })
    .await??;
    if cancel.is_cancelled() {
        return Ok(None);
    }
    output.sync_all().await?;
    store
        .activity
        .set(&job.id, crate::transfer_activity::Phase::Verifying);
    let size = output.metadata().await?.len();
    ensure!(
        size >= 24 && size <= max_output,
        "The merged MP4 is empty or exceeds its size limit"
    );
    output.rewind().await?;
    let mut header = [0; 8];
    output.read_exact(&mut header).await?;
    ensure!(
        &header[4..] == b"ftyp",
        "The merger did not produce an MP4 file"
    );
    output.rewind().await?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let n = output.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    job.integrity.actual = Some(format!("{:x}", hash.finalize()));
    job.integrity.check()?;
    job.downloaded = size;
    job.total = Some(size);
    if cancel.is_cancelled() {
        return Ok(None);
    }
    let path = engine::publish_without_overwrite(store, job, paths[2]).await?;
    // Retain the verified handle through the durable completion transaction.
    Ok(Some(FinishedFile {
        path,
        guard: Some(output.into_std().await),
    }))
}

async fn assemble_track(
    root: &Path,
    path: &Path,
    parts: &[CompletedPart],
    start: usize,
    cancel: &CancellationToken,
) -> Result<File> {
    let mut output = download_file::fresh_read_write(path).await?;
    let mut buffer = vec![0; 256 * 1024];
    for (offset, part) in parts.iter().enumerate() {
        let mut input =
            download_file::read_part(&root.join(format!("hls-{}.part", start + offset))).await?;
        let mut hash = Sha256::new();
        let mut size = 0;
        loop {
            ensure!(!cancel.is_cancelled(), "HLS merge cancelled");
            let n = input.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            size += n as u64;
            ensure!(
                size <= part.size,
                "Saved HLS segment changed before merging. Restart this download."
            );
            hash.update(&buffer[..n]);
            output.write_all(&buffer[..n]).await?;
        }
        ensure!(
            size == part.size && format!("{:x}", hash.finalize()) == part.sha256,
            "Saved HLS segment changed before merging. Restart this download."
        );
    }
    output.sync_all().await?;
    Ok(output.into_std().await)
}

fn command(
    executable: &Path,
    video: &Path,
    audio: &Path,
    download: &Download,
    max_output: u64,
    aac_start: Option<u64>,
) -> Command {
    let mut cmd = Command::new(executable);
    cmd.current_dir(video.parent().unwrap())
        .env_remove("FFREPORT")
        .args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-xerror",
            "-abort_on",
            "empty_output+empty_output_stream",
            "-max_alloc",
            "268435456",
            "-copyts",
        ]);
    if download.playlist.dash.is_none() && aac_start.is_none() {
        cmd.arg("-start_at_zero");
    }
    for (index, (path, format)) in std::iter::once((video, download.playlist.format))
        .chain(
            download
                .playlist
                .audio
                .iter()
                .map(|track| (audio, track.format)),
        )
        .enumerate()
    {
        if let Some(dash) = &download.playlist.dash {
            // DASH maps each track's sample clock to the period with its own PTO.
            // Normalizing each input separately would erase deliberate A/V offsets.
            let offset = if index == 0 {
                dash.video_offset_us
            } else {
                dash.audio_offset_us
            };
            cmd.arg("-itsoffset")
                .arg(format!("-{}.{:06}", offset / 1_000_000, offset % 1_000_000));
        } else if index == 1 {
            if let Some(ticks) = aac_start {
                cmd.arg("-itsoffset").arg(format!(
                    "{}.{:06}",
                    ticks / 90_000,
                    (ticks % 90_000) * 1_000_000 / 90_000
                ));
            } else {
                cmd.args(["-isync", "0"]);
            }
        }
        cmd.args([
            "-protocol_whitelist",
            "file",
            "-f",
            match format {
                Format::TransportStream => "mpegts",
                Format::FragmentedMp4 | Format::Progressive => "mov",
                Format::PackedAac => "aac",
            },
        ]);
        if format == Format::FragmentedMp4 {
            cmd.args(["-enable_drefs", "0", "-use_absolute_path", "0"]);
        }
        cmd.arg("-i").arg(path);
    }
    // Exact mappings fail if a selected track is missing. Use the inherited,
    // seekable stdout descriptor for ordinary MP4: this keeps timestamp edit
    // lists and automatic AAC conversion, without reopening a writable path.
    cmd.args(["-map", "0:v:0"]);
    if download.playlist.audio.is_some() {
        cmd.args(["-map", "1:a:0"]);
    }
    if download.playlist.dash.is_some() {
        cmd.arg("-t").arg(format!(
            "{}.{:03}",
            download.playlist.duration_ms / 1000,
            download.playlist.duration_ms % 1000
        ));
    }
    cmd.args([
        "-c",
        "copy",
        "-map_metadata",
        "-1",
        "-map_chapters",
        "-1",
        "-avoid_negative_ts",
        "make_zero",
        "-f",
        "mp4",
        "-protocol_whitelist",
        "fd",
        "-fd",
        "1",
        "-fs",
    ])
    .arg((max_output + 1).to_string())
    .arg("fd:");
    cmd
}

fn run(
    command: &mut Command,
    output: &File,
    cancel: &CancellationToken,
    timeout: Duration,
    max_output: u64,
) -> Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(Stdio::piped());
    let mut running = child_process::Running::spawn(command, cancel, Some(1024 * 1024 * 1024))?;
    let stderr = child_process::reader(
        running
            .child
            .stderr
            .take()
            .context("Missing merger error output")?,
    );
    let started = Instant::now();
    let status = loop {
        ensure!(!cancel.is_cancelled(), "HLS merge cancelled");
        ensure!(
            started.elapsed() < timeout,
            "HLS merge exceeded 30 minutes. Completed segments were kept; resume to retry."
        );
        ensure!(
            output.metadata()?.len() <= max_output,
            "The merged MP4 exceeds its size limit"
        );
        if let Some(status) = running.child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    drop(running);
    let (message, _) = stderr
        .recv_timeout(Duration::from_secs(2))
        .context("Merger output did not close")??;
    ensure!(
        status.success(),
        "Could not merge the selected audio and video. The streams may be damaged or incompatible with MP4. {}",
        message
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(2000)
            .collect::<String>()
    );
    ensure!(
        output.metadata()?.len() <= max_output,
        "The merged MP4 exceeds its size limit"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
