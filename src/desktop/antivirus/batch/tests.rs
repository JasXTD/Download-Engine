use super::*;
use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};

fn add_file(app: &mut Desktop, name: &str, exists: bool) -> Job {
    let mut job = Job::new(
        &format!("https://example.invalid/{name}"),
        None,
        app.store.paths.downloads.clone(),
        Uuid::new_v4().to_string(),
    )
    .unwrap();
    job.status = Status::Complete;
    job.saved_path = Some(app.store.paths.downloads.join(name));
    if exists {
        std::fs::write(
            job.saved_path.as_ref().unwrap(),
            b"harmless scanner fixture",
        )
        .unwrap();
    }
    app.store.enqueue(&job).unwrap();
    job
}
fn scanner(app: &Desktop) -> Settings {
    // Execute only this test's harmless child fixture, through the real scanner
    // process runner. No installed antivirus or OS configuration is touched.
    let settings = Settings {
        provider: Provider::Custom,
        executable: std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        arguments: vec![
            "--exact".into(),
            "desktop::antivirus::batch::tests::scanner_fixture".into(),
            "--nocapture".into(),
            "--skip={file}".into(),
        ],
        custom_confirmed: true,
        clean_exit: Some(0),
        detected_exit: Some(1),
        ..Settings::default()
    };
    app.store.save_antivirus_settings(&settings).unwrap();
    settings
}
fn external_batch(provider: Provider) -> (tempfile::TempDir, Desktop, egui::Context) {
    let (temp, mut app, ctx) = fixture();
    app.store
        .save_antivirus_settings(&Settings {
            provider,
            ..Settings::default()
        })
        .unwrap();
    for i in 1..=3 {
        add_file(&mut app, &format!("external-{i}.txt"), true);
    }
    app.open_antivirus_scan(ScanMode::Batch);
    click(&mut app, &ctx, "Select all");
    click(&mut app, &ctx, "Start scan");
    (temp, app, ctx)
}
fn external_handoff(app: &mut Desktop, verdict: Verdict) -> String {
    // Reproduce the real Explorer boundary: acknowledgement is not completion.
    // No installed antivirus is invoked by this regression test.
    let batch = app.antivirus.batch.as_ref().unwrap();
    let id = batch.current.clone().unwrap();
    app.antivirus.service.cancel_batch(batch.id);
    let job = app.store.get(&id).unwrap().unwrap();
    let provider = batch.settings.as_ref().unwrap().provider;
    app.antivirus.batch.as_mut().unwrap().completed(
        &id,
        &Ok(av::Outcome {
            report: Report {
                job_id: id.clone(),
                path: job.saved_path.unwrap(),
                scanner: provider.label().into(),
                settings_key: String::new(),
                verdict,
                message: "Explorer handoff fixture; no scan completion signal".into(),
                sha256: None,
                finished_at: 0,
            },
            permit: None,
        }),
    );
    app.advance_antivirus_batch();
    id
}

#[test]
fn malwarebytes_handoff_does_not_start_the_next_scan() {
    for external_provider in [
        Provider::Malwarebytes,
        Provider::Kaspersky,
        Provider::Bitdefender,
    ] {
        let (_temp, mut app, _ctx) = external_batch(external_provider);
        external_handoff(&mut app, Verdict::ScanRequested);
        assert!(
            !app.antivirus.service.busy(),
            "Explorer acknowledgement must not start the next file"
        );
        for _ in 0..3 {
            app.antivirus_background();
            assert!(!app.antivirus.service.busy());
        }
        assert_eq!(app.antivirus.batch.as_ref().unwrap().counts(), (1, 3));
    }
}

#[test]
fn malwarebytes_retry_next_and_finish_require_explicit_actions() {
    for external_provider in [
        Provider::Malwarebytes,
        Provider::Kaspersky,
        Provider::Bitdefender,
    ] {
        let (_temp, mut app, ctx) = external_batch(external_provider);
        let first = external_handoff(&mut app, Verdict::ScanRequested);
        click(&mut app, &ctx, "Retry this file");
        assert_eq!(
            app.antivirus.batch.as_ref().unwrap().current.as_ref(),
            Some(&first)
        );
        assert_eq!(app.antivirus.batch.as_ref().unwrap().counts(), (0, 3));
        assert_eq!(external_handoff(&mut app, Verdict::ScanRequested), first);
        click(&mut app, &ctx, "Scan next file");
        let second = external_handoff(&mut app, Verdict::ScanRequested);
        assert_ne!(second, first);
        click(&mut app, &ctx, "Scan next file");
        let third = external_handoff(&mut app, Verdict::ScanRequested);
        assert_ne!(third, second);
        assert!(app.antivirus.batch.as_ref().unwrap().running);
        assert!(!app.antivirus.service.busy());
        click(&mut app, &ctx, "Finish batch");
        let batch = app.antivirus.batch.as_ref().unwrap();
        assert!(!batch.running);
        assert!(batch.external_pending.is_none());
        assert_eq!(batch.counts(), (3, 3));
    }
}

#[test]
fn malwarebytes_failures_pause_instead_of_sending_more_requests() {
    for external_provider in [
        Provider::Malwarebytes,
        Provider::Kaspersky,
        Provider::Bitdefender,
    ] {
        for verdict in [Verdict::Failed, Verdict::TimedOut, Verdict::Cancelled] {
            let (_temp, mut app, ctx) = external_batch(external_provider);
            let first = external_handoff(&mut app, verdict);
            assert!(!app.antivirus.service.busy());
            app.antivirus_background();
            assert!(!app.antivirus.service.busy());
            click(&mut app, &ctx, "Retry this file");
            assert_eq!(
                app.antivirus.batch.as_ref().unwrap().current.as_ref(),
                Some(&first)
            );
            click(&mut app, &ctx, "Stop batch");
        }
    }
}

#[test]
fn malwarebytes_stop_and_close_cancel_pending_review_without_sending_more() {
    for external_provider in [
        Provider::Malwarebytes,
        Provider::Kaspersky,
        Provider::Bitdefender,
    ] {
        for close in [false, true] {
            let (_temp, mut app, ctx) = external_batch(external_provider);
            external_handoff(&mut app, Verdict::ScanRequested);
            click(&mut app, &ctx, if close { "×" } else { "Stop batch" });
            assert!(!app.antivirus.service.busy());
            app.antivirus_background();
            assert!(!app.antivirus.service.busy());
            if close {
                assert!(app.antivirus.batch.is_none());
            } else {
                let batch = app.antivirus.batch.as_ref().unwrap();
                assert!(!batch.running);
                assert!(batch.external_pending.is_none());
                assert_eq!(batch.counts(), (3, 3));
                assert_eq!(
                    batch
                        .entries
                        .iter()
                        .filter(|entry| matches!(
                            entry.result,
                            ResultState::Done(Verdict::Cancelled, _)
                        ))
                        .count(),
                    2
                );
                app.continue_external_batch(true);
                assert!(!app.antivirus.service.busy());
            }
        }
    }
}

#[test]
fn malwarebytes_settings_change_invalidates_pending_review() {
    for external_provider in [
        Provider::Malwarebytes,
        Provider::Kaspersky,
        Provider::Bitdefender,
    ] {
        let (_temp, mut app, ctx) = external_batch(external_provider);
        external_handoff(&mut app, Verdict::ScanRequested);
        app.store
            .save_antivirus_settings(&Settings::default())
            .unwrap();
        app.antivirus_background();
        click(&mut app, &ctx, "Scan next file");
        let batch = app.antivirus.batch.as_ref().unwrap();
        assert!(!batch.running);
        assert!(batch.external_pending.is_none());
        assert!(batch.error.as_ref().unwrap().contains("settings changed"));
        assert!(!app.antivirus.service.busy());
    }
}

#[test]
fn malwarebytes_review_controls_fit_minimum_window_in_both_themes() {
    for external_provider in [
        Provider::Malwarebytes,
        Provider::Kaspersky,
        Provider::Bitdefender,
    ] {
        for dark in [false, true] {
            let (_temp, mut app, ctx) = external_batch(external_provider);
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            external_handoff(&mut app, Verdict::ScanRequested);
            for size in [egui::vec2(860.0, 600.0), egui::vec2(1080.0, 800.0)] {
                frame(&mut app, &ctx, size, vec![]);
                let output = frame(&mut app, &ctx, size, vec![]);
                for label in ["Scan next file", "Retry this file", "Stop batch", "×"] {
                    assert!(
                        egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                            .contains_rect(text_rect(&output, label)),
                        "{label} clipped"
                    );
                }
            }
            click(&mut app, &ctx, "Stop batch");
        }
    }
}
/// Each scan starts a new copy of this large test binary. Under load (CPU at
/// 88-99%), one scan took up to 11 s, mostly waiting for CPU to create and
/// start the process; the fixture itself exits in under a second. The waits
/// detect a scan that stops making progress, so the deadline restarts whenever
/// a batch entry settles.
const SCAN_PROGRESS_LIMIT: Duration = Duration::from_secs(60);

fn poll_until(app: &mut Desktop, condition: impl Fn(&Desktop) -> bool) {
    let progress = |app: &Desktop| app.antivirus.batch.as_ref().map(|batch| batch.counts());
    let mut settled = progress(app);
    let mut deadline = Instant::now() + SCAN_PROGRESS_LIMIT;
    loop {
        app.antivirus_background();
        if condition(app) {
            break;
        }
        if progress(app) != settled {
            settled = progress(app);
            deadline = Instant::now() + SCAN_PROGRESS_LIMIT;
        }
        assert!(Instant::now() < deadline, "scan worker did not settle");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn scanner_fixture() {
    let Some(path) =
        std::env::args().find_map(|arg| arg.strip_prefix("--skip=").map(PathBuf::from))
    else {
        return;
    };
    let name = path.file_name().unwrap().to_string_lossy();
    if name.starts_with("slow") {
        std::fs::write(path.with_extension("ready"), b"ready").unwrap();
        // Outlive every wait: the scan settles in time only if Stop kills this
        // process. Its Job object still ends it if the test process exits first.
        std::thread::sleep(SCAN_PROGRESS_LIMIT * 2);
    }
    std::process::exit(if name.starts_with("detected") {
        1
    } else if name.starts_with("error") {
        7
    } else {
        0
    });
}

#[test]
fn batch_selects_completed_files_and_saves_real_scanner_results() {
    let (temp, mut app, ctx) = fixture();
    scanner(&app);
    let clean = add_file(&mut app, "clean.txt", true);
    let detected = add_file(&mut app, "detected.txt", true);
    let error = add_file(&mut app, "error.txt", true);
    let missing = add_file(&mut app, "missing.txt", false);
    let mut unfinished = add_file(&mut app, "unfinished.txt", true);
    unfinished.status = Status::Paused;
    app.store.save(&unfinished).unwrap();
    click(&mut app, &ctx, "Tools");
    click(&mut app, &ctx, "Antivirus Scan");
    click(&mut app, &ctx, "Batch");
    assert_eq!(app.dialog, Some(Dialog::AntivirusScan));
    assert_eq!(app.antivirus.batch.as_ref().unwrap().entries.len(), 4);
    click(&mut app, &ctx, "Select all");
    assert_eq!(app.antivirus.batch.as_ref().unwrap().counts(), (0, 4));
    click(&mut app, &ctx, "Clear selection");
    assert_eq!(app.antivirus.batch.as_ref().unwrap().counts(), (0, 0));
    click(&mut app, &ctx, "Select all");
    click(&mut app, &ctx, "Start scan");
    poll_until(&mut app, |app| {
        app.antivirus.batch.as_ref().unwrap().counts() == (4, 4)
    });
    for (job, expected) in [
        (&clean, Verdict::NoThreatsFound),
        (&detected, Verdict::Detected),
        (&error, Verdict::Failed),
        (&missing, Verdict::Failed),
    ] {
        assert_eq!(
            app.store
                .antivirus_report(&job.id)
                .unwrap()
                .unwrap()
                .verdict,
            expected
        );
    }
    assert!(
        app.store
            .antivirus_report(&unfinished.id)
            .unwrap()
            .is_none()
    );
    assert!(!app.antivirus.service.busy());
    // Explicit batch scanning must not turn on automatic scanning.
    assert!(!app.store.antivirus_settings().unwrap().enabled);
    click(&mut app, &ctx, "×");
    assert!(app.antivirus.batch.is_none());
    assert!(app.store.antivirus_report(&clean.id).unwrap().is_some());
    drop(app);
    temp.close().unwrap();
}

#[test]
fn single_picker_scans_only_the_chosen_file_and_keeps_its_saved_result() {
    let (temp, mut app, ctx) = fixture();
    scanner(&app);
    let first = add_file(&mut app, "single-first.txt", true);
    let second = add_file(&mut app, "single-second.txt", true);
    app.selected = Some(first.id.clone());
    click(&mut app, &ctx, "Tools");
    click(&mut app, &ctx, "Antivirus Scan");
    let batch = app.antivirus.batch.as_ref().unwrap();
    assert_eq!(batch.mode, ScanMode::Single);
    assert_eq!(batch.counts(), (0, 1));
    assert!(
        batch
            .entries
            .iter()
            .find(|entry| entry.id == first.id)
            .unwrap()
            .selected
    );
    click(&mut app, &ctx, "single-second.txt");
    assert_eq!(app.antivirus.batch.as_ref().unwrap().counts(), (0, 1));
    click(&mut app, &ctx, "Start scan");
    assert!(app.antivirus.service.status(&first.id).is_none());
    assert!(app.antivirus.service.status(&second.id).is_some());
    click(&mut app, &ctx, "Batch");
    assert_eq!(app.antivirus.batch.as_ref().unwrap().mode, ScanMode::Single);
    poll_until(&mut app, |app| {
        !app.antivirus.batch.as_ref().unwrap().busy()
    });
    assert_eq!(app.antivirus.batch.as_ref().unwrap().counts(), (1, 1));
    assert!(app.store.antivirus_report(&first.id).unwrap().is_none());
    assert_eq!(
        app.store
            .antivirus_report(&second.id)
            .unwrap()
            .unwrap()
            .verdict,
        Verdict::NoThreatsFound
    );
    // Finished reports survive changing modes and refreshing the picker.
    click(&mut app, &ctx, "Batch");
    click(&mut app, &ctx, "Select all");
    assert_eq!(app.antivirus.batch.as_ref().unwrap().counts(), (0, 2));
    click(&mut app, &ctx, "Single");
    assert_eq!(app.antivirus.batch.as_ref().unwrap().counts(), (0, 1));
    click(&mut app, &ctx, "single-second.txt");
    click(&mut app, &ctx, "Start scan");
    poll_until(&mut app, |app| {
        !app.antivirus.batch.as_ref().unwrap().busy()
    });
    let third = add_file(&mut app, "single-third.txt", true);
    click(&mut app, &ctx, "New scan");
    let batch = app.antivirus.batch.as_ref().unwrap();
    assert_eq!(batch.mode, ScanMode::Single);
    assert!(!batch.started);
    assert!(batch.entries.iter().any(|entry| entry.id == third.id));
    assert!(app.store.antivirus_report(&second.id).unwrap().is_some());
    assert!(!app.store.antivirus_settings().unwrap().enabled);
    click(&mut app, &ctx, "×");
    assert!(!app.antivirus.service.busy());
    drop(app);
    temp.close().unwrap();
}

#[test]
fn single_requires_a_selection_and_malwarebytes_waits_for_review() {
    for external_provider in [
        Provider::Malwarebytes,
        Provider::Kaspersky,
        Provider::Bitdefender,
    ] {
        let (_temp, mut app, ctx) = fixture();
        app.store
            .save_antivirus_settings(&Settings {
                provider: external_provider,
                ..Settings::default()
            })
            .unwrap();
        let first = add_file(&mut app, "single-review.txt", true);
        add_file(&mut app, "single-unselected.txt", true);
        app.selected = None;
        app.open_antivirus_scan(ScanMode::Single);
        click(&mut app, &ctx, "Start scan");
        assert!(!app.antivirus.service.busy());
        assert!(!app.antivirus.batch.as_ref().unwrap().started);
        click(&mut app, &ctx, "single-review.txt");
        click(&mut app, &ctx, "Start scan");
        assert_eq!(external_handoff(&mut app, Verdict::ScanRequested), first.id);
        click(&mut app, &ctx, "Batch");
        assert_eq!(app.antivirus.batch.as_ref().unwrap().mode, ScanMode::Single);
        click(&mut app, &ctx, "Retry this file");
        assert_eq!(external_handoff(&mut app, Verdict::ScanRequested), first.id);
        click(&mut app, &ctx, "Finish review");
        assert!(!app.antivirus.batch.as_ref().unwrap().busy());
        assert!(!app.antivirus.service.busy());
        click(&mut app, &ctx, "New scan");
        click(&mut app, &ctx, "single-review.txt");
        click(&mut app, &ctx, "Start scan");
        external_handoff(&mut app, Verdict::ScanRequested);
        click(&mut app, &ctx, "Cancel scan");
        assert!(!app.antivirus.batch.as_ref().unwrap().busy());
        app.antivirus_background();
        assert!(!app.antivirus.service.busy());
        click(&mut app, &ctx, "×");
    }
}

#[test]
fn antivirus_mode_switches_keep_the_dialog_and_backdrop_visible() {
    for dark in [false, true] {
        let (_temp, mut app, ctx) = fixture();
        let size = egui::vec2(860.0, 600.0);
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        ctx.all_styles_mut(|style| style.animation_time = 0.15);
        add_file(&mut app, "visible-picker.txt", true);
        app.open_antivirus_scan(ScanMode::Single);
        for _ in 0..24 {
            frame(&mut app, &ctx, size, vec![]);
        }
        for label in ["Batch", "Single", "Batch", "Single"] {
            let out = frame(&mut app, &ctx, size, vec![]);
            let position = text_rect(&out, label).center();
            for pressed in [true, false] {
                let out = frame(
                    &mut app,
                    &ctx,
                    size,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
                assert_scan_visible(&out, size);
            }
            for _ in 0..12 {
                assert_scan_visible(&frame(&mut app, &ctx, size, vec![]), size);
            }
        }
    }
}

fn assert_scan_visible(out: &egui::FullOutput, size: egui::Vec2) {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    assert!(out.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.rect.contains_rect(screen) && rect.fill == Color32::from_black_alpha(100))), "Scan tabs lost or faded the modal backdrop");
    for label in [
        "Antivirus Scan",
        "Single",
        "Batch",
        "Start scan",
        "×",
        "Scan settings",
    ] {
        assert!(
            screen.contains_rect(text_rect(out, label)),
            "{label} clipped or missing"
        );
    }
}

#[test]
fn stop_batch_terminates_active_child_and_preserves_unrelated_scan() {
    for mode in [ScanMode::Single, ScanMode::Batch] {
        let (temp, mut app, ctx) = fixture();
        let settings = scanner(&app);
        let slow = add_file(&mut app, "slow.txt", true);
        let waiting = add_file(&mut app, "waiting.txt", true);
        app.selected = Some(slow.id.clone());
        app.open_antivirus_scan(mode);
        // The order is explicit here: store listing order is newest first.
        app.antivirus
            .batch
            .as_mut()
            .unwrap()
            .entries
            .sort_by_key(|entry| entry.id != slow.id);
        if mode == ScanMode::Batch {
            click(&mut app, &ctx, "Select all");
        }
        click(&mut app, &ctx, "Start scan");
        poll_until(&mut app, |_| {
            slow.saved_path
                .as_ref()
                .unwrap()
                .with_extension("ready")
                .exists()
        });
        let unrelated = add_file(&mut app, "unrelated.txt", true);
        app.antivirus
            .service
            .enqueue(unrelated.clone(), settings, false);
        click(
            &mut app,
            &ctx,
            if mode == ScanMode::Single {
                "Cancel scan"
            } else {
                "Stop batch"
            },
        );
        click(
            &mut app,
            &ctx,
            if mode == ScanMode::Single {
                "Batch"
            } else {
                "Single"
            },
        );
        assert_eq!(app.antivirus.batch.as_ref().unwrap().mode, mode);
        assert!(app.antivirus.service.status(&unrelated.id).is_some());
        poll_until(&mut app, |app| !app.antivirus.service.busy());
        assert_eq!(
            app.store
                .antivirus_report(&slow.id)
                .unwrap()
                .unwrap()
                .verdict,
            Verdict::Cancelled
        );
        assert!(app.store.antivirus_report(&waiting.id).unwrap().is_none());
        assert_eq!(
            app.store
                .antivirus_report(&unrelated.id)
                .unwrap()
                .unwrap()
                .verdict,
            Verdict::NoThreatsFound
        );
        let total = if mode == ScanMode::Single { 1 } else { 2 };
        assert_eq!(
            app.antivirus.batch.as_ref().unwrap().counts(),
            (total, total)
        );
        // No remaining scanner process retains its guarded file.
        std::fs::write(slow.saved_path.as_ref().unwrap(), b"handles released").unwrap();
        click(&mut app, &ctx, "×");
        drop(app);
        temp.close().unwrap();
    }
}

#[test]
fn closing_batch_cancels_queued_work_and_releases_results() {
    let (_temp, mut app, ctx) = fixture();
    scanner(&app);
    add_file(&mut app, "clean.txt", true);
    app.open_antivirus_scan(ScanMode::Batch);
    click(&mut app, &ctx, "Select all");
    click(&mut app, &ctx, "Start scan");
    assert!(app.antivirus.service.busy());
    click(&mut app, &ctx, "×");
    assert!(!app.antivirus.service.busy());
    assert!(app.antivirus.batch.is_none());
    assert!(app.dialog.is_none());
}

#[test]
fn changed_download_and_existing_scan_are_skipped() {
    let (_temp, mut app, ctx) = fixture();
    let settings = scanner(&app);
    let mut changed = add_file(&mut app, "changed.txt", true);
    let busy = add_file(&mut app, "busy.txt", true);
    app.open_antivirus_scan(ScanMode::Batch);
    click(&mut app, &ctx, "Select all");
    changed.status = Status::Paused;
    app.store.save(&changed).unwrap();
    app.antivirus.service.enqueue(busy.clone(), settings, false);
    click(&mut app, &ctx, "Start scan");
    let batch = app.antivirus.batch.as_ref().unwrap();
    assert_eq!(batch.counts(), (2, 2));
    assert!(
        batch
            .entries
            .iter()
            .all(|entry| matches!(entry.result, ResultState::Skipped(_)))
    );
    app.stop_antivirus_batch();
    assert!(app.antivirus.service.status(&busy.id).is_some());
    app.antivirus.service.cancel_all();
}

#[test]
fn batch_controls_fit_both_themes_and_the_minimum_window() {
    for external_provider in [
        Provider::Malwarebytes,
        Provider::Kaspersky,
        Provider::Bitdefender,
    ] {
        for dark in [false, true] {
            let (_temp, mut app, ctx) = fixture();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            scanner(&app);
            add_file(
                &mut app,
                "a-very-long-file-name-that-must-not-push-controls-outside-the-dialog.txt",
                true,
            );
            app.open_antivirus_scan(ScanMode::Batch);
            for size in [egui::vec2(860.0, 600.0), egui::vec2(1080.0, 800.0)] {
                frame(&mut app, &ctx, size, vec![]);
                let output = frame(&mut app, &ctx, size, vec![]);
                for label in [
                    "Antivirus Scan",
                    "Select all",
                    "Clear selection",
                    "Start scan",
                    "×",
                    "Scan settings",
                ] {
                    assert!(
                        egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                            .contains_rect(text_rect(&output, label)),
                        "{label} clipped"
                    );
                }
            }
            app.antivirus
                .batch
                .as_mut()
                .unwrap()
                .settings
                .as_mut()
                .unwrap()
                .provider = external_provider;
            frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
            let output = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
            assert!(text_rect(&output, "Start scan").max.y < 600.0);
            click(&mut app, &ctx, "×");
            app.store.remove(&app.store.list().unwrap()[0].id).unwrap();
            app.open_antivirus_scan(ScanMode::Batch);
            assert!(app.antivirus.batch.as_ref().unwrap().entries.is_empty());
        }
    }
}
