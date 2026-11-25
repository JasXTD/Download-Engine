use super::*;
use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};

fn tone(path: &Path) -> Vec<u8> {
    // One tenth of a second of mono PCM, with a regular RIFF header.
    let mut wav = b"RIFF\x64\x06\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\x40\x06\0\0".to_vec();
    wav.resize(1644, 0);
    std::fs::write(path, &wav).unwrap();
    wav
}

fn wait_for_conversion(app: &mut Desktop, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.extras.converter.running.is_some() {
        assert!(Instant::now() < deadline, "conversion did not finish");
        app.poll_conversion(ctx);
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn toolbar_opens_form_and_waits_for_explicit_conversion() {
    let (dir, mut app, ctx) = fixture();
    let input = dir.path().join("My recording.wav");
    let original = tone(&input);
    let suggested = Format::Mp3.suggested_output(&input);
    let custom = dir.path().join("My chosen name.mp3");
    app.folder_picker.files_response = Some(Ok(None));
    click(&mut app, &ctx, "Converter");
    assert_eq!(app.dialog, Some(Dialog::Converter));
    assert_eq!(app.extras.converter.bitrate, Mp3Bitrate::Kbps192);
    click(&mut app, &ctx, "192 kbps");
    click(&mut app, &ctx, "320 kbps");
    assert_eq!(app.extras.converter.bitrate, Mp3Bitrate::Kbps320);
    assert!(
        app.folder_picker.files_response.is_some(),
        "opening the form consumed the file picker"
    );
    click(&mut app, &ctx, "Convert");
    click(&mut app, &ctx, "Save as…");
    assert!(app.folder_picker.files_response.is_some() && app.extras.converter.running.is_none());
    click(&mut app, &ctx, "Choose files");
    assert!(app.folder_picker.files_response.is_none() && app.extras.converter.rows.is_empty());
    app.folder_picker.files_response = Some(Ok(Some(vec![input.clone()])));
    click(&mut app, &ctx, "Choose files");
    assert_eq!(app.extras.converter.rows[0].output, suggested);
    assert!(!suggested.exists());
    for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
        frame(&mut app, &ctx, size, vec![]);
        let rendered = frame(&mut app, &ctx, size, vec![]);
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let choose = text_rect(&rendered, "Choose files").center();
        let save = text_rect(&rendered, "Save as…").center();
        assert!((choose.x - save.x).abs() < 2.0 && choose.x > viewport.center().x);
        for label in [
            "My recording.wav",
            "My recording.mp3",
            "Choose files",
            "Save as…",
            "Convert",
        ] {
            assert!(
                viewport.contains_rect(text_rect(&rendered, label)),
                "{label} at {size:?}"
            );
        }
    }
    app.folder_picker.save_response = Some(Ok(Some(custom.display().to_string())));
    click(&mut app, &ctx, "Save as…");
    app.folder_picker.save_response = Some(Ok(None));
    click(&mut app, &ctx, "Save as…");
    app.folder_picker.files_response = Some(Ok(Some(vec![input.clone()])));
    click(&mut app, &ctx, "Choose files");
    assert_eq!(app.extras.converter.rows[0].output, custom);
    assert!(!custom.exists() && app.extras.converter.running.is_none());
    click(&mut app, &ctx, "Convert");
    wait_for_conversion(&mut app, &ctx);
    assert_ne!(app.dialog, Some(Dialog::Converter));
    assert!(
        app.notice
            .as_ref()
            .unwrap()
            .message
            .contains("My chosen name.mp3")
    );
    assert!(custom.metadata().unwrap().len() > 0 && !suggested.exists());
    assert_eq!(std::fs::read(input).unwrap(), original);
}

#[test]
fn tools_opens_same_form_and_an_existing_output_can_be_corrected_in_place() {
    let (dir, mut app, ctx) = fixture();
    let input = dir.path().join("input.wav");
    tone(&input);
    let existing = Format::Mp3.suggested_output(&input);
    std::fs::write(&existing, b"original").unwrap();
    click(&mut app, &ctx, "Tools");
    click(&mut app, &ctx, "Converter");
    assert_eq!(app.dialog, Some(Dialog::Converter));
    app.folder_picker.files_response = Some(Ok(Some(vec![input.clone()])));
    click(&mut app, &ctx, "Choose files");
    click(&mut app, &ctx, "Convert");
    wait_for_conversion(&mut app, &ctx);
    assert_eq!(app.dialog, Some(Dialog::Converter));
    assert!(
        matches!(&app.extras.converter.rows[0].status, FileStatus::Failed(msg) if msg.contains("already exists"))
    );
    assert_eq!(std::fs::read(&existing).unwrap(), b"original");
    let other = dir.path().join("retry.mp3");
    app.folder_picker.save_response = Some(Ok(Some(other.display().to_string())));
    click(&mut app, &ctx, "Save as…");
    assert!(app.extras.converter.error.is_none());
    click(&mut app, &ctx, "Convert");
    wait_for_conversion(&mut app, &ctx);
    assert!(other.metadata().unwrap().len() > 0);
    assert_eq!(std::fs::read(existing).unwrap(), b"original");
    click(&mut app, &ctx, "Converter");
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none() && app.extras.converter.rows.is_empty());
}

#[test]
fn running_form_locks_the_file_choices_and_cancel_discards_the_output() {
    use std::io::Write;
    let (dir, mut app, ctx) = fixture();
    let input = dir.path().join("long recording.wav");
    let mut header = tone(&input);
    let bytes = 8000u32 * 2 * 1800;
    header[4..8].copy_from_slice(&(bytes + 36).to_le_bytes());
    header[40..44].copy_from_slice(&bytes.to_le_bytes());
    let mut file = std::fs::File::create(&input).unwrap();
    file.write_all(&header).unwrap();
    file.set_len(u64::from(bytes) + 44).unwrap();
    drop(file);
    click(&mut app, &ctx, "Converter");
    let later = dir.path().join("later.wav");
    tone(&later);
    app.folder_picker.files_response = Some(Ok(Some(vec![input.clone(), later.clone()])));
    click(&mut app, &ctx, "Choose files");
    click(&mut app, &ctx, "Convert");
    assert!(app.extras.converter.running.is_some());
    app.folder_picker.files_response = Some(Ok(None));
    click(&mut app, &ctx, "Choose files");
    assert!(app.folder_picker.files_response.is_some());
    click(&mut app, &ctx, "Cancel");
    wait_for_conversion(&mut app, &ctx);
    assert_eq!(app.dialog, Some(Dialog::Converter));
    assert!(
        app.extras
            .converter
            .rows
            .iter()
            .all(|r| r.status == FileStatus::Cancelled)
    );
    assert!(!Format::Mp3.suggested_output(&input).exists());
    assert!(!Format::Mp3.suggested_output(&later).exists());
    assert!(
        !std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(".de-convert-"))
    );
}

#[test]
fn format_picker_and_batch_convert_distinct_files_and_retry_only_failures() {
    let (dir, mut app, ctx) = fixture();
    let output_dir = dir.path().join("converted");
    std::fs::create_dir(&output_dir).unwrap();
    let first = dir.path().join("A.wav");
    let second = dir.path().join("B.wav");
    let broken = dir.path().join("missing audio.wav");
    tone(&first);
    tone(&second);
    std::fs::write(&broken, b"invalid media").unwrap();
    click(&mut app, &ctx, "Converter");
    click(&mut app, &ctx, "MP3");
    click(&mut app, &ctx, "WAV");
    app.folder_picker.files_response = Some(Ok(Some(vec![
        first.clone(),
        broken.clone(),
        first.clone(),
        second.clone(),
    ])));
    click(&mut app, &ctx, "Choose files");
    assert_eq!(app.extras.converter.rows.len(), 3);
    assert_eq!(app.extras.converter.format, Format::Wav);
    app.folder_picker.response = Some(Ok(Some(output_dir.display().to_string())));
    click(&mut app, &ctx, "Choose a folder");
    let a = output_dir.join("A-converted.wav");
    let b = output_dir.join("B-converted.wav");
    click(&mut app, &ctx, "Convert");
    wait_for_conversion(&mut app, &ctx);
    assert_eq!(&std::fs::read(&a).unwrap()[..4], b"RIFF");
    assert_eq!(&std::fs::read(&b).unwrap()[..4], b"RIFF");
    assert!(matches!(
        app.extras.converter.rows[1].status,
        FileStatus::Failed(_)
    ));
    assert_eq!(app.extras.converter.rows[2].status, FileStatus::Saved);
    // A retry must not touch already completed outputs, even if the user moved them.
    std::fs::remove_file(&a).unwrap();
    tone(&broken);
    click(&mut app, &ctx, "Convert");
    assert!(
        app.extras.converter.running.is_some(),
        "retry did not start; form error: {:?}, file status: {:?}",
        app.extras.converter.error,
        app.extras.converter.rows[1].status
    );
    wait_for_conversion(&mut app, &ctx);
    assert!(!a.exists());
    assert!(
        app.extras
            .converter
            .rows
            .iter()
            .all(|r| r.status == FileStatus::Saved),
        "retry result: {:?}",
        app.extras.converter.rows[1].status
    );
    assert!(output_dir.join("missing audio-converted.wav").is_file());
    click(&mut app, &ctx, "Close");
    assert!(app.dialog.is_none());
}

#[test]
fn suggested_batch_names_protect_input_and_disambiguate_equal_stems() {
    let mut state = Converter::default();
    state.select_sources(
        ["clip.mp3", "clip.flac", "clip.MP4"]
            .map(PathBuf::from)
            .into(),
    );
    assert_eq!(
        state
            .rows
            .iter()
            .map(|r| r.output.clone())
            .collect::<Vec<_>>(),
        ["clip-converted.mp3", "clip (2).mp3", "clip (3).mp3"].map(PathBuf::from)
    );
    state.change_format(Format::Wav);
    assert_eq!(
        state
            .rows
            .iter()
            .map(|r| r.output.clone())
            .collect::<Vec<_>>(),
        ["clip.wav", "clip (2).wav", "clip (3).wav"].map(PathBuf::from)
    );
}
