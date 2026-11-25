use super::*;

#[test]
fn output_writes_must_finish_and_stay_within_the_limit() {
    let input = b"abcdef";
    let mut short = [0u8; 4];
    let error = write_audio(&input[..], &mut short[..], 10).unwrap_err();
    assert_eq!(&short, b"abcd");
    assert!(error.to_string().contains("Cannot write the audio file"));
    for limit in [5, 6, 7] {
        let mut out = Vec::new();
        let result = write_audio(&input[..], &mut out, limit);
        if limit == 5 {
            assert!(result.unwrap_err().to_string().contains("size limit"));
            assert_eq!(out, b"abcde");
        } else {
            assert_eq!(result.unwrap(), input.len() as u64);
            assert_eq!(out, input);
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires an empty disposable 64 KiB filesystem"]
fn full_disk_does_not_publish_a_partial_mp3() {
    let root = std::env::var_os("DOWNLOAD_ENGINE_TEST_MP3_SMALL_VOLUME").unwrap();
    let small = tempfile::tempdir_in(root).unwrap();
    let normal = tempfile::tempdir().unwrap();
    let source = normal.path().join("tone.wav");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=2.70",
        source.to_str().unwrap(),
    ]);
    let original = fs::read(&source).unwrap();
    let out = normal.path().join("control.mp3");
    let token = CancellationToken::new();
    convert(&source, &out, &token).unwrap();
    assert!(out.metadata().unwrap().len() > 65_536);
    let full = small.path().join("must not exist.mp3");
    let error = convert(&source, &full, &token).unwrap_err();
    assert!(
        error.to_string().contains("Cannot write the audio file"),
        "{error:#}"
    );
    assert!(!full.exists());
    assert_eq!(fs::read_dir(small.path()).unwrap().count(), 0);
    assert_eq!(fs::read(source).unwrap(), original);
}

fn ffmpeg(args: &[&str]) -> Vec<u8> {
    let helper = Helper::ffmpeg(&CancellationToken::new()).unwrap();
    let out = helper
        .command()
        .args(["-nostdin", "-loglevel", "error"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn check_tone(path: &Path, format: Format) {
    // Force the selected demuxer, decode a full file and measure its actual signal.
    let pcm = ffmpeg(&[
        "-f",
        format.extension(),
        "-i",
        path.to_str().unwrap(),
        "-ar",
        "8000",
        "-ac",
        "1",
        "-f",
        "s16le",
        "pipe:1",
    ]);
    let (pairs, remainder) = pcm.as_chunks::<2>();
    assert!(remainder.is_empty(), "incomplete 16-bit PCM sample");
    let samples: Vec<_> = pairs.iter().copied().map(i16::from_le_bytes).collect();
    assert!(
        (7900..9000).contains(&samples.len()),
        "duration: {} samples",
        samples.len()
    );
    let cycles = samples
        .windows(2)
        .filter(|pair| pair[0] < 0 && pair[1] >= 0)
        .count();
    assert!(
        (410..480).contains(&cycles),
        "expected a 440 Hz tone, found {cycles} cycles"
    );
}

#[test]
fn audio_and_video_produce_playable_mp3_and_wav_from_the_first_track() {
    let dir = tempfile::tempdir().unwrap();
    let token = CancellationToken::new();
    for name in [
        "tone.wav",
        "lossless.flac",
        "two tracks.mp4",
        "vorbis-ogg.ogg",
        "vorbis-matroska.mka",
    ] {
        let source = dir.path().join(name);
        let mut args = vec!["-f", "lavfi", "-i", "sine=frequency=440:duration=1"];
        if name.ends_with(".mp4") {
            args.extend([
                "-f",
                "lavfi",
                "-i",
                "color=size=32x32:duration=1",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1000:duration=1",
                "-map",
                "1:v",
                "-map",
                "0:a",
                "-map",
                "2:a",
                "-c:v",
                "mpeg4",
                "-c:a",
                "aac",
            ]);
        } else if name.starts_with("vorbis-") {
            args.extend(["-ac", "2", "-c:a", "vorbis", "-strict", "experimental"]);
        }
        args.push(source.to_str().unwrap());
        ffmpeg(&args);
        #[cfg(windows)]
        if name == "tone.wav" {
            download_file::mark_internet(&source).unwrap();
        }
        let original = fs::read(&source).unwrap();
        let output = Format::Mp3.suggested_output(&source);
        convert(&source, &output, &token).unwrap();
        check_tone(&output, Format::Mp3);
        let wav = Format::Wav.suggested_output(&source);
        convert(&source, &wav, &token).unwrap();
        let bytes = fs::read(&wav).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize + 8,
            bytes.len()
        );
        assert_eq!(
            u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize + 44,
            bytes.len()
        );
        assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            44_100
        );
        assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 16);
        check_tone(&wav, Format::Wav);
        #[cfg(windows)]
        if name == "tone.wav" {
            let mut mark = output.as_os_str().to_owned();
            mark.push(":Zone.Identifier");
            assert!(fs::read_to_string(mark).unwrap().contains("ZoneId=3"));
        }
        assert_eq!(original, fs::read(&source).unwrap());
        assert_eq!(
            Format::Mp3
                .suggested_output(&output)
                .file_name()
                .unwrap()
                .to_string_lossy(),
            format!(
                "{}-converted.mp3",
                output.file_stem().unwrap().to_string_lossy()
            )
        );
    }
}

#[test]
fn each_mp3_bitrate_is_present_in_the_encoded_audio() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tone.wav");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=1",
        source.to_str().unwrap(),
    ]);
    for bitrate in Mp3Bitrate::ALL {
        let output = dir.path().join(format!("{}.mp3", bitrate.kbps()));
        convert_with_bitrate(&source, &output, bitrate, &CancellationToken::new()).unwrap();
        let packets = String::from_utf8(ffmpeg(&[
            "-i",
            output.to_str().unwrap(),
            "-map",
            "0:a:0",
            "-c:a",
            "copy",
            "-f",
            "framecrc",
            "pipe:1",
        ]))
        .unwrap();
        // MPEG-1 Layer III has 1,152 samples per frame. Padding adds at most one byte.
        let frame_bytes = 1152 * usize::from(bitrate.kbps()) * 1000 / (8 * 44100);
        let frames: Vec<_> = packets
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .map(|l| {
                l.split(',')
                    .nth(4)
                    .unwrap()
                    .trim()
                    .parse::<usize>()
                    .unwrap()
            })
            .collect();
        assert!(
            frames.len() >= 38,
            "Expected at least one second of MP3 frames"
        );
        assert!(
            frames
                .iter()
                .all(|&n| n == frame_bytes || n == frame_bytes + 1),
            "Unexpected frame size at {bitrate:?}: {frames:?}"
        );
    }
}

#[test]
fn no_output_for_silent_video_invalid_media_existing_destination_or_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("input.mp4");
    let out = dir.path().join("new.mp3");
    let token = CancellationToken::new();
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=size=32x32:duration=0.1",
        "-c:v",
        "mpeg4",
        source.to_str().unwrap(),
    ]);
    assert!(
        convert(&source, &out, &token)
            .unwrap_err()
            .to_string()
            .contains("decode audio")
    );
    fs::write(&source, b"#EXTM3U\n#EXTINF:1,\nfile:secret.wav\n").unwrap();
    assert!(convert(&source, &out, &token).is_err());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    fs::write(&out, b"keep me").unwrap();
    assert!(
        convert(&source, &out, &token)
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    assert!(convert(&out, &out, &token).is_err());
    assert_eq!(fs::read(&out).unwrap(), b"keep me");
    token.cancel();
    assert!(convert(&source, &dir.path().join("cancel.mp3"), &token).is_err());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn cancel_running_conversion_cleans_staging_and_releases_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("long.wav");
    let out = Format::Mp3.suggested_output(&source);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "anullsrc=r=8000:cl=mono",
        "-t",
        "0.1",
        "-fflags",
        "+bitexact",
        "-c:a",
        "pcm_s16le",
        source.to_str().unwrap(),
    ]);
    let mut wav = fs::read(&source).unwrap();
    let data = wav.windows(4).position(|w| w == b"data").unwrap();
    let length = 8_000u32 * 2 * 1800;
    wav[4..8].copy_from_slice(&(length + data as u32).to_le_bytes());
    wav[data + 4..data + 8].copy_from_slice(&length.to_le_bytes());
    let mut file = File::create(&source).unwrap();
    file.write_all(&wav).unwrap();
    file.set_len(u64::from(length) + data as u64 + 8).unwrap();
    drop(file);
    let token = CancellationToken::new();
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| convert(&source, &out, &token));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let running = fs::read_dir(dir.path()).unwrap().flatten().any(|entry| {
                entry.path().extension().is_some_and(|e| e == "part")
                    && File::open(entry.path())
                        .and_then(|file| file.metadata())
                        .is_ok_and(|m| m.len() > 0)
            });
            if running {
                break;
            }
            if worker.is_finished() || Instant::now() > deadline {
                token.cancel();
                panic!(
                    "did not observe encoded data in the temporary file: {:?}",
                    worker.join()
                );
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        // A file created while the converter runs must survive cancellation.
        fs::write(&out, b"another application").unwrap();
        token.cancel();
        assert!(
            worker
                .join()
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
    });
    assert_eq!(fs::read(&out).unwrap(), b"another application");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    fs::remove_file(source).unwrap();
}
