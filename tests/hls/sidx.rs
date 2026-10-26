use super::merge::{assert_tracks_decode, execute, ffmpeg, packet_hashes, packet_times};
use super::*;
use std::{collections::BTreeMap, process::Command};

struct Fixture {
    files: tempfile::TempDir,
    server: Server,
    inspection: hls::Inspection,
    indexes: BTreeMap<String, hls::Range>,
}

#[tokio::test]
async fn reader_security_dash_indexes_cannot_request_a_local_root() {
    let mut fixture = source().await;
    fixture.inspection.public_only = true;
    fixture.server.requests.lock().unwrap().clear();
    let error = hls::preview_with_audio(&fixture.inspection, 1, 1, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("public internet"), "{error:#}");
    assert!(fixture.server.requests.lock().unwrap().is_empty());
}
async fn source() -> Fixture {
    let files = tempfile::tempdir().unwrap();
    let server = Server::new().await;
    let mut sets = String::new();
    let mut indexes = BTreeMap::new();
    for (name, audio, language, bandwidth, input) in [
        (
            "high",
            false,
            "",
            300000,
            "testsrc2=size=160x90:rate=25:duration=2.5",
        ),
        (
            "low",
            false,
            "",
            150000,
            "testsrc2=size=80x44:rate=25:duration=2.5",
        ),
        (
            "english",
            true,
            "en",
            96000,
            "sine=frequency=660:sample_rate=48000:duration=2.5",
        ),
        (
            "french",
            true,
            "fr",
            96000,
            "sine=frequency=880:sample_rate=48000:duration=2.5",
        ),
    ] {
        let mut command = Command::new(ffmpeg());
        command.current_dir(files.path()).args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            input,
        ]);
        if audio {
            command.args(["-c:a", "aac", "-b:a", "96k"]);
        } else {
            command.args(["-c:v", "libx264", "-threads", "1", "-g", "25", "-bf", "2"]);
        }
        command
            .args(["-movflags", "+dash+global_sidx"])
            .arg(format!("{name}.mp4"));
        execute(&mut command);
        let data = std::fs::read(files.path().join(format!("{name}.mp4"))).unwrap();
        let mut offset = 0;
        let (index, scale) = loop {
            let size = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
            assert!(size >= 8 && offset + size <= data.len());
            if &data[offset + 4..offset + 8] == b"sidx" {
                break (
                    hls::Range {
                        start: offset as u64,
                        length: size as u64,
                    },
                    u32::from_be_bytes(data[offset + 16..offset + 20].try_into().unwrap()),
                );
            }
            offset += size;
        };
        let kind = if audio { "audio" } else { "video" };
        let dimensions = if name == "high" {
            " width=\"160\" height=\"90\""
        } else if name == "low" {
            " width=\"80\" height=\"44\""
        } else {
            ""
        };
        sets += &format!(
            r#"<AdaptationSet mimeType="{kind}/mp4" lang="{language}"><Representation id="{name}" bandwidth="{bandwidth}"{dimensions}><BaseURL>{name}.mp4</BaseURL><SegmentBase timescale="{scale}" indexRange="{}-{}" indexRangeExact="true"><Initialization range="0-{}"/></SegmentBase></Representation></AdaptationSet>"#,
            index.start,
            index.start + index.length - 1,
            index.start - 1
        );
        let mut reply = Reply::bytes(data);
        reply.headers = format!("ETag: \"{name}-v1\"\r\n");
        server.set(&format!("/{name}.mp4"), reply);
        indexes.insert(name.into(), index);
    }
    server.set("/base.mpd",Reply::bytes(format!(r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT3S"><Period>{sets}</Period></MPD>"#)));
    let inspection = hls::inspect(&server.url("/base.mpd"), &CancellationToken::new())
        .await
        .unwrap();
    Fixture {
        files,
        server,
        inspection,
        indexes,
    }
}
async fn selected(f: &Fixture) -> hls::Playlist {
    hls::preview_with_audio(&f.inspection, 1, 1, &CancellationToken::new())
        .await
        .unwrap()
}
fn range_count(server: &Server, part: &hls::Part) -> usize {
    let range = part.range.as_ref().unwrap();
    let target = format!(
        "GET {} HTTP/",
        reqwest::Url::parse(&part.url).unwrap().path()
    );
    let header = format!(
        "range: bytes={}-{}",
        range.start,
        range.start + range.length - 1
    );
    server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.starts_with(&target) && r.to_ascii_lowercase().contains(&header))
        .count()
}
#[tokio::test]
async fn selected_indexes_preview_before_queue_and_real_media_merge_preserves_packets_and_timing() {
    let f = source().await;
    assert_eq!(
        f.server.requests.lock().unwrap().len(),
        1,
        "Loading qualities fetched indexes"
    );
    let preview = selected(&f).await;
    let (_home, store) = fixture();
    assert!(store.list().unwrap().is_empty());
    assert!(preview.quality.contains("80 × 44"));
    assert_eq!(
        preview.audio.as_ref().unwrap().language.as_deref(),
        Some("fr")
    );
    assert_eq!(f.server.requests.lock().unwrap().len(), 3);
    for name in ["low", "french"] {
        let index = &f.indexes[name];
        assert_eq!(
            range_count(
                &f.server,
                &hls::Part {
                    url: f.server.url(&format!("/{name}.mp4")),
                    range: Some(index.clone()),
                    initialization: false
                }
            ),
            1
        );
    }
    let job = enqueue(&store, &preview);
    assert_eq!(job.hls.as_ref().unwrap().version, 4);
    let mut invalid = job.hls.clone().unwrap();
    invalid.version = 3;
    assert!(invalid.validate().is_err());
    let completed = run(&store, job).await;
    assert_eq!(completed.status, Status::Complete, "{:?}", completed.error);
    let output = completed.saved_path.unwrap();
    assert_tracks_decode(&output);
    let mut shift = None;
    for (name, selector) in [("low", "0:v:0"), ("french", "0:a:0")] {
        let original = f.files.path().join(format!("{name}.mp4"));
        assert_eq!(
            packet_hashes(&output, selector, false),
            packet_hashes(&original, selector, false)
        );
        let before = packet_times(&original, selector);
        let after = packet_times(&output, selector);
        assert_eq!(before.len(), after.len());
        let shift = *shift.get_or_insert(after[0].0 - before[0].0);
        for (before, after) in before.iter().zip(after) {
            assert!(
                (after.0 - before.0 - shift).abs() < 0.002
                    && (after.1 - before.1 - shift).abs() < 0.002
            );
        }
    }
    assert_eq!(f.server.count("/high.mp4"), 0);
    assert_eq!(f.server.count("/english.mp4"), 0);
    assert!(
        f.server
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| !r.to_lowercase().contains("cookie:")
                && !r.to_lowercase().contains("authorization:"))
    );
}
#[tokio::test]
async fn resume_refetches_indexes_and_reuses_only_verified_completed_ranges() {
    let f = source().await;
    let preview = selected(&f).await;
    let (_home, store) = fixture();
    let audio = &preview.audio.as_ref().unwrap().parts;
    let paused = pause_at_part(&f.server, &store, enqueue(&store, &preview), &audio[1]).await;
    let id = paused.id.clone();
    assert_eq!(paused.status, Status::Paused);
    assert_eq!(
        paused.hls.as_ref().unwrap().completed.len(),
        preview.parts.len() + 1
    );
    assert!(paused.hls.as_ref().unwrap().completed.len() < preview.part_count());
    assert!(paused.saved_path.is_none());
    let paths = store.paths.clone();
    drop(store);
    let store = Store::open(paths).unwrap();
    let saved = run(&store, store.get(&id).unwrap().unwrap()).await;
    assert_eq!(saved.status, Status::Complete, "{:?}", saved.error);
    assert_tracks_decode(&saved.saved_path.unwrap());
    for part in preview.parts.iter().chain(audio) {
        assert_eq!(
            range_count(&f.server, part),
            if part == &audio[1] { 2 } else { 1 }
        );
    }
}
#[tokio::test]
async fn index_changes_and_resource_validator_changes_cannot_publish() {
    let f = source().await;
    let preview = selected(&f).await;
    let (_home, store) = fixture();
    let original = f.server.routes.lock().unwrap()["/low.mp4"].clone();
    let mut changed = original.clone();
    let index = &f.indexes["low"];
    // Alter a SAP hint without changing byte ranges: the complete index is bound to review.
    changed.body[(index.start + index.length - 1) as usize] ^= 1;
    f.server.set("/low.mp4", changed);
    let job = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(job.status, Status::Failed);
    assert!(job.error.unwrap().contains("index changed"));
    assert!(job.hls.unwrap().completed.is_empty());
    let mut changed = original.clone();
    changed.headers = "ETag: \"new-version\"\r\n".into();
    f.server.set("/low.mp4", changed);
    let job = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(job.status, Status::Failed);
    assert!(job.saved_path.is_none());
    let mut changed = original;
    changed.headers_for_start = Some((0, "ETag: \"changed-after-index\"\r\n".into()));
    f.server.set("/low.mp4", changed);
    let job = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(job.status, Status::Failed);
    assert!(job.error.unwrap().contains("resource changed"));
    assert!(job.hls.unwrap().completed.is_empty());
    assert!(job.saved_path.is_none());
}
#[tokio::test]
async fn ignored_ranges_truncated_and_nested_indexes_fail_without_queueing() {
    let f = source().await;
    let original = f.server.routes.lock().unwrap()["/low.mp4"].clone();
    for mode in 0..3 {
        let mut invalid = original.clone();
        if mode == 0 {
            invalid.ignore_range = true;
        }
        if mode == 1 {
            invalid.cut = true;
        }
        if mode == 2 {
            invalid.body[f.indexes["low"].start as usize + 40] |= 0x80;
        }
        f.server.set("/low.mp4", invalid);
        assert!(
            hls::preview_with_audio(&f.inspection, 1, 1, &CancellationToken::new())
                .await
                .is_err()
        );
    }
    assert_eq!(f.server.count("/french.mp4"), 0);
}
#[tokio::test]
async fn cancellation_and_cross_origin_local_indexes_fail_before_media() {
    let f = source().await;
    let cancel = f.server.pause_on(&hls::Part {
        url: f.server.url("/low.mp4"),
        range: Some(f.indexes["low"].clone()),
        initialization: false,
    });
    let preview = hls::preview_with_audio(&f.inspection, 1, 1, &cancel);
    tokio::pin!(preview);
    let result = tokio::select! {
        result = &mut preview => result,
        _ = cancel.cancelled() => {
            tokio::time::timeout(Duration::from_secs(1), preview)
                .await
                .expect("Index preview did not stop after cancellation")
        }
    };
    assert!(
        cancel.is_cancelled(),
        "Preview did not request the selected index"
    );
    assert!(result.is_err());
    assert_eq!(f.server.count("/french.mp4"), 0);
    let other = Server::new().await;
    let text = String::from_utf8(f.server.routes.lock().unwrap()["/base.mpd"].body.clone())
        .unwrap()
        .replace("low.mp4", &other.url("/secret.mp4"));
    f.server.set("/unsafe.mpd", Reply::bytes(text));
    let inspection = hls::inspect(&f.server.url("/unsafe.mpd"), &CancellationToken::new())
        .await
        .unwrap();
    assert!(
        hls::preview_with_audio(&inspection, 1, 1, &CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(other.count("/secret.mp4"), 0);
}
