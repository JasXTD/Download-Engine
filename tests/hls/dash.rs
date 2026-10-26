use super::merge::{
    assert_tracks_decode, execute, ffmpeg, original_track, packet_hashes, packet_times,
};
use super::*;
use std::process::Command;

async fn source() -> (tempfile::TempDir, Server, hls::Inspection) {
    let files = tempfile::tempdir().unwrap();
    execute(Command::new(ffmpeg()).current_dir(files.path()).args([
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=160x90:rate=25:duration=2.5",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=660:sample_rate=48000:duration=2.5",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=880:sample_rate=48000:duration=2.5",
        "-map",
        "0:v",
        "-map",
        "0:v",
        "-map",
        "1:a",
        "-map",
        "2:a",
        "-c:v",
        "libx264",
        "-threads",
        "1",
        "-g",
        "25",
        "-bf",
        "2",
        "-b:v:0",
        "300k",
        "-b:v:1",
        "150k",
        "-s:v:1",
        "80x44",
        "-c:a",
        "aac",
        "-b:a",
        "96k",
        "-metadata:s:a:0",
        "language=eng",
        "-metadata:s:a:1",
        "language=fra",
        "-f",
        "dash",
        "-seg_duration",
        "1",
        "-use_template",
        "1",
        "-use_timeline",
        "1",
        "-adaptation_sets",
        "id=0,streams=v id=1,streams=2 id=2,streams=3",
        "main.mpd",
    ]));
    let server = Server::new().await;
    for file in std::fs::read_dir(files.path()).unwrap() {
        let file = file.unwrap();
        server.set(
            &format!("/{}", file.file_name().to_string_lossy()),
            Reply::bytes(std::fs::read(file.path()).unwrap()),
        );
    }
    let inspection = hls::inspect(&server.url("/main.mpd"), &CancellationToken::new())
        .await
        .unwrap();
    (files, server, inspection)
}
#[tokio::test]
async fn dash_selected_quality_audio_download_merge_decode_and_timing() {
    let (files, server, inspection) = source().await;
    let (_home, store) = fixture();
    assert_eq!(inspection.variants.len(), 2);
    assert_eq!(inspection.variants[0].audio.len(), 2);
    let preview = hls::preview_with_audio(&inspection, 1, 1, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        preview.audio.as_ref().unwrap().language.as_deref(),
        Some("fra")
    );
    assert!(preview.quality.contains("80 × 44"));
    assert_eq!(
        server.requests.lock().unwrap().len(),
        1,
        "Preview fetched media"
    );
    assert!(store.list().unwrap().is_empty());
    let video = original_track(files.path(), &preview, false);
    let audio = original_track(files.path(), &preview, true);
    let saved = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(saved.status, Status::Complete, "{:?}", saved.error);
    let output = saved.saved_path.unwrap();
    assert_tracks_decode(&output);
    assert_eq!(
        packet_hashes(&output, "0:v:0", false),
        packet_hashes(&video, "0:v:0", false)
    );
    let original_audio = packet_hashes(&audio, "0:a:0", false);
    let actual_audio = packet_hashes(&output, "0:a:0", false);
    assert!(actual_audio.len() >= 110);
    assert_eq!(actual_audio, original_audio[..actual_audio.len()]);
    let before_video = packet_times(&video, "0:v:0");
    let after_video = packet_times(&output, "0:v:0");
    let before_audio = packet_times(&audio, "0:a:0");
    let after_audio = packet_times(&output, "0:a:0");
    let shift = after_video[0].0 - before_video[0].0;
    for (before, after) in before_video
        .iter()
        .zip(&after_video)
        .chain(before_audio.iter().zip(&after_audio))
    {
        assert!(
            (after.0 - before.0 - shift).abs() < 0.002
                && (after.1 - before.1 - shift).abs() < 0.002,
            "DASH A/V timing changed"
        );
    }
    let requests = server.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .all(|r| !r.to_lowercase().contains("authorization:")
                && !r.to_lowercase().contains("cookie:"))
    );
    assert!(requests.iter().filter_map(|r| r.lines().next()).all(|r| {
        r.contains("main.mpd")
            || preview
                .parts
                .iter()
                .chain(&preview.audio.as_ref().unwrap().parts)
                .any(|p| r.contains(reqwest::Url::parse(&p.url).unwrap().path()))
    }));
}
#[tokio::test]
async fn dash_manifest_changes_and_new_encryption_fail_before_media_requests() {
    let (files, server, inspection) = source().await;
    let (_home, store) = fixture();
    let preview = hls::preview(&inspection, 0, &CancellationToken::new())
        .await
        .unwrap();
    let manifest = std::fs::read_to_string(files.path().join("main.mpd")).unwrap();
    for altered in [
        manifest.replace("chunk-stream", "different-stream"),
        manifest.replace(
            "<Period",
            "<ContentProtection schemeIdUri=\"urn:mpeg:dash:mp4protection:2011\"/><Period",
        ),
    ] {
        server.set("/main.mpd", Reply::bytes(altered));
        let job = run(&store, enqueue(&store, &preview)).await;
        assert_eq!(job.status, Status::Failed);
        assert!(job.saved_path.is_none());
        assert!(job.hls.unwrap().completed.is_empty());
    }
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.starts_with("GET /main.mpd "))
    );
}
#[tokio::test]
async fn dash_native_preview_receipt_is_not_a_queued_download() {
    let (_home, store) = fixture();
    let request = serde_json::json!({"action":"preview_video","request_id":format!("hls:{}",uuid::Uuid::new_v4()),"url":"https://example.test/main.mpd"});
    let reply = download_engine::native::handle(&store, &serde_json::to_vec(&request).unwrap());
    assert!(reply.ok);
    assert_eq!(reply.preview_id.as_deref(), request["request_id"].as_str());
    assert!(store.list().unwrap().is_empty());
    assert_eq!(
        store.next_hls_preview().unwrap().unwrap().1,
        "https://example.test/main.mpd"
    );
    let bad = serde_json::json!({"action":"add","request_id":"bad","url":"https://example.test/main.mpd"});
    assert!(!download_engine::native::handle(&store, &serde_json::to_vec(&bad).unwrap()).ok);
}

#[tokio::test]
async fn dash_pause_reopen_and_resume_keeps_verified_video_and_audio_segments() {
    let (_files, server, inspection) = source().await;
    let (_home, store) = fixture();
    let preview = hls::preview(&inspection, 0, &CancellationToken::new())
        .await
        .unwrap();
    let audio = &preview.audio.as_ref().unwrap().parts;
    let paused = pause_at_part(&server, &store, enqueue(&store, &preview), &audio[1]).await;
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
    let resumed = run(&store, store.get(&id).unwrap().unwrap()).await;
    assert_eq!(resumed.status, Status::Complete, "{:?}", resumed.error);
    assert_tracks_decode(&resumed.saved_path.unwrap());
    for part in preview.parts.iter().chain(audio) {
        assert_eq!(
            server.count(reqwest::Url::parse(&part.url).unwrap().path()),
            if part == &audio[1] { 2 } else { 1 }
        );
    }
}

#[tokio::test]
async fn dash_segment_list_byte_ranges_merge_real_media_and_do_not_cross_local_origins() {
    let (files, server, inspection) = source().await;
    let selected = hls::preview(&inspection, 0, &CancellationToken::new())
        .await
        .unwrap();
    let mut sets = String::new();
    for (kind, parts) in [
        ("video", &selected.parts),
        ("audio", &selected.audio.as_ref().unwrap().parts),
    ] {
        let mut data = Vec::new();
        let mut list = String::new();
        for part in parts {
            let name = reqwest::Url::parse(&part.url)
                .unwrap()
                .path()
                .trim_start_matches('/')
                .to_owned();
            let bytes = std::fs::read(files.path().join(name)).unwrap();
            let range = format!("{}-{}", data.len(), data.len() + bytes.len() - 1);
            data.extend(bytes);
            if part.initialization {
                list += &format!(r#"<Initialization range="{range}"/>"#);
            } else {
                list += &format!(r#"<SegmentURL mediaRange="{range}"/>"#);
            }
        }
        server.set(&format!("/{kind}.mp4"), Reply::bytes(data));
        sets += &format!(
            r#"<AdaptationSet mimeType="{kind}/mp4"><BaseURL>{kind}.mp4</BaseURL><Representation id="{kind}" bandwidth="100000"><SegmentList>{list}</SegmentList></Representation></AdaptationSet>"#
        );
    }
    let text = format!(
        r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" mediaPresentationDuration="PT2.52S"><Period>{sets}</Period></MPD>"#
    );
    server.set("/list.mpd", Reply::bytes(&text));
    let (_home, store) = fixture();
    let inspection = hls::inspect(&server.url("/list.mpd"), &CancellationToken::new())
        .await
        .unwrap();
    let preview = hls::preview(&inspection, 0, &CancellationToken::new())
        .await
        .unwrap();
    let job = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(job.status, Status::Complete, "{:?}", job.error);
    assert_tracks_decode(&job.saved_path.unwrap());
    let other = Server::new().await;
    server.set(
        "/unsafe.mpd",
        Reply::bytes(text.replace("video.mp4", &other.url("/secret"))),
    );
    let inspection = hls::inspect(&server.url("/unsafe.mpd"), &CancellationToken::new())
        .await
        .unwrap();
    let preview = hls::preview(&inspection, 0, &CancellationToken::new())
        .await
        .unwrap();
    let job = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(job.status, Status::Failed);
    assert!(job.saved_path.is_none());
    assert_eq!(other.count("/secret"), 0);
}
