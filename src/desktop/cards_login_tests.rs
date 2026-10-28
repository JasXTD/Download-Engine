//! Observable behavior of queue cards, settings rows and inline Add download login.
use super::interaction_tests::{click, fixture, frame, text_rect};
use super::*;

const SMALL: egui::Vec2 = egui::vec2(860.0, 600.0);

fn settled(app: &mut Desktop, ctx: &egui::Context, size: egui::Vec2) -> egui::FullOutput {
    for _ in 0..5 {
        frame(app, ctx, size, vec![]);
    }
    frame(app, ctx, size, vec![])
}

fn visible(output: &egui::FullOutput, label: &str, size: egui::Vec2) {
    let rect = text_rect(output, label);
    assert!(
        egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
        "Outside window: {label} {rect:?}"
    );
    assert!(
        output.shapes.iter().any(
            |shape| matches!(&shape.shape, egui::epaint::Shape::Text(text)
        if text.galley.job.text.trim() == label && shape.clip_rect.expand(1.0).contains_rect(rect))
        ),
        "Clipped: {label} {rect:?}"
    );
}

fn click_at(app: &mut Desktop, ctx: &egui::Context, size: egui::Vec2, pos: egui::Pos2) {
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

fn wait_for(app: &mut Desktop, predicate: impl Fn(&Desktop) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.refresh();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out waiting for scheduler result"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn inline_login_checkbox_reveals_visible_masked_fields_at_minimum_size() {
    for dark in [false, true] {
        let (_temp, mut app, ctx) = fixture();
        app.interface.appearance = if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        app.configure_appearance(&ctx);
        app.url = "https://example.invalid/private.zip".into();
        app.open_add();
        click(&mut app, &ctx, "Advanced");
        let output = settled(&mut app, &ctx, SMALL);
        visible(&output, "Use login", SMALL);
        click_at(
            &mut app,
            &ctx,
            SMALL,
            text_rect(&output, "Use login").center(),
        );
        assert!(app.add_authentication);
        *app.add_login.secret = "must-never-render".into();
        let output = settled(&mut app, &ctx, SMALL);
        for label in ["Use login", "Username", "Password", "Start download", "×"] {
            visible(&output, label, SMALL);
        }
        assert!(output.shapes.iter().all(|shape| !matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.job.text.contains("must-never-render"))));
        let state =
            egui::text_edit::TextEditState::load(&ctx, egui::Id::new("add-login-secret")).unwrap();
        assert!(
            !state
                .undoer()
                .has_undo(&(Default::default(), String::new()))
        );
        assert!(
            !state
                .undoer()
                .has_redo(&(Default::default(), String::new()))
        );
        click_at(
            &mut app,
            &ctx,
            SMALL,
            text_rect(&output, "Use login").center(),
        );
        assert!(!app.add_authentication);
        assert!(app.add_login.secret.is_empty());
        assert!(app.store.list().unwrap().is_empty());
    }
}

#[test]
fn add_destination_queue_is_a_draft_until_the_download_is_submitted() {
    let (_temp, mut app, ctx) = fixture();
    app.store.set_queue_running(MAIN_QUEUE, false).unwrap();
    let later = app.store.create_queue("Later").unwrap();
    app.refresh_and_wait();
    app.url = "https://example.invalid/draft.zip".into();
    app.open_add();
    click(&mut app, &ctx, "Main queue");
    click(&mut app, &ctx, "Later");
    assert_eq!(app.add_queue_id(), later.id);
    assert_eq!(app.queue_id, MAIN_QUEUE);
    click(&mut app, &ctx, "×");
    assert_eq!(app.queue_id, MAIN_QUEUE);
    assert!(app.store.list().unwrap().is_empty());
    app.open_add();
    click(&mut app, &ctx, "Main queue");
    click(&mut app, &ctx, "Later");
    click(&mut app, &ctx, "Add to queue");
    assert!(app.dialog.is_none());
    assert_eq!(app.queue_id, later.id);
    assert_eq!(app.store.queue_jobs(&later.id).unwrap().len(), 1);
    assert!(app.store.queue_jobs(MAIN_QUEUE).unwrap().is_empty());
}

#[test]
fn idle_main_offers_start_download_but_a_schedule_keeps_add_to_queue() {
    let (_temp, mut app, ctx) = fixture();
    app.store.set_queue_running(MAIN_QUEUE, false).unwrap();
    app.refresh_and_wait();
    app.open_add();
    text_rect(&settled(&mut app, &ctx, SMALL), "Start download");
    let now = chrono::Utc::now().timestamp();
    app.store
        .schedule_queue(MAIN_QUEUE, now + 3600, now)
        .unwrap();
    app.refresh_and_wait();
    text_rect(&settled(&mut app, &ctx, SMALL), "Add to queue");
    assert!(app.store.list().unwrap().is_empty());
}

#[test]
fn inline_login_keeps_the_url_visible_when_the_complete_form_fits() {
    let (_temp, mut app, ctx) = fixture();
    app.url = "https://example.invalid/private.zip".into();
    app.add_authentication = true;
    app.open_add();
    let size = egui::vec2(1280.0, 860.0);
    let output = settled(&mut app, &ctx, size);
    for label in [
        "File URL",
        "Use login",
        "Username",
        "Password",
        "Start download",
        "×",
    ] {
        visible(&output, label, size);
    }
}

#[test]
fn inline_login_method_and_destination_changes_discard_credentials() {
    let (_temp, mut app, ctx) = fixture();
    app.url = "https://example.invalid/private.zip".into();
    app.add_authentication = true;
    app.open_add();
    *app.add_login.username = "fixture-user".into();
    *app.add_login.secret = "fixture-password".into();
    click(&mut app, &ctx, "Bearer token");
    assert_eq!(app.add_login.method, AuthKind::Bearer);
    assert!(app.add_login.username.is_empty() && app.add_login.secret.is_empty());
    *app.add_login.secret = "fixture-token".into();
    app.url.clear();
    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("add-file-url")));
    frame(
        &mut app,
        &ctx,
        egui::vec2(1080.0, 800.0),
        vec![egui::Event::Text("https://other.invalid/file.zip".into())],
    );
    assert_eq!(app.url, "https://other.invalid/file.zip");
    assert!(app.add_login.secret.is_empty());
    assert_eq!(app.add_login.method, AuthKind::Basic);
    app.url = "https://example.invalid/stream.m3u8".into();
    app.add_authentication = true;
    *app.add_login.username = "fixture-user".into();
    app.add();
    app.wait_for_io();
    assert!(app.form_error.as_deref().unwrap().contains("direct file"));
    assert!(app.store.list().unwrap().is_empty());
}

#[test]
fn inline_login_failed_save_stays_paused_retries_same_job_and_ignores_stale_results() {
    let (_temp, mut app, ctx) = fixture();
    app.store.set_queue_running(MAIN_QUEUE, false).unwrap();
    app.refresh_and_wait();
    // Remote plain HTTP is rejected before vault access or network traffic.
    app.url = "http://example.invalid/private.zip".into();
    app.add_authentication = true;
    app.open_add();
    *app.add_login.username = "fixture-user".into();
    *app.add_login.secret = "fixture-password".into();
    app.add();
    app.wait_for_io();
    let request = app.add_login.request.unwrap();
    let id = app.add_login.job.as_ref().unwrap().id.clone();
    click(&mut app, &ctx, "×");
    assert_eq!(app.dialog, Some(Dialog::Add));
    app.authentication_event(auth::Event {
        request: Uuid::new_v4(),
        result: Ok(None),
    });
    assert_eq!(app.add_login.request, Some(request));
    wait_for(&mut app, |app| app.add_login.request.is_none());
    assert!(app.form_error.as_ref().unwrap().contains("HTTPS"));
    let job = app.store.get(&id).unwrap().unwrap();
    assert_eq!(job.status, Status::Paused);
    assert!(job.authentication.is_none());
    let output = settled(&mut app, &ctx, SMALL);
    visible(&output, "Retry login", SMALL);
    visible(&output, "×", SMALL);
    visible(&output, app.form_error.as_deref().unwrap(), SMALL);
    app.add();
    app.wait_for_io();
    assert_eq!(app.add_login.job.as_ref().unwrap().id, id);
    wait_for(&mut app, |app| app.add_login.request.is_none());
    assert_eq!(app.store.list().unwrap().len(), 1);
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none());
    assert!(app.add_login.secret.is_empty() && app.add_login.username.is_empty());
    assert!(app.add_login.job.is_none());
    assert_eq!(app.store.get(&id).unwrap().unwrap().status, Status::Paused);
}

#[cfg(windows)]
#[test]
fn inline_login_saves_through_scheduler_and_windows_vault_before_queueing() {
    for (url, method) in [
        ("https://example.invalid/private.zip", AuthKind::Basic),
        ("https://example.invalid/token.zip", AuthKind::Bearer),
        ("https://example.invalid/ntlm.zip", AuthKind::Ntlm),
        ("https://example.invalid/negotiate.zip", AuthKind::Negotiate),
        ("ftps://example.invalid/private.zip", AuthKind::Basic),
    ] {
        let (_temp, mut app, ctx) = fixture();
        // Explicit scheduling holds the authenticated job without a network
        // request; an idle, unscheduled Main queue now starts new files.
        let now = chrono::Utc::now().timestamp();
        app.store
            .schedule_queue(MAIN_QUEUE, now + 3600, now)
            .unwrap();
        app.refresh_and_wait();
        app.url = url.into();
        app.open_add();
        app.add_authentication = true;
        app.add_login.method = method;
        *app.add_login.username = "fixture-user".into();
        *app.add_login.secret = "fixture-secret".into();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            app.add();
            app.wait_for_io();
            assert!(app.add_login.request.is_some());
            let id = app.add_login.job.as_ref().unwrap().id.clone();
            click(&mut app, &ctx, "×");
            assert_eq!(app.dialog, Some(Dialog::Add));
            wait_for(&mut app, |app| app.add_login.request.is_none());
            assert!(app.form_error.is_none(), "{:?}", app.form_error);
            assert!(app.dialog.is_none() && app.authentication.is_none());
            assert!(app.add_login.secret.is_empty() && app.add_login.username.is_empty());
            let job = app.store.get(&id).unwrap().unwrap();
            assert_eq!(job.status, Status::Queued);
            assert_eq!(job.downloaded, 0);
            let binding = job.authentication.as_ref().unwrap();
            assert_eq!(binding.kind, method);
            assert!(auth::credentials_present(binding.id).unwrap());
            assert!(!app.store.queues().unwrap()[0].running);
            assert_eq!(app.store.list().unwrap().len(), 1);
        }));
        // Remove synthetic credentials through the same journaled path even on failure.
        let jobs = app.store.list().unwrap();
        for job in &jobs {
            app.scheduler.send(Command::Remove(job.id.clone())).unwrap();
        }
        // The store commits removal before its journaled vault cleanup completes.
        wait_for(&mut app, |app| {
            app.store.list().unwrap().is_empty()
                && jobs
                    .iter()
                    .filter_map(|job| job.authentication.as_ref())
                    .all(|binding| !auth::credentials_present(binding.id).unwrap())
        });
        for binding in jobs.iter().filter_map(|job| job.authentication.as_ref()) {
            assert!(!auth::credentials_present(binding.id).unwrap());
        }
        if let Err(error) = result {
            std::panic::resume_unwind(error);
        }
    }
}

#[test]
fn settings_rows_persist_toggles_and_route_to_existing_editors() {
    use preferences_window::Category as C;
    let (_temp, mut app, ctx) = fixture();
    app.store.save_interface_preferences(app.interface).unwrap();
    app.open_preferences_window();
    let before = app.interface;
    for label in ["Compact rows", "Details panel", "Sidebar labels"] {
        click(&mut app, &ctx, label);
        assert_eq!(app.interface, before);
        assert_eq!(app.store.interface_preferences().unwrap(), before);
    }
    let sidebar_before = app
        .preferences_window
        .as_ref()
        .unwrap()
        .interface
        .sidebar_labels;
    assert!(ctx.memory(|memory| memory.focused()).is_some());
    for pressed in [true, false] {
        frame(
            &mut app,
            &ctx,
            egui::vec2(1080.0, 800.0),
            vec![egui::Event::Key {
                key: egui::Key::Space,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    assert_eq!(
        app.preferences_window
            .as_ref()
            .unwrap()
            .interface
            .sidebar_labels,
        !sidebar_before
    );
    for (label, theme) in [
        ("Light", Appearance::Light),
        ("Dark", Appearance::Dark),
        ("System", Appearance::System),
    ] {
        click(&mut app, &ctx, label);
        assert_eq!(
            app.preferences_window
                .as_ref()
                .unwrap()
                .interface
                .appearance,
            theme
        );
        assert_eq!(app.store.interface_preferences().unwrap(), before);
    }
    app.preferences_window.as_mut().unwrap().category = C::Downloads;
    click(&mut app, &ctx, "Download status window");
    click(&mut app, &ctx, "Save");
    assert_eq!(app.store.interface_preferences().unwrap(), app.interface);
    assert_eq!(
        app.interface.auto_download_status,
        !before.auto_download_status
    );
    assert_eq!(app.interface.compact_rows, !before.compact_rows);
    assert_eq!(app.interface.show_details, !before.show_details);
    for (category, label, expected) in [
        (C::Appearance, "Table columns", Dialog::Columns),
        (C::Downloads, "Download folders", Dialog::DownloadFolders),
        (C::Downloads, "Speed limits", Dialog::SpeedLimits),
        (C::Downloads, "Data quotas", Dialog::DataQuotas),
        (C::Connection, "Proxy", Dialog::Proxy),
        (C::Connection, "Download connections", Dialog::Connections),
        (C::Browser, "Browser integration", Dialog::Browsers),
        (C::Browser, "AI connection (MCP)", Dialog::Mcp),
        (C::Antivirus, "Antivirus scan", Dialog::AntivirusSettings),
    ] {
        app.open_preferences_window();
        app.preferences_window.as_mut().unwrap().category = category;
        click(&mut app, &ctx, label);
        assert_eq!(app.dialog, Some(expected), "{label}");
        click(&mut app, &ctx, "×");
        frame(&mut app, &ctx, SMALL, vec![]);
        assert_eq!(app.dialog, Some(Dialog::PreferencesWindow));
        assert_eq!(app.preferences_window.as_ref().unwrap().category, category);
        app.close_preferences_window(&ctx);
    }
}

#[test]
fn settings_groups_fit_minimum_width_and_scrolling_reaches_every_summary() {
    use preferences_window::Category as C;
    for appearance in [Appearance::Light, Appearance::Dark] {
        let (_temp, mut app, ctx) = fixture();
        app.interface.appearance = appearance;
        app.configure_appearance(&ctx);
        app.open_preferences_window();
        for (category, labels) in [
            (
                C::Appearance,
                vec![
                    "Compact rows",
                    "Details panel",
                    "Sidebar labels",
                    "6 of 9 shown",
                    "System",
                    "Light",
                    "Dark",
                ],
            ),
            (
                C::Downloads,
                vec!["Download folders", "Speed limits", "Data quotas"],
            ),
            (C::Connection, vec!["System proxy", "Download connections"]),
            (
                C::Browser,
                vec![
                    "Browser integration",
                    "Set up browsers",
                    "Clipboard links",
                    "AI connection (MCP)",
                ],
            ),
            (C::Antivirus, vec!["Antivirus scan"]),
            (C::Startup, vec!["Start with Windows", "Start in the tray"]),
        ] {
            app.preferences_window.as_mut().unwrap().category = category;
            let output = settled(&mut app, &ctx, SMALL);
            for label in labels {
                visible(&output, label, SMALL);
            }
        }
    }
}

#[test]
fn queue_cards_read_each_queue_and_route_actions_to_the_clicked_card() {
    let (_temp, mut app, ctx) = fixture();
    app.store.set_queue_running(MAIN_QUEUE, false).unwrap();
    let second = app.store.create_queue("Later").unwrap();
    let mut job = app.jobs[0].clone();
    job.total = Some(1000);
    job.downloaded = 200;
    app.store.enqueue_batch(&[job], MAIN_QUEUE).unwrap();
    let mut unknown = app.jobs[1].clone();
    unknown.status = Status::Paused;
    unknown.total = None;
    unknown.downloaded = 100;
    app.store.enqueue_batch(&[unknown], &second.id).unwrap();
    app.view = View::Queues;
    app.refresh_and_wait();
    let main = app.queue_summaries[MAIN_QUEUE].as_ref().unwrap();
    assert_eq!(
        (main.files, main.downloaded, main.total, main.unknown),
        (1, 200, 1000, 0)
    );
    let other = app.queue_summaries[&second.id].as_ref().unwrap();
    assert_eq!((other.files, other.downloaded, other.unknown), (1, 100, 1));
    let size = egui::vec2(1080.0, 800.0);
    let output = settled(&mut app, &ctx, size);
    visible(&output, "20%", size);
    visible(&output, "100 B downloaded · 1 file sizes unknown", size);
    let actions: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == "View downloads" => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .collect();
    assert_eq!(actions.len(), 2);
    app.search = "unrelated".into();
    click_at(&mut app, &ctx, size, actions[1]);
    app.wait_for_io();
    assert_eq!(app.view, View::Downloads);
    assert_eq!(app.queue_id, second.id);
    assert_eq!(app.search, "unrelated");
    assert_eq!(app.jobs.len(), 1);
    app.view = View::Queues;
    click(&mut app, &ctx, "⋯");
    let output = settled(&mut app, &ctx, size);
    visible(&output, "Download at once", size);
    click(&mut app, &ctx, "Rename queue");
    assert_eq!(app.dialog, Some(Dialog::RenameQueue));
    assert_eq!(app.queue_id, MAIN_QUEUE);
    assert_eq!(app.queue_name, "Main queue");
}

#[test]
fn queue_card_number_input_rejects_invalid_limits_and_updates_only_its_queue() {
    for dark in [false, true] {
        let (_temp, mut app, ctx) = fixture();
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        app.store.set_queue_running(MAIN_QUEUE, false).unwrap();
        let other = app.store.create_queue("Later").unwrap();
        app.view = View::Queues;
        app.refresh_and_wait();
        assert_eq!(app.queue_id, MAIN_QUEUE);

        for limit in [20, 30, 32, 1] {
            let output = settled(&mut app, &ctx, SMALL);
            let menus: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == "⋯" => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(menus.len(), 2);
            click_at(&mut app, &ctx, SMALL, menus[1]);
            let output = settled(&mut app, &ctx, SMALL);
            visible(&output, "Download at once", SMALL);
            let current = app
                .queues
                .iter()
                .find(|q| q.id == other.id)
                .unwrap()
                .max_active;
            click_at(
                &mut app,
                &ctx,
                SMALL,
                ctx.read_response(egui::Id::new(("queue-limit-input", &other.id)).with("editor"))
                    .unwrap()
                    .rect
                    .center(),
            );
            for typed in [
                "0".to_owned(),
                "33".to_owned(),
                "abc".to_owned(),
                limit.to_string(),
            ] {
                frame(
                    &mut app,
                    &ctx,
                    SMALL,
                    vec![
                        egui::Event::Key {
                            key: egui::Key::A,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers {
                                ctrl: true,
                                command: true,
                                ..Default::default()
                            },
                        },
                        egui::Event::Text(typed.clone()),
                    ],
                );
                let output = settled(&mut app, &ctx, SMALL);
                if typed == "30" {
                    for pressed in [true, false] {
                        frame(
                            &mut app,
                            &ctx,
                            SMALL,
                            vec![egui::Event::Key {
                                key: egui::Key::Enter,
                                physical_key: None,
                                pressed,
                                repeat: false,
                                modifiers: egui::Modifiers::NONE,
                            }],
                        );
                    }
                } else {
                    click_at(&mut app, &ctx, SMALL, text_rect(&output, "Set").center());
                }
                if typed != limit.to_string() {
                    assert_eq!(
                        app.store
                            .queues()
                            .unwrap()
                            .iter()
                            .find(|q| q.id == other.id)
                            .unwrap()
                            .max_active,
                        current
                    );
                    assert!(egui::Popup::is_any_open(&ctx));
                    settled(&mut app, &ctx, SMALL);
                    let input = ctx
                        .read_response(
                            egui::Id::new(("queue-limit-input", &other.id)).with("editor"),
                        )
                        .unwrap();
                    click_at(&mut app, &ctx, SMALL, input.rect.center());
                }
            }
            wait_for(&mut app, |app| {
                app.queues
                    .iter()
                    .any(|q| q.id == other.id && q.max_active == limit)
            });
            let saved = app.store.queues().unwrap();
            assert_eq!(
                saved.iter().find(|q| q.id == other.id).unwrap().max_active,
                limit
            );
            assert_eq!(
                saved
                    .iter()
                    .find(|q| q.id == MAIN_QUEUE)
                    .unwrap()
                    .max_active,
                3
            );
            assert_eq!(app.queue_id, MAIN_QUEUE);
            assert!(!egui::Popup::is_any_open(&ctx));
        }
    }
}
