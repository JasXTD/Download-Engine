use super::*;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub(super) fn ffmpeg() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/ffmpeg/ffmpeg.exe")
}
pub(super) fn execute(command: &mut Command) -> Vec<u8> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn generate_track(root: &Path, name: &str, audio: bool, mp4: bool) {
    let mut cmd = Command::new(ffmpeg());
    cmd.current_dir(root).args([
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        if audio {
            "sine=frequency=660:sample_rate=48000:duration=2.5"
        } else {
            "testsrc2=size=160x90:rate=25:duration=2.5"
        },
    ]);
    if audio {
        cmd.args(["-c:a", "aac", "-b:a", "96k", "-output_ts_offset", "0.32"]);
    } else {
        cmd.args(["-c:v", "libx264", "-threads", "1", "-g", "25", "-bf", "2"]);
    }
    cmd.args([
        "-f",
        "hls",
        "-hls_playlist_type",
        "vod",
        "-hls_time",
        "1",
        "-start_number",
        if audio { "41" } else { "7" },
        "-hls_segment_type",
        if mp4 { "fmp4" } else { "mpegts" },
    ]);
    if mp4 {
        cmd.args(["-hls_fmp4_init_filename", &format!("{name}-init.mp4")]);
    }
    cmd.arg("-hls_segment_filename")
        .arg(format!("{name}-%d.{}", if mp4 { "m4s" } else { "ts" }))
        .arg(format!("{name}.m3u8"));
    execute(&mut cmd);
}
async fn source(video_mp4: bool, audio_mp4: bool) -> (tempfile::TempDir, Server, hls::Inspection) {
    let files = tempfile::tempdir().unwrap();
    generate_track(files.path(), "video", false, video_mp4);
    generate_track(files.path(), "audio", true, audio_mp4);
    let server = Server::new().await;
    for entry in std::fs::read_dir(files.path()).unwrap() {
        let entry = entry.unwrap();
        server.set(
            &format!("/{}", entry.file_name().to_str().unwrap()),
            Reply::bytes(std::fs::read(entry.path()).unwrap()),
        );
    }
    server.set("/master.m3u8", Reply::bytes("#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"sound\",NAME=\"Other\",LANGUAGE=\"fr\",URI=\"unused-audio.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"sound\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=160x90,AUDIO=\"sound\",CODECS=\"avc1.42c01e,mp4a.40.2\"\nvideo.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=400000,RESOLUTION=80x45,AUDIO=\"sound\"\nunused-video.m3u8\n"));
    let inspection = hls::inspect(&server.url("/master.m3u8"), &CancellationToken::new())
        .await
        .unwrap();
    (files, server, inspection)
}
pub(super) fn assert_tracks_decode(path: &Path) {
    execute(
        Command::new(ffmpeg())
            .args([
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "error",
                "-xerror",
                "-i",
            ])
            .arg(path)
            .args([
                "-map", "0:v:0", "-map", "0:a:0", "-threads", "1", "-f", "null", "-",
            ]),
    );
}
pub(super) fn packet_hashes(path: &Path, selector: &str, ts_audio: bool) -> Vec<String> {
    let mut cmd = Command::new(ffmpeg());
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-map", selector, "-c", "copy"]);
    if ts_audio {
        cmd.args(["-bsf:a", "aac_adtstoasc"]);
    }
    if selector == "0:v:0" {
        cmd.args(["-bsf:v", "h264_mp4toannexb"]);
        let bytes = execute(cmd.args(["-f", "h264", "-"]));
        // Compare encoded slice NAL units, independent of legal 3/4-byte Annex B
        // prefixes and SPS/PPS/AUD container headers added during remuxing.
        let mut starts = Vec::new();
        let mut offset = 0;
        while offset + 3 <= bytes.len() {
            let length = if bytes[offset..].starts_with(&[0, 0, 0, 1]) {
                4
            } else if bytes[offset..].starts_with(&[0, 0, 1]) {
                3
            } else {
                offset += 1;
                continue;
            };
            starts.push((offset, offset + length));
            offset += length;
        }
        use sha2::{Digest, Sha256};
        return starts
            .iter()
            .enumerate()
            .filter_map(|(index, &(_, start))| {
                let mut end = starts.get(index + 1).map_or(bytes.len(), |v| v.0);
                while end > start && bytes[end - 1] == 0 {
                    end -= 1;
                }
                (end > start && (1..=5).contains(&(bytes[start] & 0x1f)))
                    .then(|| format!("{:x}", Sha256::digest(&bytes[start..end])))
            })
            .collect();
    }
    let out = execute(cmd.args(["-f", "framehash", "-hash", "sha256", "-"]));
    String::from_utf8(out)
        .unwrap()
        .lines()
        .filter(|s| !s.starts_with('#') && !s.trim().is_empty())
        // MPEG-TS may append side-data hashes after the packet hash.
        .map(|s| s.split(',').nth(5).unwrap().trim().to_owned())
        .collect()
}
pub(super) fn original_track(root: &Path, playlist: &hls::Playlist, audio: bool) -> PathBuf {
    let parts = if audio {
        &playlist.audio.as_ref().unwrap().parts
    } else {
        &playlist.parts
    };
    let format = if audio {
        playlist.audio.as_ref().unwrap().format
    } else {
        playlist.format
    };
    let result = root.join(format!("joined-{audio}.{}", format.extension()));
    let data: Vec<_> = parts
        .iter()
        .flat_map(|part| {
            let name = reqwest::Url::parse(&part.url)
                .unwrap()
                .path()
                .trim_start_matches('/')
                .to_owned();
            std::fs::read(root.join(name)).unwrap()
        })
        .collect();
    std::fs::write(&result, data).unwrap();
    result
}

pub(super) fn packet_times(path: &Path, selector: &str) -> Vec<(f64, f64)> {
    let data = execute(
        Command::new(ffmpeg())
            .args([
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "error",
                "-copyts",
                "-i",
            ])
            .arg(path)
            .args(["-map", selector, "-c", "copy", "-f", "framehash", "-"]),
    );
    let text = String::from_utf8(data).unwrap();
    let base = text
        .lines()
        .find_map(|line| line.strip_prefix("#tb 0: "))
        .unwrap();
    let (num, den) = base.split_once('/').unwrap();
    let scale = num.parse::<f64>().unwrap() / den.parse::<f64>().unwrap();
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split(',').collect();
            (
                fields[1].trim().parse::<f64>().unwrap() * scale,
                fields[2].trim().parse::<f64>().unwrap() * scale,
            )
        })
        .collect()
}

#[tokio::test]
async fn selected_separate_tracks_merge_to_playable_mp4_without_reencoding() {
    for (video_mp4, audio_mp4) in [(false, false), (true, true), (false, true), (true, false)] {
        let (files, server, inspection) = source(video_mp4, audio_mp4).await;
        let (_temp, store) = fixture();
        assert_eq!(inspection.variants[0].default_audio(), 1);
        let preview = hls::preview(&inspection, 0, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(preview.audio.as_ref().unwrap().name, "English");
        assert_eq!(preview.sequence, 7);
        assert_eq!(preview.audio.as_ref().unwrap().sequence, 41);
        assert_eq!(preview.output_extension(), "mp4");
        assert!(store.list().unwrap().is_empty());
        assert_eq!(
            server.requests.lock().unwrap().len(),
            3,
            "Preview must not download media"
        );
        let original_video = original_track(files.path(), &preview, false);
        let original_audio = original_track(files.path(), &preview, true);
        let job = enqueue(&store, &preview);
        assert_eq!(job.hls.as_ref().unwrap().version, 2);
        std::fs::create_dir_all(&job.directory).unwrap();
        let existing = job.directory.join(&job.name);
        std::fs::write(&existing, b"existing user file").unwrap();
        let saved = run(&store, job).await;
        assert_eq!(
            saved.status,
            Status::Complete,
            "{:?} ({video_mp4}, {audio_mp4})",
            saved.error
        );
        let path = saved.saved_path.clone().unwrap();
        assert_ne!(path, existing);
        assert_eq!(std::fs::read(&existing).unwrap(), b"existing user file");
        assert_eq!(path.extension().unwrap(), "mp4");
        assert_eq!(Some(std::fs::metadata(&path).unwrap().len()), saved.total);
        assert_eq!(saved.downloaded, saved.total.unwrap());
        assert_tracks_decode(&path);
        let video_hashes = packet_hashes(&original_video, "0:v:0", false);
        let audio_hashes = packet_hashes(&original_audio, "0:a:0", !audio_mp4);
        assert!(!video_hashes.is_empty() && !audio_hashes.is_empty());
        let actual_video = packet_hashes(&path, "0:v:0", false);
        let actual_audio = packet_hashes(&path, "0:a:0", false);
        assert_eq!(
            actual_video.len(),
            video_hashes.len(),
            "Video packet count ({video_mp4}, {audio_mp4})"
        );
        assert_eq!(
            actual_audio.len(),
            audio_hashes.len(),
            "Audio packet count ({video_mp4}, {audio_mp4})"
        );
        assert_eq!(
            actual_video, video_hashes,
            "Video payloads ({video_mp4}, {audio_mp4})"
        );
        assert_eq!(
            actual_audio, audio_hashes,
            "Audio payloads ({video_mp4}, {audio_mp4})"
        );
        let video_times = packet_times(&original_video, "0:v:0");
        let audio_times = packet_times(&original_audio, "0:a:0");
        let merged_video = packet_times(&path, "0:v:0");
        let merged_audio = packet_times(&path, "0:a:0");
        let shift = merged_video[0].0 - video_times[0].0;
        // Both tracks must retain one shared clock offset, including B-frame
        // presentation times and audio that begins before or after the video.
        for (expected, actual) in video_times
            .iter()
            .chain(&audio_times)
            .zip(merged_video.iter().chain(&merged_audio))
        {
            assert!(
                (actual.0 - expected.0 - shift).abs() < 0.002
                    && (actual.1 - expected.1 - shift).abs() < 0.002,
                "Changed A/V timing ({video_mp4},{audio_mp4}): {expected:?} -> {actual:?}, shift {shift}"
            );
        }
        assert_eq!(server.count("/unused-video.m3u8"), 0);
        assert_eq!(server.count("/unused-audio.m3u8"), 0);
        use sha2::{Digest, Sha256};
        assert_eq!(
            saved.integrity.actual,
            Some(format!(
                "{:x}",
                Sha256::digest(std::fs::read(&path).unwrap())
            ))
        );
        #[cfg(windows)]
        assert!(
            std::fs::read_to_string(format!("{}:Zone.Identifier", path.display()))
                .unwrap()
                .contains("ZoneId=3")
        );
        // Model a crash after atomic publication but before the completed state
        // was committed. Recover the merged output, whose size differs from TS.
        let mut interrupted = saved.clone();
        interrupted.status = Status::Downloading;
        interrupted.integrity.actual = None;
        let staging = saved
            .directory
            .join(format!(".idm-ex-{}.assembling", saved.id));
        std::fs::hard_link(&path, &staging).unwrap();
        store.save(&interrupted).unwrap();
        let count = server.requests.lock().unwrap().len();
        let recovered = run(&store, interrupted).await;
        assert_eq!(recovered.status, Status::Complete, "{:?}", recovered.error);
        assert_eq!(recovered.saved_path.as_ref(), Some(&path));
        assert_eq!(recovered.integrity.actual, saved.integrity.actual);
        assert_eq!(server.requests.lock().unwrap().len(), count);
    }
}

#[tokio::test]
async fn changed_or_encrypted_audio_blocks_all_media_and_never_requests_keys() {
    let (_files, server, inspection) = source(false, false).await;
    let (_temp, store) = fixture();
    let preview = hls::preview(&inspection, 0, &CancellationToken::new())
        .await
        .unwrap();
    for body in [media(&["different.ts"]), "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\"\n#EXTINF:6,\na.ts\n#EXT-X-ENDLIST\n".to_owned()] {
        server.set("/audio.m3u8", Reply::bytes(body));
        let saved = run(&store, enqueue(&store, &preview)).await;
        assert_eq!(saved.status, Status::Failed, "{:?}", saved.error);
        assert_eq!(saved.downloaded, 0);
        assert!(saved.saved_path.is_none());
    }
    assert_eq!(server.count("/key"), 0);
    for part in &preview.parts {
        assert_eq!(
            server.count(reqwest::Url::parse(&part.url).unwrap().path()),
            0
        );
    }
    assert!(
        hls::preview(&inspection, 0, &CancellationToken::new())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_missing_selected_audio_stream_fails_without_publishing_silent_video() {
    let (files, server, inspection) = source(false, false).await;
    // The advertised audio playlist contains valid video-only TS packets.
    for entry in std::fs::read_dir(files.path()).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        if name.starts_with("audio-") {
            server.set(
                &format!("/{name}"),
                Reply::bytes(std::fs::read(files.path().join("video-7.ts")).unwrap()),
            );
        }
    }
    let preview = hls::preview(&inspection, 0, &CancellationToken::new())
        .await
        .unwrap();
    let (_temp, store) = fixture();
    let failed = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(failed.status, Status::Failed, "{:?}", failed.error);
    assert!(failed.error.as_ref().unwrap().contains("Could not merge"));
    assert!(failed.saved_path.is_none());
    assert!(
        std::fs::read_dir(&failed.directory)
            .unwrap()
            .next()
            .is_none()
    );
    let count = server.count("/video-7.ts");
    // Merge failure keeps complete segments for retry; it never re-downloads them.
    let again = run(&store, failed).await;
    assert_eq!(again.status, Status::Failed);
    assert_eq!(server.count("/video-7.ts"), count);
}

#[tokio::test]
async fn pssh_in_a_media_fragment_fails_without_publishing_a_file() {
    let (files, server, inspection) = source(true, true).await;
    let preview = hls::preview(&inspection, 0, &CancellationToken::new())
        .await
        .unwrap();
    let part = preview
        .parts
        .iter()
        .find(|part| !part.initialization)
        .unwrap();
    let url = reqwest::Url::parse(&part.url).unwrap();
    let original = std::fs::read(files.path().join(url.path().trim_start_matches('/'))).unwrap();
    // One small valid version-0 PSSH box before the movie fragment. Initialization
    // remains clear, so this checks the real merger path after download validation.
    let mut protected = Vec::from(32u32.to_be_bytes());
    protected.extend_from_slice(b"pssh");
    protected.extend_from_slice(&[0; 24]);
    protected.extend_from_slice(&original);
    server.set(url.path(), Reply::bytes(protected));
    let (_temp, store) = fixture();
    let failed = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(failed.status, Status::Failed, "{:?}", failed.error);
    assert!(failed.error.as_ref().unwrap().contains("pssh"));
    assert!(failed.saved_path.is_none());
    assert_eq!(
        failed.hls.as_ref().unwrap().completed.len(),
        preview.part_count()
    );
    assert!(
        std::fs::read_dir(&failed.directory)
            .unwrap()
            .next()
            .is_none()
    );
}

#[tokio::test]
async fn pause_during_audio_resumes_verified_video_after_reopening_the_store() {
    let (_files, server, inspection) = source(true, true).await;
    let preview = hls::preview(&inspection, 0, &CancellationToken::new())
        .await
        .unwrap();
    let audio = &preview.audio.as_ref().unwrap().parts;
    let audio_path = reqwest::Url::parse(&audio[0].url)
        .unwrap()
        .path()
        .to_owned();
    let (_temp, store) = fixture();
    let paused = pause_at_part(&server, &store, enqueue(&store, &preview), &audio[0]).await;
    let id = paused.id.clone();
    assert_eq!(paused.status, Status::Paused);
    assert_eq!(
        paused.hls.as_ref().unwrap().completed.len(),
        preview.parts.len()
    );
    assert!(paused.saved_path.is_none());
    let reopened = Store::open(store.paths.clone()).unwrap();
    let resumed = run(&reopened, reopened.get(&id).unwrap().unwrap()).await;
    assert_eq!(resumed.status, Status::Complete, "{:?}", resumed.error);
    assert_tracks_decode(resumed.saved_path.as_ref().unwrap());
    assert_eq!(server.count(&audio_path), 2);
    for part in &preview.parts {
        assert_eq!(
            server.count(reqwest::Url::parse(&part.url).unwrap().path()),
            1
        );
    }
    assert_eq!(
        resumed
            .hls
            .as_ref()
            .unwrap()
            .playlist
            .audio
            .as_ref()
            .unwrap()
            .name,
        "English"
    );
}
