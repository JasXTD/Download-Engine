use super::*;
use crate::{
    download_file, engine, hls::merge::Helper, model::Job, store::Store,
    synchronization::FinishedFile, transfer_activity::Phase,
};
use anyhow::Context;
use sha2::{Digest, Sha256};
use std::{fs::File, path::Path, process::Command};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::sync::CancellationToken;

fn command(mut cmd: Command, root: &Path, plan: &Plan, max_output: u64) -> Command {
    cmd.current_dir(root).env_remove("FFREPORT").args([
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
        "-start_at_zero",
    ]);
    for (index, track) in plan.tracks.iter().enumerate() {
        cmd.args([
            "-protocol_whitelist",
            "file",
            "-f",
            match track.format {
                Format::Mp4 => "mov",
                Format::Mpegts => "mpegts",
                Format::Aac => "aac",
                Format::Srt => "srt",
                Format::Webvtt => "webvtt",
                Format::Ass => "ass",
            },
        ]);
        if track.format == Format::Mp4 {
            cmd.args(["-enable_drefs", "0", "-use_absolute_path", "0"]);
        }
        cmd.arg("-i")
            .arg(root.join(format!("provider-track-{index}.part")));
    }
    for (index, track) in plan.tracks.iter().enumerate() {
        let kind = match track.kind {
            TrackKind::Video => "v",
            TrackKind::Audio => "a",
            TrackKind::Subtitle => "s",
        };
        cmd.arg("-map").arg(format!("{index}:{kind}:0"));
    }
    cmd.args(["-c", "copy", "-map_metadata", "-1", "-map_chapters", "-1"]);
    if plan.container == Container::Mp4 {
        cmd.args(["-c:s", "mov_text"]);
    }
    for (index, track) in plan.tracks.iter().enumerate() {
        cmd.arg(format!("-metadata:s:{index}")).arg(format!(
            "language={}",
            if track.language.is_empty() {
                "und"
            } else {
                &track.language
            }
        ));
        cmd.arg(format!("-metadata:s:{index}"))
            .arg(format!("title={}", track.title));
        cmd.arg(format!("-disposition:{index}"))
            .arg(if track.default { "default" } else { "0" });
    }
    cmd.args([
        "-avoid_negative_ts",
        "make_zero",
        "-f",
        match plan.container {
            Container::Mkv => "matroska",
            Container::Mp4 => "mp4",
        },
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

pub(super) async fn finish(
    store: &Store,
    job: &mut Job,
    root: &Path,
    plan: &Plan,
    tracks: Vec<File>,
    helper: Helper,
    cancel: &CancellationToken,
) -> Result<Option<FinishedFile>> {
    ensure!(
        root.is_absolute(),
        "The media cache directory must be absolute"
    );
    let staging = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
    let _destination = download_file::guard_parents(&staging)?;
    let output = download_file::fresh_read_write(&staging)
        .await?
        .into_std()
        .await;
    ensure!(
        tracks.len() == plan.tracks.len(),
        "Missing processed media track"
    );
    let processed_size = tracks.iter().try_fold(0_u64, |sum, track| {
        sum.checked_add(track.metadata()?.len())
            .context("Processed media size overflow")
    })?;
    let allowed_size: u64 = plan
        .tracks
        .iter()
        .flat_map(|t| &t.segments)
        .map(|s| s.output_limit())
        .sum();
    ensure!(
        processed_size > 0 && processed_size <= allowed_size,
        "Processed media exceeds its size limit"
    );
    let max_output = processed_size + processed_size / 2 + 8 * 1024 * 1024;
    let mut cmd = command(helper.command(), root, plan, max_output);
    let token = cancel.clone();
    store.activity.set(&job.id, Phase::Merging);
    let result = tokio::task::spawn_blocking(move || {
        // Keep all verified inputs guarded until the child has exited.
        let _guards = (helper, tracks);
        crate::hls::merge::run_media(&mut cmd, &output, &token, max_output)?;
        Ok::<_, anyhow::Error>(output)
    })
    .await?;
    let file = match result {
        Ok(file) => file,
        Err(error) => {
            let _ = tokio::fs::remove_file(&staging).await;
            return Err(error);
        }
    };
    let mut output = tokio::fs::File::from_std(file);
    output.sync_all().await?;
    if cancel.is_cancelled() {
        return Ok(None);
    }
    let size = output.metadata().await?.len();
    ensure!(
        (24..=max_output).contains(&size),
        "The merged media is empty or exceeds its size limit"
    );
    output.rewind().await?;
    let mut header = [0; 8];
    output.read_exact(&mut header).await?;
    ensure!(
        match plan.container {
            Container::Mkv => header[..4] == [0x1a, 0x45, 0xdf, 0xa3],
            Container::Mp4 => &header[4..] == b"ftyp",
        },
        "The merger produced the wrong container"
    );
    output.rewind().await?;
    store.activity.set(&job.id, Phase::Verifying);
    let mut digest = Sha256::new();
    let mut buffer = vec![0; crate::provider::CHUNK_BYTES];
    loop {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let n = output.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    job.integrity.actual = Some(format!("{:x}", digest.finalize()));
    job.integrity.check()?;
    job.total = Some(size);
    job.downloaded = size;
    if cancel.is_cancelled() {
        return Ok(None);
    }
    let path = engine::publish_without_overwrite(store, job, &staging).await?;
    Ok(Some(FinishedFile {
        path,
        guard: Some(output.into_std().await),
    }))
}
