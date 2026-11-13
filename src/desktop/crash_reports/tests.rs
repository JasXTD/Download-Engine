use super::super::interaction_tests::{click, fixture, frame, text_rect};
use super::*;

fn saved(app: &Desktop) -> service::Report {
    let report = service::Report {
        schema: 1,
        id: Uuid::new_v4(),
        session_started_utc: "2026-11-04T01:00:00.000Z".into(),
        recorded_utc: "2026-11-04T01:01:00.000Z".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        os: "windows".into(),
        architecture: "x86_64".into(),
        component: download_engine::diagnostics::Component::Desktop,
        kind: Kind::RustPanic,
        location: Some(service::SourceLocation {
            file: "src/desktop.rs".into(),
            line: 42,
            column: 3,
        }),
    };
    let root = service::directory(&app.store.paths);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join(format!("report-{}.json", report.id)),
        serde_json::to_vec(&report).unwrap(),
    )
    .unwrap();
    report
}

fn settle(app: &mut Desktop, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.crash_reports.task.is_some() {
        app.poll_crash_reports(ctx);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}

#[test]
fn next_launch_notice_waits_for_other_dialogs_and_dismissal_is_persistent() {
    let (_temp, mut app, ctx) = fixture();
    let report = saved(&app);
    app.dialog = Some(Dialog::About);
    app.check_crash_reports_on_launch(&ctx);
    settle(&mut app, &ctx);
    assert_eq!(app.dialog, Some(Dialog::About));
    assert!(app.crash_reports.prompt);
    app.dialog = None;
    app.poll_crash_reports(&ctx);
    assert_eq!(app.dialog, Some(Dialog::CrashReports));
    app.close_crash_reports(&ctx);
    settle(&mut app, &ctx);
    let snapshot = service::load(&app.store.paths).unwrap();
    assert_eq!(snapshot.reports[0].report.id, report.id);
    assert!(!snapshot.reports[0].unreviewed);
    app.check_crash_reports_on_launch(&ctx);
    settle(&mut app, &ctx);
    assert_eq!(app.dialog, None);
    app.open_crash_reports(&ctx);
    settle(&mut app, &ctx);
    assert_eq!(app.crash_reports.reports.len(), 1);
}

#[test]
fn help_export_cancel_error_retry_and_review_preserve_downloads() {
    let (temp, mut app, ctx) = fixture();
    let report = saved(&app);
    let jobs = app.jobs.len();
    let selected = app.selected.clone();
    click(&mut app, &ctx, "Help");
    click(&mut app, &ctx, "Crash reports");
    settle(&mut app, &ctx);
    assert_eq!(app.dialog, Some(Dialog::CrashReports));
    app.folder_picker.response = Some(Ok(None));
    app.export_crash_report(&ctx);
    assert!(app.crash_reports.task.is_none());
    assert!(app.crash_reports.saved.is_none());
    assert_eq!(app.dialog, Some(Dialog::CrashReports));
    app.folder_picker.response = Some(Ok(Some(temp.path().join("missing").display().to_string())));
    app.export_crash_report(&ctx);
    settle(&mut app, &ctx);
    assert!(app.crash_reports.error.is_some());
    assert_eq!(app.dialog, Some(Dialog::CrashReports));
    app.folder_picker.response = Some(Ok(Some(temp.path().display().to_string())));
    click(&mut app, &ctx, "Export report");
    app.export_crash_report(&ctx); // duplicate input cannot start another writer
    settle(&mut app, &ctx);
    assert!(app.crash_reports.error.is_none());
    assert_eq!(
        std::fs::read_to_string(app.crash_reports.saved.as_ref().unwrap()).unwrap(),
        service::preview(&report).unwrap()
    );
    assert_eq!(app.jobs.len(), jobs);
    assert_eq!(app.selected, selected);
    assert_eq!(app.dialog, None);
    assert!(!service::load(&app.store.paths).unwrap().reports[0].unreviewed);
    app.check_crash_reports_on_launch(&ctx);
    settle(&mut app, &ctx);
    assert_eq!(app.dialog, None);
    app.open_crash_reports(&ctx);
    settle(&mut app, &ctx);
    assert_eq!(app.dialog, Some(Dialog::CrashReports));
    // Exercise the real egui commands without opening a browser or touching
    // the system clipboard. The issue URL must contain no report data.
    let size = egui::vec2(1080.0, 800.0);
    for label in ["Copy report", "Report an issue"] {
        frame(&mut app, &ctx, size, vec![]);
        let output = frame(&mut app, &ctx, size, vec![]);
        let pos = text_rect(&output, label).center();
        let mut commands = Vec::new();
        for pressed in [true, false] {
            let output = frame(
                &mut app,
                &ctx,
                size,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            commands.extend(output.platform_output.commands);
        }
        if label == "Copy report" {
            assert!(commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(text) if text == &service::preview(&report).unwrap())));
            assert!(
                !commands
                    .iter()
                    .any(|c| matches!(c, egui::OutputCommand::OpenUrl(_)))
            );
        } else {
            assert!(commands.iter().any(|c| matches!(c, egui::OutputCommand::OpenUrl(url) if url.url == "https://github.com/JasXTD/Download-Engine/issues/new")));
        }
    }
}

#[test]
fn export_completion_does_not_dismiss_another_dialog() {
    let (temp, mut app, ctx) = fixture();
    saved(&app);
    app.open_crash_reports(&ctx);
    settle(&mut app, &ctx);
    app.folder_picker.response = Some(Ok(Some(temp.path().display().to_string())));
    app.export_crash_report(&ctx);
    app.dialog = Some(Dialog::About);
    settle(&mut app, &ctx);
    assert_eq!(app.dialog, Some(Dialog::About));
    assert!(app.crash_reports.saved.is_some());
    assert!(app.crash_reports.error.is_none());
    assert!(service::load(&app.store.paths).unwrap().reports[0].unreviewed);
}

#[test]
fn empty_launch_is_quiet_and_report_states_fit_shared_modal() {
    for dark in [false, true] {
        for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
            let (_temp, mut app, ctx) = fixture();
            app.check_crash_reports_on_launch(&ctx);
            settle(&mut app, &ctx);
            assert_eq!(app.dialog, None);
            app.interface.appearance = if dark {
                Appearance::Dark
            } else {
                Appearance::Light
            };
            app.configure_appearance(&ctx);
            app.open_crash_reports(&ctx);
            settle(&mut app, &ctx);
            for filled in [false, true] {
                if filled {
                    saved(&app);
                    app.load_crash_reports(&ctx, false);
                    settle(&mut app, &ctx);
                }
                frame(&mut app, &ctx, size, vec![]);
                let output = frame(&mut app, &ctx, size, vec![]);
                let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                for label in [
                    "Crash reports",
                    "Export report",
                    "Refresh",
                    "Report an issue",
                    if filled {
                        "A Rust panic was captured"
                    } else {
                        "No crash reports saved"
                    },
                ] {
                    assert!(
                        bounds.contains_rect(text_rect(&output, label)),
                        "{label} at {size:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn corrupt_reports_are_visible_as_skipped_without_a_false_crash_prompt() {
    let (_temp, mut app, ctx) = fixture();
    let report = saved(&app);
    std::fs::write(
        service::directory(&app.store.paths).join(format!("report-{}.json", report.id)),
        "incomplete",
    )
    .unwrap();
    app.check_crash_reports_on_launch(&ctx);
    settle(&mut app, &ctx);
    assert_eq!(app.dialog, None);
    app.open_crash_reports(&ctx);
    settle(&mut app, &ctx);
    assert_eq!(app.crash_reports.skipped, 1);
    assert!(app.crash_reports.reports.is_empty());
}
