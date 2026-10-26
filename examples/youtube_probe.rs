//! Explicit live verification; never part of the offline test suite.
//! cargo run --offline --locked --example youtube_probe -- URL [--download]
use anyhow::{Context, Result, ensure};
use download_engine::{
    bandwidth::{Limiter, Limits},
    engine,
    paths::AppPaths,
    queue::MAIN_QUEUE,
    store::Store,
    youtube,
};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let url = args.get(1).context("Provide a public YouTube video URL")?;
    let cancel = CancellationToken::new();
    if let Some(index) = args.iter().position(|a| a == "--resume-store") {
        let root =
            std::path::PathBuf::from(args.get(index + 1).context("Missing isolated store path")?);
        let store = Store::open(AppPaths {
            data: root.join("data"),
            downloads: root.join("downloads"),
        })?;
        let jobs = store.list()?;
        ensure!(
            jobs.len() == 1,
            "Resume probe requires exactly one test job"
        );
        let job = jobs[0].clone();
        ensure!(
            job.url == youtube::watch_url(&youtube::video_id(url)?)?
                && job.hls.as_ref().is_some_and(|h| h.youtube.is_some()),
            "Unexpected test job"
        );
        let id = job.id.clone();
        engine::run(
            store.clone(),
            job,
            cancel,
            Limiter::new(&Limits::default())?,
        )
        .await?;
        let saved = store.get(&id)?.context("Missing resumed test job")?;
        ensure!(
            saved.status == download_engine::model::Status::Complete,
            "Live resume failed: {:?}",
            saved.error
        );
        println!(
            "download complete: {} bytes; {}",
            saved.downloaded,
            saved
                .saved_path
                .as_deref()
                .context("Missing output")?
                .display()
        );
        return Ok(());
    }
    let video = youtube::inspect(url, &cancel).await?;
    println!("title: {}", video.title);
    for (i, variant) in video.inspection.variants.iter().enumerate() {
        println!(
            "quality {i}: {} ({} audio choices)",
            variant.label(),
            variant.audio.len()
        );
    }
    let index = video
        .inspection
        .variants
        .iter()
        .enumerate()
        .rev()
        .find(|(_, v)| v.unsupported.is_none() && v.codecs.contains("avc1") && !v.audio.is_empty())
        .map(|(i, _)| i)
        .context("No supported AVC quality with audio")?;
    let audio = video.inspection.variants[index].default_audio();
    let preview = youtube::preview(&video.inspection, index, audio, &cancel).await?;
    println!(
        "preview: {} ms, {} parts, output {}",
        preview.playlist.duration_ms,
        preview.playlist.part_count(),
        preview.playlist.output_extension()
    );
    ensure!(preview.playlist.audio.is_some(), "Expected separate audio");
    if args
        .iter()
        .any(|arg| arg == "--download" || arg == "--pause-resume")
    {
        let root = std::env::current_dir()?
            .join("artifacts/youtube-rust-port-20261014")
            .join(format!("live-{}", uuid::Uuid::new_v4()));
        let store = Store::open(AppPaths {
            data: root.join("data"),
            downloads: root.join("downloads"),
        })?;
        store.set_queue_running(MAIN_QUEUE, false)?;
        let name = format!(
            "{}.{}",
            download_engine::model::safe_filename(&video.title),
            preview.playlist.output_extension()
        );
        let job = store.enqueue_youtube(
            &preview,
            &name,
            None,
            MAIN_QUEUE,
            &uuid::Uuid::new_v4().to_string(),
        )?;
        let id = job.id.clone();
        let job = if args.iter().any(|arg| arg == "--pause-resume") {
            let pause = CancellationToken::new();
            let worker = engine::run(
                store.clone(),
                job,
                pause.clone(),
                Limiter::new(&Limits::default())?,
            );
            tokio::pin!(worker);
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
            let deadline = tokio::time::sleep(std::time::Duration::from_secs(120));
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    result = &mut worker => { result?; anyhow::bail!("Download ended before the pause check"); }
                    _ = &mut deadline => { pause.cancel(); worker.await?; anyhow::bail!("Pause check timed out"); }
                    _ = tick.tick() => {
                        if store.get(&id)?.and_then(|j| j.hls).is_some_and(|h| h.completed.len() >= 2) {
                            pause.cancel(); worker.await?; break;
                        }
                    }
                }
            }
            let mut paused = store.get(&id)?.context("Missing paused job")?;
            ensure!(
                paused.status == download_engine::model::Status::Paused,
                "Expected paused status"
            );
            let saved_parts = paused.hls.as_ref().unwrap().completed.len();
            // Force expired links in the saved state. Resumption must re-extract;
            // requesting any of these deliberately expired links would fail.
            let expire = |value: &mut String| {
                let mut url = reqwest::Url::parse(value).unwrap();
                let mut parts = url.path().split('/').map(str::to_owned).collect::<Vec<_>>();
                if let Some(i) = parts.iter().position(|p| p == "expire")
                    && i + 1 < parts.len()
                {
                    parts[i + 1] = "1".into();
                }
                url.set_path(&parts.join("/"));
                *value = url.to_string();
            };
            let hls = paused.hls.as_mut().unwrap();
            expire(&mut hls.playlist.url);
            for part in &mut hls.playlist.parts {
                expire(&mut part.url);
            }
            if let Some(audio) = &mut hls.playlist.audio {
                expire(&mut audio.url);
                for part in &mut audio.parts {
                    expire(&mut part.url);
                }
            }
            store.save(&paused)?;
            println!("paused: {saved_parts} verified parts; forced saved links to expire");
            paused
        } else {
            job
        };
        drop(store);
        let store = Store::open(AppPaths {
            data: root.join("data"),
            downloads: root.join("downloads"),
        })?;
        ensure!(
            store
                .get(&id)?
                .context("Missing reopened job")?
                .hls
                .as_ref()
                .unwrap()
                .completed
                .len()
                == job.hls.as_ref().unwrap().completed.len(),
            "Partial progress did not survive reopening"
        );
        engine::run(
            store.clone(),
            job,
            cancel,
            Limiter::new(&Limits::default())?,
        )
        .await?;
        let saved = store.get(&id)?.context("Missing live test download")?;
        ensure!(
            saved.status == download_engine::model::Status::Complete,
            "Live download failed: {:?}",
            saved.error
        );
        println!(
            "download complete: {} bytes; {}",
            saved.downloaded,
            saved
                .saved_path
                .as_deref()
                .context("Missing output")?
                .display()
        );
    }
    Ok(())
}
