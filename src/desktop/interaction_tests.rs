use super::*;
use download_engine::paths::AppPaths;

#[test]
fn language_picker_saves_explicitly_and_preserves_download_state() {
    use download_engine::i18n::Language;
    let (_temp, mut app, ctx) = fixture();
    // Preferences may refresh the real store while this layout fixture is open.
    for job in &app.jobs {
        app.store.enqueue(job).unwrap();
    }
    app.refresh_and_wait();
    let names: Vec<_> = app
        .jobs
        .iter()
        .map(|job| (job.id.clone(), job.name.clone(), job.url.clone()))
        .collect();
    let selected = app.selected.clone();
    app.open_preferences_window();
    click(&mut app, &ctx, "English");
    click(&mut app, &ctx, "简体中文");
    assert_eq!(app.interface.language, Language::English);
    click(&mut app, &ctx, "Save");
    assert_eq!(app.interface.language, Language::SimplifiedChinese);
    assert_eq!(
        app.store.interface_preferences().unwrap().language,
        Language::SimplifiedChinese
    );
    let out = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
    text_rect(&out, "更多");
    app.open_preferences_window();
    click(&mut app, &ctx, "简体中文");
    click(&mut app, &ctx, "Bahasa Melayu");
    click(&mut app, &ctx, "保存");
    assert_eq!(
        app.store.interface_preferences().unwrap().language,
        Language::Malay
    );
    let out = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
    text_rect(&out, "Lagi");
    assert_eq!(
        names,
        app.jobs
            .iter()
            .map(|job| (job.id.clone(), job.name.clone(), job.url.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(app.selected, selected);
}

#[test]
fn localized_pages_and_shared_dialogs_fit_both_window_sizes() {
    use download_engine::i18n::Language;
    let (_temp, mut app, ctx) = fixture();
    for language in [
        Language::English,
        Language::SimplifiedChinese,
        Language::TraditionalChinese,
        Language::Malay,
    ] {
        app.interface.language = language;
        let _scope = download_engine::i18n::scope(language);
        for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
            for view in [View::Downloads, View::Queues, View::Guide] {
                app.view = view;
                frame(&mut app, &ctx, size, vec![]);
                let out = frame(&mut app, &ctx, size, vec![]);
                for clipped in &out.shapes {
                    if let egui::Shape::Text(text) = &clipped.shape {
                        let rect = text.galley.rect.translate(text.pos.to_vec2());
                        if matches!(language, Language::English | Language::Malay)
                            && rect.top() < 108.0
                            && [tr("Scripting"), tr("Grabber")]
                                .contains(&text.galley.job.text.as_str())
                        {
                            assert_eq!(
                                text.galley.rows.len(),
                                1,
                                "Toolbar word split: {}",
                                text.galley.job.text
                            );
                        }
                        if rect.intersects(clipped.clip_rect) {
                            assert!(
                                rect.right() <= size.x + 1.0,
                                "{language:?} {view:?}: horizontal overflow: {}",
                                text.galley.job.text
                            );
                        }
                    }
                }
            }
            app.open_queue_dialog(Dialog::NewQueue);
            frame(&mut app, &ctx, size, vec![]);
            let out = frame(&mut app, &ctx, size, vec![]);
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                    .contains_rect(text_rect(&out, tr("Create queue")))
            );
            app.dialog = None;
        }
        app.open_guide("Lua scripting");
        assert_eq!(
            app.guide_topic, 6,
            "Localized topic must keep its stable ID"
        );
    }
    #[cfg(windows)]
    ctx.fonts_mut(|fonts| {
        assert!(fonts.has_glyphs(
            &egui::FontId::proportional(14.0),
            "简体中文繁體中文下載設定佇列"
        ));
        assert!(fonts.has_glyphs(&egui::FontId::monospace(12.0), "日志記錄警告错误錯誤"));
    });
}

pub(super) fn fixture() -> (tempfile::TempDir, Desktop, egui::Context) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let lock = File::create(temp.path().join("ui.lock")).unwrap();
    let mut app = Desktop::new(store, scheduler, lock);
    app.interface.language = download_engine::i18n::Language::English;
    for (name, status) in [
        ("first.zip", Status::Paused),
        ("guide.pdf", Status::Complete),
        ("failed.zip", Status::Failed),
    ] {
        let mut job = Job::new(
            &format!("https://example.com/{name}"),
            None,
            temp.path().join("downloads"),
            Uuid::new_v4().to_string(),
        )
        .unwrap();
        job.status = status;
        job.total = Some(4096);
        job.downloaded = if status == Status::Complete {
            4096
        } else {
            1024
        };
        if status == Status::Failed {
            job.error = Some("Server returned HTTP 403".into());
        }
        app.jobs.push(job);
    }
    app.reconcile_selection();
    let ctx = egui::Context::default();
    configure(&ctx);
    ctx.all_styles_mut(|style| style.animation_time = 0.0);
    (temp, app, ctx)
}

pub(super) fn frame(
    app: &mut Desktop,
    ctx: &egui::Context,
    size: egui::Vec2,
    mut events: Vec<egui::Event>,
) -> egui::FullOutput {
    let modifiers = events
        .iter()
        .rev()
        .find_map(|e| match e {
            egui::Event::Key { modifiers, .. } | egui::Event::PointerButton { modifiers, .. } => {
                Some(*modifiers)
            }
            _ => None,
        })
        .unwrap_or_default();
    events.insert(0, egui::Event::ModifiersChanged(modifiers));
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    // This harness inspects layout and input; it does not upload GPU textures.
    output.textures_delta.clear();
    output
}

pub(super) fn text_rect(output: &egui::FullOutput, label: &str) -> egui::Rect {
    if label == "×" {
        return topmost_text_rect(output, label);
    }
    output
        .shapes
        .iter()
        .find_map(|clipped| {
            if let egui::epaint::Shape::Text(text) = &clipped.shape
                && text.galley.job.text.trim() == label
            {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("No rendered text: {label}"))
}

fn topmost_text_rect(output: &egui::FullOutput, label: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .rev()
        .find_map(|clipped| {
            if let egui::epaint::Shape::Text(text) = &clipped.shape
                && text.galley.job.text.trim() == label
            {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("No rendered text: {label}"))
}

pub(super) fn click(app: &mut Desktop, ctx: &egui::Context, label: &str) {
    app.wait_for_io();
    let size = egui::vec2(1080.0, 800.0);
    // Modal areas and panel sizing need a settled frame before hit testing.
    frame(app, ctx, size, vec![]);
    let mut output = frame(app, ctx, size, vec![]);
    let has_label = |out: &egui::FullOutput, label: &str| {
        out.shapes.iter().any(|shape|
        matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.trim() == label))
    };
    // Follow the same overflow navigation as a person using the compact toolbar.
    if app.dialog.is_none()
        && !egui::Popup::is_any_open(ctx)
        && !has_label(&output, label)
        && [
            "Remove",
            "Open folder",
            "Clear completed history",
            "Queues",
            "Scheduler",
            "Scripting",
            "Grabber",
            "MCP",
            "Logs",
            "Preferences",
        ]
        .iter()
        .any(|key| {
            let _scope = download_engine::i18n::scope(app.interface.language);
            tr(key) == label
        })
    {
        let more = {
            let _scope = download_engine::i18n::scope(app.interface.language);
            tr("More")
        };
        click(app, ctx, more);
        output = frame(app, ctx, size, vec![]);
    }
    // A modal can share labels (such as ×) with controls covered by its backdrop.
    let pos = if app.dialog.is_some() || egui::Popup::is_any_open(ctx) {
        topmost_text_rect(&output, label).center()
    } else {
        text_rect(&output, label).center()
    };
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
    app.wait_for_io();
}

#[test]
fn inspector_drag_saves_only_on_release_and_restores_its_height() {
    let (_root, mut app, ctx) = fixture();
    let size = egui::vec2(1080.0, 800.0);
    frame(&mut app, &ctx, size, vec![]);
    let rect = egui::containers::panel::PanelState::load(&ctx, egui::Id::new("file-details"))
        .unwrap()
        .outer_rect;
    let start = rect.center_top();
    let end = start - egui::vec2(0.0, 60.0);
    frame(
        &mut app,
        &ctx,
        size,
        vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    frame(&mut app, &ctx, size, vec![egui::Event::PointerMoved(end)]);
    assert_eq!(
        app.store.interface_preferences().unwrap().details_height,
        None
    );
    frame(
        &mut app,
        &ctx,
        size,
        vec![
            egui::Event::PointerMoved(end),
            egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let saved = app
        .store
        .interface_preferences()
        .unwrap()
        .details_height
        .expect("Released resize persisted");
    assert!((f32::from(saved) - rect.height() - 60.0).abs() <= 2.0);
    let fresh = egui::Context::default();
    super::configure(&fresh);
    frame(&mut app, &fresh, size, vec![]);
    let restored = egui::containers::panel::PanelState::load(&fresh, egui::Id::new("file-details"))
        .unwrap()
        .outer_rect;
    assert!((restored.height() - f32::from(saved)).abs() <= 1.0);
}

#[test]
fn tabs_and_filename_clicks_update_the_selected_file() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "Completed");
    assert_eq!(app.filter, Filter::Complete);
    assert_eq!(app.selected.as_deref(), Some(app.jobs[1].id.as_str()));
    click(&mut app, &ctx, "All downloads");
    click(&mut app, &ctx, "failed.zip");
    assert_eq!(app.selected.as_deref(), Some(app.jobs[2].id.as_str()));
    let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
    text_rect(&output, "Server returned HTTP 403");
}

#[test]
fn about_card_fits_both_themes_and_leaves_the_selected_download_unchanged() {
    let build_version = env!("CARGO_PKG_VERSION");
    let version = format!("Version {build_version}");
    for dark in [false, true] {
        for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
            let (_temp, mut app, ctx) = fixture();
            app.interface.appearance = if dark {
                Appearance::Dark
            } else {
                Appearance::Light
            };
            app.configure_appearance(&ctx);
            let selected = app.selected.clone();
            click(&mut app, &ctx, "Help");
            click(&mut app, &ctx, "About");
            assert_eq!(app.dialog, Some(Dialog::About));
            frame(&mut app, &ctx, size, vec![]);
            let output = frame(&mut app, &ctx, size, vec![]);
            for label in [
                "Download Engine",
                &version,
                "Created by Xetro",
                "GitHub",
                "Legal & licenses",
            ] {
                let rect = topmost_text_rect(&output, label);
                assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect));
                assert!(
                    output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::epaint::Shape::Text(text) if text.galley.job.text.trim() == label
                        && shape.clip_rect.contains_rect(rect))),
                    "About text clipped: {label}"
                );
            }
            let title = topmost_text_rect(&output, "Download Engine");
            let close = ctx
                .read_response(egui::Id::new("about-close"))
                .unwrap()
                .rect;
            assert!(close.width() >= 32.0 && close.height() >= 32.0);
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(close));
            assert!(close.left() > title.right());
            assert!((close.center().y - title.center().y).abs() < 4.0);
            assert!(text_rect(&output, &version).top() > title.bottom());
            assert_eq!(
                text_rect(&output, "Created by Xetro").left(),
                text_rect(&output, "GitHub").left()
            );
            assert!(text_rect(&output, "Created by Xetro").left() < title.left());
            assert!(
                text_rect(&output, "Created by Xetro").top()
                    > text_rect(&output, &version).bottom()
            );

            // The repository link opens the official project without dismissing the card.
            let pos = text_rect(&output, "GitHub").center();
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
                if !pressed {
                    assert!(output.platform_output.commands.iter().any(|command|
                        matches!(command, egui::OutputCommand::OpenUrl(url) if url.url == download_engine::updates::REPOSITORY_URL)));
                }
            }
            assert_eq!(app.dialog, Some(Dialog::About));
            for pressed in [true, false] {
                frame(
                    &mut app,
                    &ctx,
                    size,
                    vec![
                        egui::Event::PointerMoved(close.center()),
                        egui::Event::PointerButton {
                            pos: close.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert_eq!(app.dialog, None);
            // Render the closed state so egui releases the previous modal's focus.
            frame(&mut app, &ctx, size, vec![egui::Event::PointerGone]);
            assert!(!ctx.memory(|memory| memory.has_focus(egui::Id::new("about-close"))));
            for key in [egui::Key::Enter, egui::Key::Space] {
                app.dialog = Some(Dialog::About);
                frame(&mut app, &ctx, size, vec![egui::Event::PointerGone]);
                frame(
                    &mut app,
                    &ctx,
                    size,
                    vec![egui::Event::Key {
                        key: egui::Key::Tab,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                frame(
                    &mut app,
                    &ctx,
                    size,
                    vec![egui::Event::Key {
                        key: egui::Key::Tab,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                assert!(
                    ctx.read_response(egui::Id::new("about-close"))
                        .unwrap()
                        .has_focus()
                );
                frame(
                    &mut app,
                    &ctx,
                    size,
                    vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                assert_eq!(app.dialog, None);
                frame(
                    &mut app,
                    &ctx,
                    size,
                    vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
            app.dialog = Some(Dialog::About);
            frame(&mut app, &ctx, size, vec![]);
            frame(
                &mut app,
                &ctx,
                size,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert_eq!(app.dialog, None);
            assert_eq!(app.selected, selected);
        }
    }
}

#[test]
fn toolbar_pages_remain_reachable_without_changing_the_download_selection() {
    let (_temp, mut app, ctx) = fixture();
    let selected = app.selected.clone();
    let queue = app.queue_id.clone();
    for appearance in [Appearance::Dark, Appearance::Light] {
        app.interface.appearance = appearance;
        app.configure_appearance(&ctx);
        for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
            for view in [
                View::Downloads,
                View::Automation,
                View::Logs,
                View::Guide,
                View::Changelog,
            ] {
                app.view = view;
                frame(&mut app, &ctx, size, vec![]);
                let output = frame(&mut app, &ctx, size, vec![]);
                let label = if view == View::Downloads {
                    "Add URL"
                } else {
                    "Download list"
                };
                let row = text_rect(&output, label);
                assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(row));
                let more = text_rect(&output, "More");
                assert!(more.left() > row.right());
                assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(more));
                assert_eq!(app.selected, selected);
            }
        }
    }
    for labels in [true, false] {
        app.interface.sidebar_labels = labels;
        click(&mut app, &ctx, "Queues");
        assert_eq!(app.view, View::Queues);
        click(&mut app, &ctx, "Scripting");
        assert_eq!(app.view, View::Automation);
        click(&mut app, &ctx, "Grabber");
        assert_eq!(app.dialog, Some(Dialog::Grabber));
        assert!(app.grabber.is_some());
        assert_eq!(app.view, View::Automation);
        click(&mut app, &ctx, "×");
        click(&mut app, &ctx, "Logs");
        assert_eq!(app.view, View::Logs);
        click(&mut app, &ctx, "Preferences");
        assert_eq!(app.view, View::Logs);
        assert_eq!(app.dialog, Some(Dialog::PreferencesWindow));
        click(&mut app, &ctx, "Cancel");
        click(&mut app, &ctx, "Scripting");
        assert_eq!(app.view, View::Automation);
        assert_eq!(app.selected, selected);
        assert_eq!(app.queue_id, queue);
        assert!(app.dialog.is_none());
    }
    app.interface.sidebar_labels = true;
    click(&mut app, &ctx, "Download list");
    assert_eq!(app.view, View::Downloads);
    assert_eq!(app.selected, selected);
}

#[test]
fn logs_navigation_stays_in_app_and_export_cancellation_keeps_the_queue_unchanged() {
    let (temp, mut app, ctx) = fixture();
    let selected = app.selected.clone();
    click(&mut app, &ctx, "Logs");
    assert_eq!(app.view, View::Logs);
    assert_eq!(app.diagnostics.opened, None);
    assert!(app.dialog.is_none());
    let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
    text_rect(&output, "Download list");
    assert!(
        !output.shapes.iter().any(
            |s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == "Manage queues")
        )
    );
    click(&mut app, &ctx, "Open log folder");
    assert_eq!(
        app.diagnostics.opened,
        Some(app.store.paths.data.join("logs"))
    );
    assert!(app.dialog.is_none());
    app.folder_picker.response = Some(Ok(None));
    click(&mut app, &ctx, "Export diagnostics…");
    assert!(!app.diagnostics.busy());
    assert_eq!(app.selected, selected);
    assert!(!std::fs::read_dir(temp.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|extension| extension == "json")
    }));
}

#[test]
fn diagnostics_export_errors_are_visible_and_allow_retry() {
    let (_temp, mut app, _ctx) = fixture();
    app.folder_picker.response = Some(Ok(Some(app.store.paths.downloads.display().to_string())));
    app.export_diagnostics();
    assert!(app.diagnostics.busy());
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.diagnostics.busy() {
        app.poll_diagnostic_export();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(matches!(
        app.notice.as_ref().unwrap().kind,
        interactions::NoticeKind::Error
    ));
    assert!(
        app.notice
            .as_ref()
            .unwrap()
            .message
            .contains("No diagnostic logs")
    );
    app.folder_picker.response = Some(Ok(None));
    app.export_diagnostics();
    assert!(!app.diagnostics.busy());
}

#[test]
fn diagnostics_export_finishes_once_and_stays_in_the_logs_view() {
    let (temp, mut app, ctx) = fixture();
    let log_dir = app.store.paths.data.join("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let entry = serde_json::json!({"time":"2026-10-14T00:00:00.000Z","version":"0.1.0",
        "component":"desktop","pid":1,"session":Uuid::new_v4(),"level":"info","event":"ready","details":{}});
    std::fs::write(log_dir.join("desktop.log"), format!("{entry}\n")).unwrap();
    let selected = app.selected.clone();
    app.folder_picker.response = Some(Ok(Some(temp.path().display().to_string())));
    click(&mut app, &ctx, "Logs");
    click(&mut app, &ctx, "Export diagnostics…");
    assert!(app.diagnostics.busy());
    app.export_diagnostics(); // repeated input must not start a second writer
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.diagnostics.busy() {
        app.poll_diagnostic_export();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(app.diagnostics.opened, None);
    assert_eq!(app.view, View::Logs);
    assert_eq!(app.selected, selected);
    assert!(matches!(
        app.notice.as_ref().unwrap().kind,
        interactions::NoticeKind::Success
    ));
    let exports: Vec<_> = std::fs::read_dir(temp.path())
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("DownloadEngine-diagnostics-")
        })
        .collect();
    assert_eq!(exports.len(), 1);
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(exports[0].path()).unwrap()).unwrap();
    assert_eq!(report["records"][0]["event"], "ready");
}

#[test]
fn toolbar_clear_finished_keeps_files_and_pause_all_covers_other_queues() {
    let (temp, mut app, ctx) = fixture();
    app.store.set_queue_running(MAIN_QUEUE, false).unwrap();
    let second = app.store.create_queue("Second queue").unwrap();
    app.store.set_queue_running(&second.id, true).unwrap();
    let mut complete = app.jobs[1].clone();
    let saved = temp.path().join("saved-guide.pdf");
    std::fs::write(&saved, b"keep this file").unwrap();
    complete.saved_path = Some(saved.clone());
    app.store.enqueue(&complete).unwrap();
    app.store.enqueue(&app.jobs[0]).unwrap();
    app.refresh_and_wait();
    click(&mut app, &ctx, "Clear completed history");
    click(&mut app, &ctx, "Tasks");
    click(&mut app, &ctx, "Pause all");
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.store.get(&complete.id).unwrap().is_some()
        || app.store.queues().unwrap().iter().any(|q| q.running)
    {
        assert!(Instant::now() < deadline, "Toolbar commands did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(std::fs::read(saved).unwrap(), b"keep this file");
    assert_eq!(app.store.list().unwrap().len(), 1);
    app.refresh_and_wait();
    assert!(
        app.jobs
            .iter()
            .any(|job| app.selected.as_deref() == Some(&job.id))
    );
}

#[test]
fn unfinished_filter_and_empty_selection_keep_toolbar_actions_safe() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "Unfinished");
    assert_eq!(app.jobs.iter().filter(|job| app.accepts(job)).count(), 2);
    click(&mut app, &ctx, "Downloading");
    assert_eq!(app.selected, None);
    for label in ["Resume", "Pause", "Stop", "Remove", "Open folder"] {
        click(&mut app, &ctx, label);
    }
    assert_eq!(app.dialog, None);
    assert!(app.notice.is_none());
    assert_eq!(app.jobs.len(), 3);
    assert!(app.store.list().unwrap().is_empty());
    click(&mut app, &ctx, "All downloads");
    assert!(app.selected.is_some());
}

#[test]
fn removing_or_filtering_the_selected_file_never_leaves_stale_details() {
    let (_temp, mut app, _ctx) = fixture();
    app.jobs.remove(0);
    app.reconcile_selection();
    assert_eq!(app.selected.as_deref(), Some(app.jobs[0].id.as_str()));
    app.filter = Filter::Complete;
    app.jobs.retain(|job| job.status != Status::Complete);
    app.reconcile_selection();
    assert_eq!(app.selected, None);
}

#[test]
fn add_dialog_keeps_validation_errors_and_draft_in_the_dialog() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "Add URL");
    assert!(app.dialog == Some(Dialog::Add));
    app.url = "sftp://example.com/file.zip".into();
    click(&mut app, &ctx, "Start download");
    assert!(app.dialog == Some(Dialog::Add));
    assert!(app.form_error.as_deref().unwrap().contains("HTTP"));
    assert!(app.store.list().unwrap().is_empty());
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none());
    assert_eq!(app.url, "sftp://example.com/file.zip");
    click(&mut app, &ctx, "Add URL");
    assert_eq!(app.url, "sftp://example.com/file.zip");
    assert!(app.form_error.is_none());
}

#[test]
fn ftp_dialogs_select_tls_mode_require_protected_login_and_fit_small_windows() {
    let (_temp, mut app, ctx) = fixture();
    let size = egui::vec2(860.0, 600.0);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(theme);
        app.url = "ftps://example.com/archive.zip".into();
        app.open_add();
        app.add_authentication = true;
        for _ in 0..2 {
            frame(&mut app, &ctx, size, vec![]);
        }
        let output = frame(&mut app, &ctx, size, vec![]);
        for label in [
            "Add download",
            "Start download",
            "×",
            download_engine::ftp::Security::Implicit.label(),
        ] {
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                    .contains_rect(text_rect(&output, label)),
                "{label}"
            );
        }
    }
    app.add_ftp_security = Some(download_engine::ftp::Security::Explicit);
    // Check TLS and existing per-file login actions without creating a live transfer.
    let mut job = app.prepare_add_download().unwrap();
    job.status = Status::Paused;
    app.store.enqueue_batch(&[job], MAIN_QUEUE).unwrap();
    let saved = app.store.list().unwrap().pop().unwrap();
    assert_eq!(saved.url, "ftps://example.com:21/archive.zip");
    assert_eq!(
        saved.ftp.as_ref().unwrap().security,
        download_engine::ftp::Security::Explicit
    );
    assert_eq!(saved.status, Status::Paused);
    app.open_authentication(&saved);
    assert!(app.dialog == Some(Dialog::Authentication));
    for _ in 0..2 {
        frame(&mut app, &ctx, size, vec![]);
    }
    let output = frame(&mut app, &ctx, size, vec![]);
    for label in ["Username / password", "None", "Save and continue", "×"] {
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                .contains_rect(text_rect(&output, label))
        );
    }
    assert!(!output.shapes.iter().any(|s|matches!(&s.shape,egui::epaint::Shape::Text(t) if t.galley.job.text.contains("Bearer token"))));
    assert_eq!(resume_action(&saved), "Resume");
    app.authentication = None;
    app.dialog = None;
    app.duplicate_download(&ctx, &saved);
    let copy = app
        .store
        .list()
        .unwrap()
        .into_iter()
        .find(|j| j.id != saved.id)
        .unwrap();
    assert_eq!(
        copy.ftp.as_ref().unwrap().security,
        download_engine::ftp::Security::Explicit
    );
    app.url = "ftp://example.com/public.zip".into();
    app.add_authentication = true;
    app.open_add();
    frame(&mut app, &ctx, size, vec![]);
    assert!(!app.add_authentication);
    app.add();
    app.wait_for_io();
    let plain = app
        .store
        .list()
        .unwrap()
        .into_iter()
        .find(|j| j.url.starts_with("ftp:"))
        .unwrap();
    assert!(!supports_authentication(&plain));
    app.open_authentication(&plain);
    assert!(app.authentication.is_none());
    assert!(app.notice.as_ref().unwrap().contains("require FTPS"));
}

#[test]
fn browser_setup_and_escape_do_not_change_the_download_filter() {
    let (_temp, mut app, ctx) = fixture();
    app.filter = Filter::Complete;
    app.reconcile_selection();
    click(&mut app, &ctx, "Tools");
    let menu = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
    text_rect(&menu, "Browser setup");
    assert!(!menu.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::epaint::Shape::Text(text) if text.galley.job.text == "Browser integration")));
    click(&mut app, &ctx, "Browser setup");
    assert!(app.dialog == Some(Dialog::Browsers));
    let size = egui::vec2(860.0, 600.0);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(theme);
        frame(&mut app, &ctx, size, vec![]);
        let output = frame(&mut app, &ctx, size, vec![]);
        for label in [
            "Browser integration",
            "Chrome",
            "Edge",
            "Brave",
            "Firefox",
            "1. Load the extension",
            "2. Connect to Download Engine",
            "3. Check the connection",
        ] {
            let rect = text_rect(&output, label);
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect));
            assert!(
                output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::epaint::Shape::Text(text)
                    if text.galley.job.text.trim() == label && shape.clip_rect.contains_rect(rect))
                }),
                "Browser instructions clipped: {label}"
            );
        }
    }
    frame(
        &mut app,
        &ctx,
        egui::vec2(1080.0, 800.0),
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(app.dialog.is_none());
    assert_eq!(app.filter, Filter::Complete);
}

#[test]
fn browser_setup_validates_the_id_and_preserves_drafts_between_browsers() {
    let (_temp, mut app, ctx) = fixture();
    app.dialog = Some(Dialog::Browsers);
    app.browser_status = Some(RegistrationStatus::Missing);
    app.browser_ids[0] = "not-an-extension-id".into();
    click(&mut app, &ctx, "Connect browser");
    assert!(app.browser_notice.as_ref().unwrap().0);
    assert!(app.browser_notice.as_ref().unwrap().1.contains("32-letter"));
    click(&mut app, &ctx, "Edge");
    assert_eq!(app.browser, Browser::Edge);
    click(&mut app, &ctx, "Chrome");
    assert_eq!(app.browser_ids[0], "not-an-extension-id");
    // Brave has its own ID draft, separate from Chrome's.
    click(&mut app, &ctx, "Brave");
    assert_eq!(app.browser, Browser::Brave);
    app.browser_ids[2] = "brave-draft".into();
    let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
    text_rect(&output, "brave://extensions");
    text_rect(&output, "brave-draft");
    text_rect(
        &output,
        "This extension can only be used in Developer mode.",
    );
    click(&mut app, &ctx, "Chrome");
    assert_eq!(app.browser_ids[0], "not-an-extension-id");
    assert_eq!(app.browser_ids[2], "brave-draft");
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none());
}

#[test]
fn browser_setup_explains_the_filled_in_id_and_flags_a_different_connection() {
    let (_temp, mut app, ctx) = fixture();
    let folder_id = "ejdcchcgambklhfegfjioigmbdomalgk";
    let connected = "aldojdbffhahojhmbkbnjhfmbjoidfpg";
    app.dialog = Some(Dialog::Browsers);
    app.browser = Browser::Brave;
    app.browser_ids[2] = folder_id.into();
    app.browser_expected = Some(folder_id.into());
    app.browser_status = Some(RegistrationStatus::Registered {
        extension_id: connected.into(),
    });
    frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
    let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
    text_rect(
        &output,
        "Filled in from this copy's extension folder. If you loaded the extension from another folder, paste the ID your browser shows.",
    );
    text_rect(
        &output,
        &format!(
            "Connected to a different extension ID ({connected}). If your browser shows the ID above, click Connect browser to update it."
        ),
    );
    // Connected with this copy's own folder ID: no warning.
    app.browser_status = Some(RegistrationStatus::Registered {
        extension_id: folder_id.into(),
    });
    let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::epaint::Shape::Text(text) if text.galley.job.text.starts_with("Connected to a different"))));
}

#[test]
fn table_actions_fit_the_minimum_window_in_both_appearances() {
    let (_temp, mut app, ctx) = fixture();
    let size = egui::vec2(860.0, 600.0);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(theme);
        frame(&mut app, &ctx, size, vec![]);
        let output = frame(&mut app, &ctx, size, vec![]);
        for label in [
            "File name",
            "Status",
            "Size",
            "Speed",
            "Actions",
            "Start again",
            "Remove",
            "Paused",
            "More",
        ] {
            let rect = text_rect(&output, label);
            let shape = output.shapes.iter().find(|clipped| matches!(&clipped.shape, egui::epaint::Shape::Text(t) if t.galley.job.text.trim() == label)).unwrap();
            assert!(
                shape.clip_rect.expand(1.0).contains_rect(rect),
                "Clipped {label}"
            );
            assert!(
                rect.left() >= 0.0
                    && rect.right() <= size.x
                    && rect.top() >= 0.0
                    && rect.bottom() <= size.y,
                "{label} is outside the window: {rect:?}"
            );
        }
        for label in ["first.zip", "\u{e768}"] {
            let clipped = output.shapes.iter().rfind(|clipped| matches!(&clipped.shape, egui::epaint::Shape::Text(t) if t.galley.job.text.trim() == label)).unwrap();
            let egui::epaint::Shape::Text(text) = &clipped.shape else {
                unreachable!()
            };
            assert!(
                clipped
                    .clip_rect
                    .expand(1.0)
                    .contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size())),
                "Clipped first row {label}"
            );
        }
    }
}
#[test]
fn view_status_filter_returns_to_downloads_and_keeps_the_queue_and_search() {
    let (_temp, mut app, ctx) = fixture();
    let queue = app.queue_id.clone();
    click(&mut app, &ctx, "View");
    let menu = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
    for shape in &menu.shapes {
        if let egui::epaint::Shape::Text(text) = &shape.shape {
            assert!(
                ![
                    "Status filter",
                    "Show details",
                    "Compact rows",
                    "Table columns"
                ]
                .contains(&text.galley.job.text.as_str())
            );
        }
    }
    frame(
        &mut app,
        &ctx,
        egui::vec2(1080.0, 800.0),
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    app.search = "zip".into();
    for filter in [
        Filter::Failed,
        Filter::Queued,
        Filter::Complete,
        Filter::Unfinished,
        Filter::Downloading,
        Filter::All,
    ] {
        app.view = View::Queues;
        click(&mut app, &ctx, filter.title());
        assert_eq!(app.filter, filter);
        assert_eq!(app.view, View::Downloads);
        assert_eq!(app.queue_id, queue);
        assert_eq!(app.search, "zip");
        if filter == Filter::Failed {
            assert_eq!(app.selected.as_deref(), Some(app.jobs[2].id.as_str()));
        }
    }
}

#[test]
fn new_queue_and_batch_dialogs_save_to_the_selected_queue() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "Tasks");
    click(&mut app, &ctx, "New queue");
    app.queue_name = "Later".into();
    click(&mut app, &ctx, "Create queue");
    assert!(app.dialog.is_none());
    assert_eq!(app.current_queue_name(), "Later");
    assert!(app.jobs.is_empty());
    assert!(
        !app.queues
            .iter()
            .find(|q| q.id == app.queue_id)
            .unwrap()
            .running
    );
    click(&mut app, &ctx, "Tasks");
    click(&mut app, &ctx, "Add batch");
    app.batch = "https://example.com/a.zip\nsftp://example.com/b.zip".into();
    click(&mut app, &ctx, "Add files");
    assert!(app.dialog == Some(Dialog::Batch));
    assert!(app.form_error.as_deref().unwrap().contains("Line 2"));
    assert!(app.store.list().unwrap().is_empty());
    assert!(!app.batch.is_empty());
    app.batch = "https://example.com/a.zip\nhttps://example.com/b.zip".into();
    click(&mut app, &ctx, "Add files");
    assert!(app.dialog.is_none());
    assert_eq!(app.jobs.len(), 2);
    assert!(app.jobs.iter().all(|j| j.status == Status::Queued));
    assert!(app.store.queue_jobs(MAIN_QUEUE).unwrap().is_empty());
    click(&mut app, &ctx, "Add URL");
    app.url = "https://example.com/c.zip".into();
    click(&mut app, &ctx, "Add to queue");
    assert_eq!(app.jobs.len(), 3);
}

#[test]
fn recurring_schedule_form_validates_days_and_window_and_keeps_drafts() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "Scheduler");
    click(&mut app, &ctx, "Selected days");
    app.schedule_days = [false; 7];
    app.schedule_time = "23:00".into();
    app.schedule_stop = "07:00".into();
    click(&mut app, &ctx, "Set schedule");
    assert!(app.form_error.as_ref().unwrap().contains("start day"));
    assert!(app.dialog == Some(Dialog::Schedule));
    click(&mut app, &ctx, "Mon");
    app.schedule_stop = "23:00".into();
    click(&mut app, &ctx, "Set schedule");
    assert!(app.form_error.as_ref().unwrap().contains("different"));
    assert_eq!(app.schedule_time, "23:00");
    app.schedule_stop = "07:00".into();
    let draft = app.recurring_schedule_draft().unwrap();
    assert_eq!(draft.days, 1);
    assert!(draft.overnight());
    click(&mut app, &ctx, "Daily");
    assert_eq!(app.recurring_schedule_draft().unwrap().days, EVERY_DAY);
    click(&mut app, &ctx, "×");
    assert!(!app.store.queues().unwrap()[0].has_schedule());
}

#[test]
fn recurring_schedule_form_saves_and_reopens_while_fitting_the_small_window() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "Scheduler");
    click(&mut app, &ctx, "Selected days");
    app.schedule_days = [true, false, true, false, true, false, false];
    app.schedule_time = "23:00".into();
    app.schedule_stop = "07:00".into();
    let size = egui::vec2(860.0, 600.0);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(theme);
        frame(&mut app, &ctx, size, vec![]);
        let output = frame(&mut app, &ctx, size, vec![]);
        for label in [
            "Start on",
            "Mon",
            "Start (HH:MM)",
            "Pause (HH:MM)",
            "Set schedule",
            "×",
        ] {
            let rect = text_rect(&output, label);
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                "{label}: {rect:?}"
            );
        }
    }
    click(&mut app, &ctx, "Set schedule");
    assert!(app.dialog.is_none());
    let deadline = Instant::now() + Duration::from_secs(3);
    while !app.store.queues().unwrap()[0].automatic() {
        assert!(
            Instant::now() < deadline,
            "Schedule command was not persisted"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    app.refresh_and_wait();
    click(&mut app, &ctx, "Scheduler");
    assert!(app.schedule_mode == ScheduleMode::SelectedDays);
    assert_eq!(
        app.schedule_days,
        [true, false, true, false, true, false, false]
    );
    assert_eq!(app.schedule_time, "23:00");
    assert_eq!(app.schedule_stop, "07:00");
}

#[test]
fn schedule_dialog_rejects_bad_time_and_preserves_input() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "Scheduler");
    app.schedule_date = "not-a-date".into();
    app.schedule_time = "25:99".into();
    click(&mut app, &ctx, "Set schedule");
    assert!(app.dialog == Some(Dialog::Schedule));
    assert!(app.form_error.is_some());
    assert_eq!(app.schedule_time, "25:99");
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none());
    assert!(app.store.queues().unwrap()[0].scheduled_at.is_none());
}
#[test]
fn queue_controls_and_selected_error_details_remain_visible_at_minimum_size() {
    let (_temp, mut app, ctx) = fixture();
    let size = egui::vec2(860.0, 600.0);
    app.selected = Some(app.jobs[2].id.clone());
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(theme);
        frame(&mut app, &ctx, size, vec![]);
        let output = frame(&mut app, &ctx, size, vec![]);
        for label in [
            "Add URL",
            "Start again",
            "Pause",
            "Stop",
            "Stop all",
            "More",
            "File actions",
            "Server returned HTTP 403",
            "File name",
            "Actions",
        ] {
            let clipped = output.shapes.iter().find(|clipped| {
                matches!(&clipped.shape, egui::epaint::Shape::Text(text) if text.galley.job.text.trim() == label)
            }).unwrap_or_else(||panic!("No visible control: {label}"));
            let rect = text_rect(&output, label);
            assert!(
                clipped.clip_rect.expand(1.0).contains_rect(rect),
                "{label} is clipped: {rect:?}, clip: {:?}",
                clipped.clip_rect
            );
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                "{label} is outside window"
            );
        }
        let toolbar = ["Add URL", "Start again", "Pause", "Stop", "Stop all"];
        for labels in toolbar.windows(2) {
            let left = text_rect(&output, labels[0]);
            let right = text_rect(&output, labels[1]);
            assert!(
                left.max.x + 2.0 <= right.min.x,
                "Toolbar labels overlap: {labels:?}, {left:?}, {right:?}"
            );
        }
    }
}

#[test]
fn link_dialog_validates_drafts_and_cancel_ignores_late_results() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "File actions");
    click(&mut app, &ctx, "Update download link");
    assert!(app.dialog == Some(Dialog::UpdateLink));
    app.link.as_mut().unwrap().url = "ftp://example.com/file.zip".into();
    click(&mut app, &ctx, "Check link");
    assert!(app.form_error.as_ref().unwrap().contains("HTTP"));
    assert_eq!(app.link.as_ref().unwrap().url, "ftp://example.com/file.zip");
    assert!(app.link.as_ref().unwrap().request.is_none());
    let request = Uuid::new_v4();
    app.link.as_mut().unwrap().request = Some(request);
    app.link.as_mut().unwrap().state = LinkState::Checking {
        checked: 256,
        total: 1024,
    };
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none());
    assert!(app.link.is_none());
    app.link_event(LinkEvent::Checked {
        request,
        outcome: LinkOutcome::Fresh,
    });
    assert!(app.dialog.is_none());
    assert!(app.store.list().unwrap().is_empty());
}

#[test]
fn right_click_and_three_dot_menus_start_only_the_existing_selected_file() {
    for context_menu in [true, false] {
        let (_temp, mut app, ctx) = fixture();
        let now = chrono::Utc::now().timestamp();
        // Hold transfers with an explicit schedule and a full quota: the test
        // exercises real UI/scheduler dispatch without making network requests.
        app.store
            .schedule_queue(MAIN_QUEUE, now + 3600, now)
            .unwrap();
        app.store
            .set_quota_limits_at(
                quota::Limits {
                    hourly: Some(1024 * 1024),
                    daily: None,
                },
                now,
            )
            .unwrap();
        app.store.record_quota_bytes_at(1024 * 1024, now).unwrap();
        let mut job = app.jobs[0].clone();
        job.status = Status::Queued;
        app.store.enqueue(&job).unwrap();
        app.refresh_and_wait();
        app.set_selection(Some(job.id.clone()));
        // The details panel repeats the filename; target the actual table button.
        app.interface.show_details = false;
        let size = egui::vec2(1080.0, 800.0);
        if context_menu {
            frame(&mut app, &ctx, size, vec![]);
            let output = frame(&mut app, &ctx, size, vec![]);
            let pos = text_rect(&output, &job.name).center();
            for pressed in [true, false] {
                frame(
                    &mut app,
                    &ctx,
                    size,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        } else {
            click(&mut app, &ctx, "⋮");
        }
        assert!(
            egui::Popup::is_any_open(&ctx),
            "Menu did not open; context menu: {context_menu}"
        );
        click(&mut app, &ctx, "Start immediately");
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.store.immediate_downloads().unwrap().is_empty() {
            assert!(
                Instant::now() < deadline,
                "Start immediately was not applied"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let saved = app.store.get(&job.id).unwrap().unwrap();
        assert_eq!(saved.downloaded, job.downloaded);
        assert_eq!(saved.id, job.id);
        assert_eq!(app.store.list().unwrap().len(), 1);
        assert!(!app.store.queues().unwrap()[0].running);
        assert_eq!(
            app.store.queues().unwrap()[0].scheduled_at,
            Some(now + 3600)
        );
        app.scheduler.send(Command::Pause(job.id.clone())).unwrap();
    }
}

#[test]
fn changing_a_checked_link_requires_a_new_check_and_rejects_old_results() {
    let (_temp, mut app, ctx) = fixture();
    app.open_link(&app.jobs[2].clone());
    let old = Uuid::new_v4();
    let link = app.link.as_mut().unwrap();
    link.url = "https://example.com/fresh.zip".into();
    link.request = Some(old);
    link.state = LinkState::Ready(LinkOutcome::RestartRequired("Saved data differs.".into()));
    click(&mut app, &ctx, "Change link");
    assert!(matches!(
        app.link.as_ref().unwrap().state,
        LinkState::Editing
    ));
    assert_eq!(
        app.link.as_ref().unwrap().url,
        "https://example.com/fresh.zip"
    );
    app.link_event(LinkEvent::Checked {
        request: old,
        outcome: LinkOutcome::Fresh,
    });
    assert!(matches!(
        app.link.as_ref().unwrap().state,
        LinkState::Editing
    ));
    let fresh = Uuid::new_v4();
    app.link.as_mut().unwrap().request = Some(fresh);
    app.link_event(LinkEvent::Failed {
        request: fresh,
        message: "The link expired again".into(),
    });
    assert!(matches!(
        app.link.as_ref().unwrap().state,
        LinkState::Editing
    ));
    assert_eq!(app.form_error.as_deref(), Some("The link expired again"));
}

#[test]
fn link_results_and_actions_fit_the_minimum_window_in_both_appearances() {
    let (_temp, mut app, ctx) = fixture();
    let size = egui::vec2(860.0, 600.0);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(theme);
        for outcome in [
            LinkOutcome::Resume {
                saved: 1024,
                total: 4096,
            },
            LinkOutcome::Fresh,
            LinkOutcome::RestartRequired("The replacement does not match the saved data.".into()),
        ] {
            app.open_link(&app.jobs[2].clone());
            let action = match outcome {
                LinkOutcome::Resume { .. } => "Update and resume",
                LinkOutcome::Fresh => "Update and start",
                LinkOutcome::RestartRequired(_) => "Update and restart",
            };
            app.link.as_mut().unwrap().state = LinkState::Ready(outcome);
            frame(&mut app, &ctx, size, vec![]);
            frame(&mut app, &ctx, size, vec![]);
            let output = frame(&mut app, &ctx, size, vec![]);
            for label in ["New file URL", action, "Change link"] {
                let rect = text_rect(&output, label);
                let clip = output.shapes.iter().find(|clipped| {
                    matches!(&clipped.shape, egui::epaint::Shape::Text(text) if text.galley.job.text.trim() == label)
                }).unwrap();
                assert!(
                    clip.clip_rect.expand(1.0).contains_rect(rect),
                    "{label} is clipped"
                );
                assert!(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                    "{label} is outside window"
                );
            }
        }
    }
}

#[test]
fn authenticated_add_reveals_fields_validates_before_creating_and_cancel_clears_draft() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "Add URL");
    app.url = "https://example.com/protected.zip".into();
    click(&mut app, &ctx, "Advanced");
    click(&mut app, &ctx, "Use login");
    let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
    text_rect(&output, "Username");
    text_rect(&output, "Password");
    click(&mut app, &ctx, "Start download");
    assert_eq!(app.dialog, Some(Dialog::Add));
    assert!(app.form_error.as_deref().unwrap().contains("username"));
    assert!(app.store.list().unwrap().is_empty());
    *app.add_login.secret = "discard-on-cancel".into();
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none() && app.authentication.is_none());
    assert!(app.add_login.secret.is_empty());
    assert!(app.store.list().unwrap().is_empty());
    app.open_add();
    assert!(app.add_login.secret.is_empty());
}

#[test]
fn authentication_validates_switches_methods_and_correlates_save_results() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "File actions");
    click(&mut app, &ctx, "Authentication");
    app.authentication.as_mut().unwrap().restart_needed = false;
    click(&mut app, &ctx, "Save and continue");
    assert!(app.form_error.as_ref().unwrap().contains("username"));
    assert!(app.authentication.as_ref().unwrap().request.is_none());
    *app.authentication.as_mut().unwrap().username = "user".into();
    *app.authentication.as_mut().unwrap().secret = "old-secret".into();
    click(&mut app, &ctx, "Bearer token");
    assert!(app.authentication.as_ref().unwrap().secret.is_empty());
    assert!(app.authentication.as_ref().unwrap().username.is_empty());
    *app.authentication.as_mut().unwrap().secret = "Bearer invalid".into();
    click(&mut app, &ctx, "Save and continue");
    assert!(app.form_error.as_ref().unwrap().contains("prefix"));
    let request = Uuid::new_v4();
    app.authentication.as_mut().unwrap().request = Some(request);
    click(&mut app, &ctx, "×");
    assert!(app.dialog == Some(Dialog::Authentication));
    app.authentication_event(auth::Event {
        request: Uuid::new_v4(),
        result: Ok(None),
    });
    assert!(app.authentication.is_some());
    app.authentication_event(auth::Event {
        request,
        result: Err("Storage unavailable".into()),
    });
    assert!(app.authentication.as_ref().unwrap().request.is_none());
    assert_eq!(app.form_error.as_deref(), Some("Storage unavailable"));
    app.authentication.as_mut().unwrap().request = Some(request);
    app.authentication_event(auth::Event {
        request,
        result: Ok(Some("Saved; cleanup will retry.".into())),
    });
    assert!(app.authentication.is_none() && app.dialog.is_none());
    assert_eq!(app.notice.as_deref(), Some("Saved; cleanup will retry."));
}

#[test]
fn authentication_inputs_are_masked_fit_the_minimum_window_and_drop_undo_history() {
    let (_temp, mut app, ctx) = fixture();
    let size = egui::vec2(860.0, 600.0);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(theme);
        for method in [
            Some(AuthKind::Basic),
            Some(AuthKind::Bearer),
            Some(AuthKind::Ntlm),
            Some(AuthKind::Negotiate),
            None,
        ] {
            app.open_authentication(&app.jobs[0].clone());
            let form = app.authentication.as_mut().unwrap();
            form.method = method;
            *form.secret = "must-never-render".into();
            form.restart_needed = true;
            app.form_error = Some("Windows could not save these credentials. Try again.".into());
            frame(&mut app, &ctx, size, vec![]);
            frame(&mut app, &ctx, size, vec![]);
            let output = frame(&mut app, &ctx, size, vec![]);
            for shape in &output.shapes {
                if let egui::epaint::Shape::Text(text) = &shape.shape {
                    assert!(!text.galley.job.text.contains("must-never-render"));
                }
            }
            for label in [
                "Username / password",
                "Bearer token",
                "None",
                "Discard this download's partial data and restart",
                "Save and continue",
            ] {
                let rect = text_rect(&output, label);
                let clipped = output.shapes.iter().find(|clipped| matches!(&clipped.shape, egui::epaint::Shape::Text(text) if text.galley.job.text.trim() == label)).unwrap();
                assert!(
                    clipped.clip_rect.expand(1.0).contains_rect(rect),
                    "{label} clipped"
                );
                assert!(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                    "{label} outside window"
                );
            }
            if method.is_some() {
                let state = egui::text_edit::TextEditState::load(
                    &ctx,
                    egui::Id::new("authentication-secret"),
                )
                .unwrap();
                let undo = state.undoer();
                assert!(!undo.has_undo(&(Default::default(), String::new())));
                assert!(!undo.has_redo(&(Default::default(), String::new())));
            }
            // A click cannot discard partial data without explicit consent.
            click(&mut app, &ctx, "Save and continue");
            assert!(app.authentication.as_ref().unwrap().request.is_none());
        }
    }
}
#[test]
fn retry_details_fit_minimum_window_and_pause_is_actionable() {
    let (_temp, mut app, ctx) = fixture();
    let size = egui::vec2(860.0, 600.0);
    app.jobs.truncate(1);
    app.jobs[0].status = Status::Queued;
    app.jobs[0].retry = download_engine::retry::State {
        attempts: 3,
        next_at: Some(chrono::Utc::now().timestamp_millis() + 120000),
    };
    app.jobs[0].error = Some("Server returned HTTP 503.".into());
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(theme);
        for running in [true, false] {
            app.queues[0].running = running;
            frame(&mut app, &ctx, size, vec![]);
            let output = frame(&mut app, &ctx, size, vec![]);
            let description = retry_description(&app.jobs[0], running).unwrap();
            for label in [description.as_str(), "Server returned HTTP 503.", "Pause"] {
                let rect = text_rect(&output, label);
                let clip = output.shapes.iter().find(|clipped| {
                    matches!(&clipped.shape, egui::epaint::Shape::Text(text) if text.galley.job.text.trim() == label)
                }).unwrap().clip_rect;
                assert!(
                    clip.expand(1.0).contains_rect(rect),
                    "Clipped {label}: {rect:?} in {clip:?}"
                );
            }
            assert!(app.summary().contains("1 waiting to retry"));
        }
    }
    // Persist the preview item with its queue scheduled: clicking Pause should
    // cancel the delayed retry through the real scheduler.
    let now = chrono::Utc::now().timestamp();
    app.store
        .schedule_queue(&app.queue_id, now + 3600, now)
        .unwrap();
    app.store.enqueue(&app.jobs[0]).unwrap();
    let id = app.jobs[0].id.clone();
    click(&mut app, &ctx, "Pause");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let saved = app.store.get(&id).unwrap().unwrap();
        if saved.status == Status::Paused {
            assert!(saved.retry.next_at.is_none());
            break;
        }
        assert!(Instant::now() < deadline, "Pause was not applied");
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn speed_dialog_validates_saves_and_ignores_stale_acknowledgements() {
    let (_temp, mut app, ctx) = fixture();
    app.store.set_queue_running(&app.queue_id, false).unwrap();
    let job = app.jobs[0].clone();
    app.store.enqueue(&job).unwrap();
    app.selected = Some(job.id.clone());
    click(&mut app, &ctx, "No speed limit");
    for input in ["", "NaN", "inf", "0", "-1", "1048577", "1.0001"] {
        let field = &mut app.speed_limits.as_mut().unwrap().global;
        field.enabled = true;
        field.kib = input.into();
        app.save_speed_limits();
        assert!(app.form_error.is_some(), "{input}");
        assert!(app.speed_limits.as_ref().unwrap().request.is_none());
    }
    let form = app.speed_limits.as_mut().unwrap();
    form.global.kib = "2048".into();
    let file = &mut form.download.as_mut().unwrap().2;
    file.enabled = true;
    file.kib = "256".into();
    click(&mut app, &ctx, "Apply limits");
    let request = app.speed_limits.as_ref().unwrap().request.unwrap();
    app.bandwidth_event(bandwidth::Event {
        request: Uuid::new_v4(),
        result: Err("stale".into()),
    });
    assert_eq!(app.speed_limits.as_ref().unwrap().request, Some(request));
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.speed_limits.is_some() {
        app.refresh();
        assert!(Instant::now() < deadline, "No save acknowledgement");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.limits.global, Some(2 * 1024 * 1024));
    assert_eq!(app.limits.downloads.get(&job.id), Some(&(256 * 1024)));
    app.open_speed_limits();
    assert_eq!(app.speed_limits.as_ref().unwrap().global.kib, "2048");
    app.speed_limits.as_mut().unwrap().global.enabled = false;
    click(&mut app, &ctx, "×");
    assert_eq!(
        app.store.bandwidth_limits().unwrap().global,
        Some(2 * 1024 * 1024)
    );
    app.open_speed_limits();
    app.store.remove(&job.id).unwrap();
    app.save_speed_limits();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        app.refresh();
        if app.form_error.is_some() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.form_error.as_ref().unwrap().contains("removed"));
    assert!(app.speed_limits.as_ref().unwrap().request.is_none());
    assert_eq!(
        app.store.bandwidth_limits().unwrap().global,
        Some(2 * 1024 * 1024)
    );
}
#[test]
fn speed_controls_fit_minimum_window_in_both_themes_and_without_selection() {
    let (_temp, mut app, ctx) = fixture();
    let size = egui::vec2(860.0, 600.0);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(theme);
        for selected in [true, false] {
            app.selected = selected.then(|| app.jobs[0].id.clone());
            app.open_speed_limits();
            frame(&mut app, &ctx, size, vec![]);
            let output = frame(&mut app, &ctx, size, vec![]);
            for label in [
                "All downloads and queues",
                "Limit total speed",
                "KiB/s",
                "Apply limits",
                if selected {
                    "Limit this file"
                } else {
                    "Select an unfinished file to set its own speed limit."
                },
            ] {
                let rect = text_rect(&output, label);
                let clip = output.shapes.iter().find(|clipped| matches!(&clipped.shape,egui::epaint::Shape::Text(t) if t.galley.job.text==label)).unwrap().clip_rect;
                assert!(clip.expand(1.0).contains_rect(rect), "Clipped {label}");
                assert!(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                    "Outside {label}"
                );
            }
            app.dialog = None;
            app.speed_limits = None;
            app.limits.global = Some(bandwidth::MAX_RATE);
            app.limits
                .downloads
                .insert(app.jobs[0].id.clone(), 1024 * 1024);
            frame(&mut app, &ctx, size, vec![]);
            let output = frame(&mut app, &ctx, size, vec![]);
            for label in ["1024.0 MiB/s", "More", "≤ 1.0 MiB/s"] {
                let rect = text_rect(&output, label);
                assert!(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                    "Outside {label}"
                );
            }
        }
    }
}
#[test]
fn preferences_cancel_save_and_failed_save_preserve_the_right_state() {
    let (_temp, mut app, ctx) = fixture();
    let interface = InterfacePreferences {
        appearance: Appearance::Light,
        compact_rows: true,
        sidebar_labels: false,
        ..InterfacePreferences::default()
    };
    app.save_interface(&ctx, interface);
    // Tray settings must not rewrite the separate appearance/layout record.
    crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3"))
        .unwrap()
        .execute_batch("CREATE TRIGGER protect_appearance BEFORE UPDATE ON interface_preferences BEGIN SELECT RAISE(ABORT, 'Appearance is not editable in tray preferences'); END;")
        .unwrap();
    app.open_preferences_window();
    app.preferences_window.as_mut().unwrap().category = preferences_window::Category::Startup;
    click(&mut app, &ctx, "Notify when a download finishes");
    click(&mut app, &ctx, "×");
    assert_eq!(app.interface, interface);
    assert_eq!(app.preferences, Preferences::default());
    assert!(app.preferences_window.is_none());
    assert_eq!(app.store.preferences().unwrap(), Preferences::default());
    assert_eq!(app.store.interface_preferences().unwrap(), app.interface);
    app.open_preferences_window();
    app.preferences_window.as_mut().unwrap().category = preferences_window::Category::Startup;
    click(&mut app, &ctx, "Notify when a download finishes");
    click(&mut app, &ctx, "Save");
    assert!(app.dialog.is_none());
    assert_eq!(app.interface, interface);
    assert_eq!(app.store.interface_preferences().unwrap(), app.interface);
    assert!(!app.preferences.notify_complete);
    assert_eq!(app.store.preferences().unwrap(), app.preferences);
    app.open_preferences_window();
    app.preferences_window.as_mut().unwrap().category = preferences_window::Category::Startup;
    click(&mut app, &ctx, "Notify when a download needs attention");
    crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3"))
        .unwrap()
        .execute("DELETE FROM desktop_preferences", [])
        .unwrap();
    click(&mut app, &ctx, "Save");
    assert!(matches!(app.dialog, Some(Dialog::PreferencesWindow)));
    assert!(
        !app.preferences_window
            .as_ref()
            .unwrap()
            .behavior
            .notify_failed
    );
    assert!(app.preferences.notify_failed);
    assert_eq!(app.interface, interface);
    assert_eq!(app.store.interface_preferences().unwrap(), app.interface);
    assert!(app.preferences_window.as_ref().unwrap().error.is_some());
}
#[test]
fn preferences_fit_minimum_window_in_both_themes() {
    for appearance in [Appearance::Light, Appearance::Dark] {
        let (_temp, mut app, ctx) = fixture();
        app.interface.appearance = appearance;
        app.configure_appearance(&ctx);
        app.open_preferences_window();
        let size = egui::vec2(860.0, 600.0);
        for category in preferences_window::Category::ALL {
            app.preferences_window.as_mut().unwrap().category = category;
            frame(&mut app, &ctx, size, vec![]);
            let output = frame(&mut app, &ctx, size, vec![]);
            for label in ["Save", "Cancel", "Reset", "×"] {
                assert!(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                        .contains_rect(text_rect(&output, label)),
                    "{category:?}: {label}"
                );
            }
        }
    }
}

#[test]
fn tray_actions_preserve_dialogs_review_other_queues_and_keep_unavailable_window_visible() {
    let (_temp, mut app, ctx) = fixture();
    let queue = app.store.create_queue("Other downloads").unwrap();
    let job = app.jobs[2].clone();
    app.store
        .enqueue_batch(std::slice::from_ref(&job), &queue.id)
        .unwrap();
    app.dialog = Some(Dialog::Add);
    app.url = "https://example.com/unsaved.zip".into();
    app.tray_action(TrayAction::Settings, &ctx);
    assert!(matches!(app.dialog, Some(Dialog::Add)));
    app.tray_action(TrayAction::BrowserSetup, &ctx);
    assert!(matches!(app.dialog, Some(Dialog::Add)));
    app.tray_action(TrayAction::Review(job.id.clone()), &ctx);
    app.background(&ctx);
    assert_eq!(app.pending_review.as_deref(), Some(job.id.as_str()));
    assert_eq!(app.url, "https://example.com/unsaved.zip");
    app.dialog = None;
    let selected = app.selected.clone();
    app.tray_action(TrayAction::Settings, &ctx);
    assert_eq!(app.view, View::Downloads);
    assert_eq!(app.dialog, Some(Dialog::PreferencesWindow));
    app.close_preferences_window(&ctx);
    assert_eq!(app.selected, selected);
    app.category = Some(Category::Video);
    app.search = "not this file".into();
    app.view = View::Queues;
    app.interface.show_details = false;
    app.background(&ctx);
    assert!(app.search.is_empty());
    assert_eq!(app.view, View::Downloads);
    assert!(app.interface.show_details);
    assert!(app.store.interface_preferences().unwrap().show_details);
    assert_eq!(app.category, None);
    assert_eq!(app.queue_id, queue.id);
    assert_eq!(app.selected, Some(job.id));
    app.tray_action(TrayAction::Review("removed-job".into()), &ctx);
    app.background(&ctx);
    assert!(app.notice.as_ref().unwrap().contains("removed"));
    app.preferences.close_to_tray = true;
    let output = ctx.run_logic(&egui::RawInput::default(), |ctx| app.hide_to_tray(ctx));
    let commands = &output.viewport_commands[&egui::ViewportId::ROOT];
    assert!(
        !commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Visible(false)))
    );
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Visible(true)))
    );
    let mut input = egui::RawInput::default();
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    let output = ctx.run_logic(&input, |ctx| {
        app.tray_action(TrayAction::Exit, ctx);
        app.background(ctx);
    });
    assert!(app.exit_requested);
    assert!(
        output.viewport_commands[&egui::ViewportId::ROOT]
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Close))
    );
    assert!(
        !output.viewport_commands[&egui::ViewportId::ROOT]
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::CancelClose))
    );
}

#[test]
fn category_filter_combines_with_status_tabs_and_clears_stale_details() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "Documents");
    assert_eq!(app.category, Some(Category::Documents));
    assert_eq!(app.selected.as_deref(), Some(app.jobs[1].id.as_str()));
    click(&mut app, &ctx, "Unfinished");
    assert!(app.selected.is_none());
    let output = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
    text_rect(&output, "No downloads match these filters");
    click(&mut app, &ctx, "Documents ×");
    assert_eq!(app.category, None);
    assert_eq!(app.filter, Filter::Unfinished);
    assert_eq!(app.selected.as_deref(), Some(app.jobs[0].id.as_str()));
}

#[test]
fn folder_settings_cancel_validate_persist_and_retain_failed_drafts() {
    let (temp, mut app, ctx) = fixture();
    app.open_download_folders();
    app.folders_draft.as_mut().unwrap().paths[0] =
        temp.path().join("documents").to_string_lossy().into_owned();
    click(&mut app, &ctx, "×");
    assert!(app.folders_draft.is_none());
    assert_eq!(
        app.store.download_folders().unwrap(),
        DownloadFolders::default()
    );
    app.open_download_folders();
    app.folders_draft.as_mut().unwrap().paths[0] = "relative".into();
    click(&mut app, &ctx, "Save folder rules");
    assert!(matches!(app.dialog, Some(Dialog::DownloadFolders)));
    assert!(app.form_error.as_deref().unwrap().contains("absolute"));
    assert_eq!(app.folders_draft.as_ref().unwrap().paths[0], "relative");
    let custom = temp.path().join("custom documents");
    app.folders_draft.as_mut().unwrap().paths[0] = custom.to_string_lossy().into_owned();
    click(&mut app, &ctx, "Save folder rules");
    assert!(app.dialog.is_none());
    assert_eq!(
        app.store.download_folders().unwrap().overrides[&Category::Documents],
        custom
    );
    assert_eq!(
        app.download_folders.as_ref().unwrap(),
        &app.store.download_folders().unwrap()
    );
    app.open_download_folders();
    assert_eq!(
        app.folders_draft.as_ref().unwrap().paths[0],
        custom.to_string_lossy()
    );
    click(
        &mut app,
        &ctx,
        "Automatically sort new downloads by category",
    );
    crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3")).unwrap()
        .execute_batch("CREATE TRIGGER reject_folders BEFORE UPDATE ON download_folders BEGIN SELECT RAISE(ABORT,'fixture save rejected'); END;").unwrap();
    click(&mut app, &ctx, "Save folder rules");
    assert!(matches!(app.dialog, Some(Dialog::DownloadFolders)));
    assert!(!app.folders_draft.as_ref().unwrap().organize);
    assert!(app.store.download_folders().unwrap().organize);
    assert!(app.download_folders.as_ref().unwrap().organize);
    assert!(
        app.form_error
            .as_deref()
            .unwrap()
            .contains("fixture save rejected")
    );
}

#[test]
fn clearing_an_override_restores_defaults_and_disabling_sorting_is_persistent() {
    let (temp, mut app, ctx) = fixture();
    let mut rules = DownloadFolders::default();
    rules
        .overrides
        .insert(Category::Documents, temp.path().join("custom"));
    app.store.save_download_folders(&rules).unwrap();
    app.open_download_folders();
    app.folders_draft.as_mut().unwrap().paths[0].clear();
    click(
        &mut app,
        &ctx,
        "Automatically sort new downloads by category",
    );
    click(&mut app, &ctx, "Save folder rules");
    let saved = Store::open(app.store.paths.clone())
        .unwrap()
        .download_folders()
        .unwrap();
    assert!(!saved.organize);
    assert!(saved.overrides.is_empty());
    assert_eq!(
        saved.destination(&app.store.paths.downloads, "a.pdf"),
        app.store.paths.downloads
    );
}

#[test]
fn add_and_batch_apply_rules_and_explicit_overrides_and_reveal_new_jobs() {
    let (temp, mut app, ctx) = fixture();
    let queue = app.store.create_queue("Later").unwrap();
    app.queue_id = queue.id;
    app.refresh_and_wait();
    app.category = Some(Category::Video);
    app.open_add();
    app.url = "https://example.test/original.zip".into();
    app.name = "renamed.pdf".into();
    click(&mut app, &ctx, "Add to queue");
    assert_eq!(app.jobs.len(), 1);
    assert_eq!(
        app.jobs[0].directory,
        app.store.paths.downloads.join("Documents")
    );
    assert_eq!(app.category, None);
    assert_eq!(app.selected.as_deref(), Some(app.jobs[0].id.as_str()));
    app.open_queue_dialog(Dialog::Batch);
    app.batch = "https://example.test/a.zip\nhttps://example.test/b.mp3".into();
    click(&mut app, &ctx, "Add files");
    assert_eq!(app.jobs.len(), 3);
    for job in &app.jobs {
        assert_eq!(
            job.directory,
            app.store.paths.downloads.join(job.category().label())
        );
    }
    let explicit = temp.path().join("selected folder");
    app.folder = explicit.to_string_lossy().into_owned();
    app.open_queue_dialog(Dialog::Batch);
    app.batch = "https://example.test/c.pdf\nhttps://example.test/d.mp4".into();
    click(&mut app, &ctx, "Add files");
    assert!(
        app.jobs
            .iter()
            .filter(|j| ["c.pdf", "d.mp4"].contains(&j.name.as_str()))
            .all(|j| j.directory == explicit)
    );
    app.open_add();
    app.url = "https://example.test/e.zip".into();
    click(&mut app, &ctx, "Add to queue");
    assert_eq!(app.jobs.len(), 6);
    assert_eq!(
        app.jobs
            .iter()
            .find(|j| j.name == "e.zip")
            .unwrap()
            .directory,
        explicit
    );
}

#[test]
fn category_controls_and_folder_dialog_fit_the_minimum_window_in_both_themes() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let (_temp, mut app, ctx) = fixture();
        ctx.set_theme(theme);
        let size = egui::vec2(860.0, 600.0);
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        frame(&mut app, &ctx, size, vec![]);
        let output = frame(&mut app, &ctx, size, vec![]);
        let tabs = text_rect(&output, "Completed");
        let category = text_rect(&output, "Documents");
        assert!(tabs.bottom() < category.top());
        assert!(bounds.contains_rect(category));
        app.open_download_folders();
        app.folders_draft.as_mut().unwrap().paths[0] = "relative".into();
        app.form_error =
            Some("Documents folder is invalid: Choose an absolute download folder".into());
        frame(&mut app, &ctx, size, vec![]);
        let output = frame(&mut app, &ctx, size, vec![]);
        for label in [
            "Download folders",
            "Automatically sort new downloads by category",
            "Documents",
            "Programs",
            "Compressed",
            "Music",
            "Video",
            "Other",
            "Save folder rules",
            "×",
        ] {
            assert!(
                bounds.contains_rect(text_rect(&output, label)),
                "{label} overflows the minimum window"
            );
        }
    }
}

#[test]
fn category_add_preview_uses_renamed_filename_and_custom_rule_at_minimum_size() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let (temp, mut app, ctx) = fixture();
        ctx.set_theme(theme);
        let folder = temp.path().join("my documents");
        let mut rules = DownloadFolders::default();
        rules.overrides.insert(Category::Documents, folder.clone());
        app.store.save_download_folders(&rules).unwrap();
        app.open_add();
        app.wait_for_io();
        app.url = "https://example.test/original.zip".into();
        app.name = "guide.pdf".into();
        let size = egui::vec2(860.0, 600.0);
        frame(&mut app, &ctx, size, vec![]);
        let output = frame(&mut app, &ctx, size, vec![]);
        for label in [
            "Documents · example.test · HTTPS",
            &format!("Automatic → {}", folder.display()),
            "Start download",
            "×",
        ] {
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                    .contains_rect(text_rect(&output, label)),
                "{label}"
            );
        }
    }
}
#[test]
fn filename_search_combines_with_status_and_category_and_can_be_cleared() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "Search downloads…");
    frame(
        &mut app,
        &ctx,
        egui::vec2(1080.0, 800.0),
        vec![egui::Event::Text("  .PDF  ".into())],
    );
    assert_eq!(app.search, "  .PDF  ");
    assert_eq!(app.jobs.iter().filter(|j| app.accepts(j)).count(), 1);
    assert_eq!(app.selected.as_deref(), Some(app.jobs[1].id.as_str()));
    click(&mut app, &ctx, "Completed");
    click(&mut app, &ctx, "Documents");
    assert_eq!(app.category, Some(Category::Documents));
    click(&mut app, &ctx, "Clear");
    assert!(app.search.is_empty());
    assert_eq!(app.filter, Filter::Complete);
    assert_eq!(app.jobs.iter().filter(|j| app.accepts(j)).count(), 1);
}
#[test]
fn details_can_be_hidden_and_reopened_by_clicking_a_filename() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "×");
    assert!(!app.interface.show_details);
    assert!(!app.store.interface_preferences().unwrap().show_details);
    click(&mut app, &ctx, "failed.zip");
    assert!(app.interface.show_details);
    assert!(app.store.interface_preferences().unwrap().show_details);
    assert_eq!(app.selected.as_deref(), Some(app.jobs[2].id.as_str()));
    let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
    text_rect(&output, "Server returned HTTP 403");
}
#[test]
fn navigation_preserves_filters_and_queue_reordering_remains_reachable() {
    let (_temp, mut app, ctx) = fixture();
    app.search = "zip".into();
    app.reconcile_selection();
    let selected = app.selected.clone();
    click(&mut app, &ctx, "Manage queues");
    assert_eq!(app.view, View::Queues);
    click(&mut app, &ctx, "All downloads");
    assert_eq!(app.view, View::Downloads);
    assert_eq!(app.search, "zip");
    assert_eq!(app.selected, selected);
    click(&mut app, &ctx, "File actions");
    click(&mut app, &ctx, "Move to");
    let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
    text_rect(&output, "Move up");
    text_rect(&output, "Move down");
}
#[test]
fn duplicate_preserves_the_original_file_and_requires_fresh_authentication() {
    for authenticated in [false, true] {
        let (_temp, mut app, ctx) = fixture();
        let now = chrono::Utc::now().timestamp();
        app.store
            .schedule_queue(&app.queue_id, now + 3600, now)
            .unwrap();
        let mut original = app.jobs[1].clone();
        std::fs::create_dir_all(&original.directory).unwrap();
        let saved = original.directory.join(&original.name);
        std::fs::write(&saved, b"original file").unwrap();
        original.saved_path = Some(saved.clone());
        original.plan = Some(download_engine::model::Plan {
            size: 4096,
            etag: "fixture".into(),
            final_url: original.url.clone(),
            segments: vec![],
        });
        if authenticated {
            original.authentication = Some(auth::Binding {
                id: Uuid::new_v4(),
                origin: "https://example.com".into(),
                kind: auth::Kind::Bearer,
            });
        }
        app.store
            .enqueue_batch(std::slice::from_ref(&original), &app.queue_id)
            .unwrap();
        app.refresh_and_wait();
        app.selected = Some(original.id.clone());
        click(&mut app, &ctx, "File actions");
        click(&mut app, &ctx, "Duplicate");
        let jobs = app.store.queue_jobs(&app.queue_id).unwrap();
        assert_eq!(jobs.len(), 2);
        let copy = jobs.iter().find(|j| j.id != original.id).unwrap();
        assert_ne!(copy.request_id, original.request_id);
        assert_eq!(copy.name, original.name);
        assert_eq!(copy.url, original.url);
        assert_eq!(copy.directory, original.directory);
        assert_eq!(copy.downloaded, 0);
        assert!(copy.plan.is_none());
        assert!(copy.saved_path.is_none());
        assert!(copy.authentication.is_none());
        assert!(copy.retry.next_at.is_none());
        assert_eq!(std::fs::read(saved).unwrap(), b"original file");
        assert_eq!(
            app.store.get(&original.id).unwrap().unwrap().status,
            Status::Complete
        );
        if authenticated {
            assert_eq!(copy.status, Status::Paused);
            assert!(matches!(app.dialog, Some(Dialog::Authentication)));
            assert_eq!(app.authentication.as_ref().unwrap().job, copy.id);
        } else {
            assert_eq!(copy.status, Status::Queued);
            assert!(app.dialog.is_none());
        }
    }
}

#[test]
fn startup_rows_follow_the_entry_and_save_without_the_run_key() {
    // Unit-test builds use an in-memory Run entry, never the Windows registry.
    let (_root, mut app, ctx) = fixture();
    app.open_preferences_window();
    app.preferences_window.as_mut().unwrap().category = preferences_window::Category::Startup;
    click(&mut app, &ctx, "Start in the tray");
    assert!(app.preferences_window.as_ref().unwrap().start_hidden);
    click(&mut app, &ctx, "Start with Windows");
    assert!(
        !app.startup.enabled(),
        "Editing must not write the Run entry"
    );
    assert!(
        app.store
            .startup_preferences()
            .unwrap()
            .registered
            .is_none()
    );
    click(&mut app, &ctx, "Start in the tray");
    click(&mut app, &ctx, "Save");
    assert!(app.startup.enabled());
    assert!(!app.startup.start_hidden());
    assert_eq!(
        app.store.startup_preferences().unwrap().registered,
        Some(std::env::current_exe().unwrap())
    );
    app.open_preferences_window();
    app.preferences_window.as_mut().unwrap().category = preferences_window::Category::Startup;
    click(&mut app, &ctx, "Start with Windows");
    click(&mut app, &ctx, "Cancel");
    assert!(app.startup.enabled(), "Cancel must leave startup unchanged");
    app.open_preferences_window();
    app.preferences_window.as_mut().unwrap().category = preferences_window::Category::Startup;
    click(&mut app, &ctx, "Start with Windows");
    click(&mut app, &ctx, "Save");
    assert!(!app.startup.enabled());
    assert_eq!(
        app.store.startup_preferences().unwrap(),
        download_engine::preferences::StartupPreferences {
            start_hidden: false,
            registered: None
        }
    );
}

#[test]
fn sign_in_start_without_a_tray_keeps_the_window_open_once() {
    let (_root, mut app, ctx) = fixture();
    let shows = |output: &egui::LogicOutput, visible: bool| {
        output
            .viewport_commands
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|commands| commands.contains(&egui::ViewportCommand::Visible(visible)))
    };
    // Unit tests have no tray icon, so hiding would leave the window unreachable.
    app.hide_on_start = true;
    let output = ctx.run_logic(&egui::RawInput::default(), |ctx| app.background(ctx));
    assert!(shows(&output, true) && !shows(&output, false));
    assert!(app.notice.as_ref().unwrap().contains("tray"));
    let output = ctx.run_logic(&egui::RawInput::default(), |ctx| app.background(ctx));
    assert!(!shows(&output, true) && !shows(&output, false));
}
