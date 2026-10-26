use super::*;

#[tokio::test]
async fn packed_aac_remux_preserves_the_transport_timestamp_against_video() {
    use crate::hls::{AudioTrack, Part, Playlist};
    fn execute(command: &mut Command) -> Vec<u8> {
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
    let temp = tempfile::tempdir().unwrap();
    let video = temp.path().join("video.ts");
    let audio = temp.path().join("audio.aac");
    let cancel = CancellationToken::new();
    let helper = Helper::checked(&cancel).await.unwrap();
    execute(
        Command::new(&helper.path)
            .args([
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=25:duration=1",
                "-c:v",
                "libx264",
                "-threads",
                "1",
                "-bf",
                "0",
                "-muxdelay",
                "0",
                "-muxpreload",
                "0",
                "-f",
                "mpegts",
            ])
            .arg(&video),
    );
    execute(
        Command::new(&helper.path)
            .args([
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=660:sample_rate=48000:duration=1",
                "-c:a",
                "aac",
                "-b:a",
                "96k",
                "-f",
                "adts",
            ])
            .arg(&audio),
    );
    let mut payload = b"com.apple.streaming.transportStreamTimestamp\0".to_vec();
    payload.extend(28800u64.to_be_bytes());
    let mut tag = b"ID3\x03\x00\x00\x00\x00\x00".to_vec();
    tag.push((payload.len() + 10) as u8);
    tag.extend(b"PRIV");
    tag.extend((payload.len() as u32).to_be_bytes());
    tag.extend([0, 0]);
    tag.extend(payload);
    tag.extend(std::fs::read(&audio).unwrap());
    std::fs::write(&audio, tag).unwrap();
    let timestamp = crate::hls::aac::validate(&audio, &cancel).await.unwrap();
    assert_eq!(timestamp, 28800);
    let part = Part {
        url: "https://example.test/part".into(),
        range: None,
        initialization: false,
    };
    let download = Download::new(Playlist {
        public_only: false,
        source: "https://example.test/master".into(),
        url: "https://example.test/video".into(),
        quality: "fixture".into(),
        bandwidth: None,
        duration_ms: 1000,
        sequence: 0,
        format: Format::TransportStream,
        parts: vec![part.clone()],
        dash: None,
        audio: Some(AudioTrack {
            name: "Audio".into(),
            language: None,
            url: "https://example.test/audio".into(),
            duration_ms: 1000,
            sequence: 0,
            format: Format::PackedAac,
            parts: vec![part],
        }),
    });
    let output_path = temp.path().join("output.mp4");
    let output = crate::download_file::fresh_read_write(&output_path)
        .await
        .unwrap()
        .into_std()
        .await;
    run(
        &mut command(
            &helper.path,
            &video,
            &audio,
            &download,
            2_000_000,
            Some(timestamp),
        ),
        &output,
        &cancel,
        Duration::from_secs(15),
        2_000_000,
    )
    .unwrap();
    drop(output);
    execute(
        Command::new(&helper.path)
            .args([
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "error",
                "-xerror",
                "-i",
            ])
            .arg(&output_path)
            .args(["-map", "0:v:0", "-map", "0:a:0", "-f", "null", "-"]),
    );
    let first_pts = |selector: &str| {
        let data = execute(
            Command::new(&helper.path)
                .args([
                    "-hide_banner",
                    "-nostdin",
                    "-loglevel",
                    "error",
                    "-copyts",
                    "-i",
                ])
                .arg(&output_path)
                .args(["-map", selector, "-c", "copy", "-f", "framehash", "-"]),
        );
        let data = String::from_utf8(data).unwrap();
        let tb = data
            .lines()
            .find_map(|s| s.strip_prefix("#tb 0: "))
            .unwrap();
        let (num, den) = tb.split_once('/').unwrap();
        let pts = data
            .lines()
            .find(|s| !s.starts_with('#') && !s.trim().is_empty())
            .unwrap()
            .split(',')
            .nth(2)
            .unwrap()
            .trim()
            .parse::<f64>()
            .unwrap();
        pts * num.parse::<f64>().unwrap() / den.parse::<f64>().unwrap()
    };
    let offset = first_pts("0:a:0") - first_pts("0:v:0");
    assert!((offset - 0.32).abs() < 0.025, "audio offset was {offset}");
}

#[test]
fn untrusted_or_missing_helper_is_never_started() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ffmpeg.exe");
    assert!(Helper::at(&path, &CancellationToken::new()).is_err());
    std::fs::write(&path, b"not the bundled helper").unwrap();
    let error = Helper::at(&path, &CancellationToken::new()).err().unwrap();
    assert!(error.to_string().contains("merger changed"));
}

// A child of the unit-test executable, never an arbitrary command or live scanner.
#[test]
fn merger_child_fixture() {
    let Ok(mode) = std::env::var("IDM_EX_MERGE_CHILD") else {
        return;
    };
    if mode == "flood" {
        use std::io::Write;
        std::io::stdout().write_all(&vec![42; 128 * 1024]).unwrap();
        std::io::stderr().write_all(&vec![b'e'; 64 * 1024]).unwrap();
        std::process::exit(0);
    }
    if mode == "error" {
        std::process::exit(7);
    }
    let ready = PathBuf::from(std::env::var_os("IDM_EX_MERGE_READY").unwrap());
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let _lock = options.open(ready).unwrap();
    std::thread::sleep(Duration::from_secs(20));
}
fn fixture(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "hls::merge::tests::merger_child_fixture",
            "--nocapture",
        ])
        .env("IDM_EX_MERGE_CHILD", mode);
    command
}

#[test]
fn merge_runner_bounds_output_and_reports_process_failure() {
    for (mode, maximum, expected) in [
        ("flood", 1024, "size limit"),
        ("error", 1024, "Could not merge"),
    ] {
        let file = tempfile::tempfile().unwrap();
        let error = run(
            &mut fixture(mode),
            &file,
            &CancellationToken::new(),
            Duration::from_secs(10),
            maximum,
        )
        .unwrap_err();
        assert!(error.to_string().contains(expected), "{error:#}");
    }
}

#[test]
fn pause_and_timeout_wait_for_the_owned_merger_to_exit() {
    for timeout in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let ready = temp.path().join("ready");
        let token = CancellationToken::new();
        let mut command = fixture("wait");
        command.env("IDM_EX_MERGE_READY", &ready);
        let ready_copy = ready.clone();
        let trigger = token.clone();
        let observer = std::thread::spawn(move || {
            let start = Instant::now();
            while !ready_copy.exists() {
                assert!(start.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(10));
            }
            if !timeout {
                trigger.cancel();
            }
        });
        let error = run(
            &mut command,
            &tempfile::tempfile().unwrap(),
            &token,
            if timeout {
                Duration::from_secs(1)
            } else {
                Duration::from_secs(10)
            },
            1024,
        )
        .unwrap_err();
        observer.join().unwrap();
        assert!(
            error
                .to_string()
                .contains(if timeout { "exceeded" } else { "cancelled" })
        );
        // The fixture holds this file exclusively until process termination.
        std::fs::remove_file(ready).unwrap();
    }
}

#[test]
fn cancelled_merge_never_launches_a_child() {
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("ready");
    let token = CancellationToken::new();
    token.cancel();
    let mut command = fixture("wait");
    command.env("IDM_EX_MERGE_READY", &ready);
    assert!(
        run(
            &mut command,
            &tempfile::tempfile().unwrap(),
            &token,
            Duration::from_secs(10),
            1024
        )
        .is_err()
    );
    assert!(!ready.exists());
}
