use super::*;
use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};
use egui::{Rect, vec2};
use std::io::Write;

fn write_event(app: &Desktop, event: Event, level: Level, component: Component) -> Record {
    let record = Record {
        time: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        version: "0.1.0".into(),
        component,
        pid: 1,
        session: Uuid::new_v4(),
        level,
        event,
        details: diagnostics::Details {
            failure: (level == Level::Error).then(|| diagnostics::Failure::message("HTTP 403")),
            ..Default::default()
        },
    };
    let root = diagnostics::directory(&app.store.paths);
    std::fs::create_dir_all(&root).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join(if component == Component::Desktop {
            "desktop.log"
        } else {
            "native-host.log"
        }))
        .unwrap();
    writeln!(file, "{}", serde_json::to_string(&record).unwrap()).unwrap();
    record
}
fn load(app: &mut Desktop, ctx: &egui::Context) {
    app.logs.request_refresh();
    app.logs.refresh(&app.store.paths, ctx);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.logs.pending.is_some() {
        app.logs.poll();
        assert!(Instant::now() < deadline, "Log snapshot did not complete");
        std::thread::yield_now();
    }
}

#[test]
fn cached_logs_retranslate_after_worker_completion_without_changing_raw_records() {
    use download_engine::i18n::{Language, scope};
    let (_temp, mut app, ctx) = fixture();
    let record = write_event(
        &app,
        Event::NativeRequestFailed,
        Level::Error,
        Component::NativeHost,
    );
    let raw = serde_json::to_string(&record).unwrap();
    load(&mut app, &ctx);
    app.logs.auto_refresh = false;
    for language in [
        Language::SimplifiedChinese,
        Language::Malay,
        Language::English,
    ] {
        let _language = scope(language);
        app.logs.update_text();
        assert!(
            app.logs
                .display
                .contains(&level_label(Level::Error).to_uppercase())
        );
        assert!(
            app.logs
                .display
                .contains(source_label(Component::NativeHost))
        );
        assert!(app.logs.display.contains("HTTP 403"));
        assert_eq!(
            serde_json::to_string(&app.logs.data.entries[0].record).unwrap(),
            raw
        );
    }
}
fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn choose_popup(app: &mut Desktop, ctx: &egui::Context, label: &str) {
    assert!(egui::Popup::is_any_open(ctx));
    let size = vec2(1080.0, 800.0);
    frame(app, ctx, size, vec![]);
    let output = frame(app, ctx, size, vec![]);
    // Choose the topmost label; the selected control can repeat the option.
    let pos = output
        .shapes
        .iter()
        .rev()
        .find_map(|shape| {
            if let egui::epaint::Shape::Text(text) = &shape.shape
                && text.galley.job.text.trim() == label
            {
                Some(Rect::from_min_size(text.pos, text.galley.size()).center())
            } else {
                None
            }
        })
        .expect("Popup option must be rendered");
    for pressed in [true, false] {
        frame(
            app,
            ctx,
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
    }
}

#[test]
fn logs_read_both_components_filter_and_copy_plain_text() {
    let (_temp, mut app, ctx) = fixture();
    write_event(&app, Event::Ready, Level::Info, Component::Desktop);
    write_event(
        &app,
        Event::NativeRequestFailed,
        Level::Error,
        Component::NativeHost,
    );
    click(&mut app, &ctx, "Logs");
    load(&mut app, &ctx);
    assert_eq!(app.logs.data.entries.len(), 2);
    let output = frame(&mut app, &ctx, vec2(860.0, 600.0), vec![]);
    assert!(
        app.logs
            .display
            .contains("ERROR | Browser connector: Browser request failed")
    );
    assert!(app.logs.display.contains("Network error  ·  HTTP 403"));
    assert!(!app.logs.display.contains("SECRET"));
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if ["Time (local)", "Level", "Source", "Event"].contains(&text.galley.job.text.as_str()))));
    click(&mut app, &ctx, "All levels");
    choose_popup(&mut app, &ctx, "Error");
    assert_eq!(app.logs.level, Some(Level::Error));
    click(&mut app, &ctx, "All sources");
    choose_popup(&mut app, &ctx, "Desktop");
    assert_eq!(app.logs.source, Some(Component::Desktop));
    let output = frame(&mut app, &ctx, vec2(860.0, 600.0), vec![]);
    text_rect(&output, "No events match your filters");
    click(&mut app, &ctx, "Clear filters");
    assert_eq!(app.logs.source, None);
    assert_eq!(app.logs.level, None);
    app.logs.search = "HTTP 403".into();
    let output = frame(&mut app, &ctx, vec2(860.0, 600.0), vec![]);
    text_rect(&output, "1 / 2 events");
    let expected = app.logs.display.clone();
    // Exercise the copy button through actual pointer input and inspect the OS
    // clipboard command emitted by egui; never touch the user's clipboard.
    let output = frame(&mut app, &ctx, vec2(1080.0, 800.0), vec![]);
    let pos = text_rect(&output, "Copy logs").center();
    let mut copied = None;
    for pressed in [true, false] {
        let output = frame(
            &mut app,
            &ctx,
            vec2(1080.0, 800.0),
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
        for command in output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                copied = Some(text);
            }
        }
    }
    assert_eq!(copied.unwrap(), expected);
}

#[test]
fn logs_refresh_can_pause_survives_errors_and_recovers() {
    let (_temp, mut app, ctx) = fixture();
    write_event(&app, Event::Ready, Level::Info, Component::Desktop);
    app.view = View::Logs;
    load(&mut app, &ctx);
    app.logs.auto_refresh = false;
    write_event(
        &app,
        Event::DownloadCompleted,
        Level::Info,
        Component::Desktop,
    );
    app.logs.checked = Some(Instant::now() - Duration::from_secs(10));
    app.logs.refresh(&app.store.paths, &ctx);
    assert!(
        app.logs.pending.is_none(),
        "Paused refresh must not start a read"
    );
    assert_eq!(app.logs.data.entries.len(), 1);
    load(&mut app, &ctx);
    assert_eq!(app.logs.data.entries.len(), 2);
    let path = diagnostics::directory(&app.store.paths).join("desktop.log");
    let valid = std::fs::read(&path).unwrap();
    std::fs::write(&path, vec![b'x'; 4097]).unwrap();
    load(&mut app, &ctx);
    assert!(app.logs.error.as_ref().unwrap().contains("size limit"));
    assert_eq!(
        app.logs.data.entries.len(),
        2,
        "Retain the last successful snapshot"
    );
    assert!(
        app.notice.is_none(),
        "Do not generate a new error log on every refresh"
    );
    std::fs::write(&path, valid).unwrap();
    load(&mut app, &ctx);
    assert!(app.logs.error.is_none());
    std::fs::write(path, b"").unwrap();
    load(&mut app, &ctx);
    app.logs.update_text();
    assert!(
        app.logs.display.is_empty(),
        "Removed records cannot remain displayed"
    );
}

#[test]
fn logs_search_shortcuts_do_not_change_download_filters_or_selection() {
    let (_temp, mut app, ctx) = fixture();
    app.search = "first".into();
    let selected = app.selected.clone();
    click(&mut app, &ctx, "Logs");
    frame(
        &mut app,
        &ctx,
        vec2(1080.0, 800.0),
        vec![key(egui::Key::F, egui::Modifiers::COMMAND)],
    );
    assert_eq!(app.view, View::Logs);
    assert!(ctx.memory(|m| m.has_focus(egui::Id::new("logs-search"))));
    frame(
        &mut app,
        &ctx,
        vec2(1080.0, 800.0),
        vec![egui::Event::Text("failure".into())],
    );
    assert_eq!(app.logs.search, "failure");
    frame(
        &mut app,
        &ctx,
        vec2(1080.0, 800.0),
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert!(app.logs.search.is_empty());
    assert_eq!(app.search, "first");
    assert_eq!(app.selected, selected);
    click(&mut app, &ctx, "All downloads");
    assert_eq!(app.view, View::Downloads);
    assert_eq!(app.search, "first");
}

#[test]
fn help_has_guide_changelog_and_about_while_logs_keep_their_own_actions() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "Help");
    let output = frame(&mut app, &ctx, vec2(860.0, 600.0), vec![]);
    text_rect(&output, "About");
    text_rect(&output, "Guide");
    text_rect(&output, "Changelog");
    for shape in &output.shapes {
        if let egui::epaint::Shape::Text(text) = &shape.shape {
            assert!(
                !["Open logs", "Export diagnostics…", "Lua guide", "MCP setup"]
                    .contains(&text.galley.job.text.as_str())
            );
        }
    }
    frame(
        &mut app,
        &ctx,
        vec2(860.0, 600.0),
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    click(&mut app, &ctx, "Tasks");
    let tasks = frame(&mut app, &ctx, vec2(860.0, 600.0), vec![]);
    text_rect(&tasks, "Exit");
    for shape in &tasks.shapes {
        if let egui::epaint::Shape::Text(text) = &shape.shape {
            assert_ne!(text.galley.job.text.as_str(), "Exit Download Engine");
        }
    }
    frame(
        &mut app,
        &ctx,
        vec2(860.0, 600.0),
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    click(&mut app, &ctx, "Logs");
    load(&mut app, &ctx);
    for appearance in [Appearance::Dark, Appearance::Light] {
        app.interface.appearance = appearance;
        app.configure_appearance(&ctx);
        let output = frame(&mut app, &ctx, vec2(860.0, 600.0), vec![]);
        text_rect(&output, "No log entries yet");
        text_rect(&output, "Local desktop and browser connector diagnostics.");
        let refresh = text_rect(&output, "Refresh");
        let folder = text_rect(&output, "Open log folder");
        let export = text_rect(&output, "Export diagnostics…");
        assert!(refresh.right() < folder.left() && folder.right() < export.left());
        assert!(export.right() < 860.0);
        let search = text_rect(&output, "Search logs…");
        for label in ["All levels", "All sources", "Auto-refresh"] {
            assert!(
                (text_rect(&output, label).center().y - search.center().y).abs() <= 1.0,
                "Search and filter text should share a baseline"
            );
        }
    }
    assert_eq!(app.diagnostics.opened, None);
}

#[test]
fn returning_to_logs_refreshes_an_in_flight_snapshot_even_with_auto_refresh_off() {
    let (_temp, mut app, ctx) = fixture();
    write_event(&app, Event::Ready, Level::Info, Component::Desktop);
    app.logs.auto_refresh = false;
    app.logs.refresh(&app.store.paths, &ctx);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !app.logs.worker.as_ref().unwrap().is_finished() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    // The UI was on another page and has not received the old snapshot yet.
    write_event(
        &app,
        Event::DownloadStarted,
        Level::Info,
        Component::Desktop,
    );
    app.logs.request_refresh();
    app.logs.refresh(&app.store.paths, &ctx);
    assert!(
        app.logs.pending.is_some(),
        "Returning must start a fresh read"
    );
    while app.logs.pending.is_some() {
        assert!(Instant::now() < deadline);
        app.logs.poll();
        std::thread::yield_now();
    }
    assert_eq!(app.logs.data.entries.len(), 2);
}

#[test]
fn log_text_is_selectable_read_only_and_does_not_change_during_selection() {
    let (_temp, mut app, ctx) = fixture();
    write_event(&app, Event::Ready, Level::Info, Component::Desktop);
    app.view = View::Logs;
    load(&mut app, &ctx);
    frame(&mut app, &ctx, vec2(1080.0, 800.0), vec![]);
    let original = app.logs.display.clone();
    ctx.memory_mut(|m| m.request_focus(egui::Id::new("logs-text")));
    frame(
        &mut app,
        &ctx,
        vec2(1080.0, 800.0),
        vec![key(egui::Key::A, egui::Modifiers::COMMAND)],
    );
    let output = frame(&mut app, &ctx, vec2(1080.0, 800.0), vec![egui::Event::Copy]);
    assert!(output.platform_output.commands.iter().any(
        |command| matches!(command, egui::OutputCommand::CopyText(text) if text == &original)
    ));
    frame(
        &mut app,
        &ctx,
        vec2(1080.0, 800.0),
        vec![
            egui::Event::Text("Do not edit logs".into()),
            key(egui::Key::Delete, egui::Modifiers::NONE),
        ],
    );
    assert_eq!(app.logs.display, original);
    write_event(
        &app,
        Event::DownloadCompleted,
        Level::Info,
        Component::Desktop,
    );
    app.logs.checked = Some(Instant::now() - Duration::from_secs(10));
    frame(&mut app, &ctx, vec2(1080.0, 800.0), vec![]);
    assert_eq!(
        app.logs.display, original,
        "Refresh must not replace selected text"
    );
    assert!(app.logs.pending.is_none());
    ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("logs-text")));
    load(&mut app, &ctx);
    frame(&mut app, &ctx, vec2(1080.0, 800.0), vec![]);
    assert!(app.logs.display.contains("Download completed"));
    assert!(app.logs.display.contains("Ready"));
}

#[test]
fn explicit_refresh_updates_scrolled_text_while_automatic_refresh_is_paused() {
    let (_temp, mut app, ctx) = fixture();
    write_event(&app, Event::Ready, Level::Info, Component::Desktop);
    load(&mut app, &ctx);
    app.logs.update_text();
    let original = app.logs.display.clone();
    app.logs.scrolled_away = true;
    app.logs.checked = Some(Instant::now() - Duration::from_secs(10));
    write_event(
        &app,
        Event::DownloadCompleted,
        Level::Info,
        Component::Desktop,
    );
    app.logs.refresh(&app.store.paths, &ctx);
    assert!(app.logs.pending.is_none());
    app.logs.request_refresh();
    app.logs.refresh(&app.store.paths, &ctx);
    assert!(app.logs.pending.is_some());
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.logs.pending.is_some() {
        assert!(Instant::now() < deadline);
        app.logs.refresh(&app.store.paths, &ctx);
        std::thread::yield_now();
    }
    app.logs.update_text();
    assert_ne!(app.logs.display, original);
    assert!(app.logs.display.contains("Download completed"));
}
