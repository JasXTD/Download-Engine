use super::*;
use crate::desktop::interaction_tests::{click as ui_click, fixture, frame, text_rect};

fn click(app: &mut Desktop, ctx: &egui::Context, label: &str) {
    // These assertions inspect the settled UI, not scroll animation timing.
    ctx.all_styles_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
    if matches!(
        label,
        "Allow changes" | "View only" | "Start with Download Engine" | "Save settings"
    ) && !app.mcp.advanced_open
    {
        ui_click(app, ctx, "Advanced settings");
        frame(app, ctx, SIZE, vec![]);
        assert!(
            app.mcp.advanced_open,
            "Advanced settings must open when clicked"
        );
        for _ in 0..30 {
            frame(app, ctx, SIZE, vec![]);
            if !app.mcp.advanced_reveal {
                break;
            }
        }
        assert!(
            !app.mcp.advanced_reveal,
            "Expanded controls should be revealed"
        );
    }
    ui_click(app, ctx, label);
}
use serde_json::{Value, json};

const SIZE: egui::Vec2 = egui::vec2(1080.0, 800.0);

fn port() -> u16 {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
fn settle(app: &mut Desktop, ctx: &egui::Context, ready: impl Fn(&State) -> bool) {
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        frame(app, ctx, SIZE, vec![]);
        if ready(&app.mcp) {
            return;
        }
        assert!(
            Instant::now() < until,
            "MCP control did not settle: {:?}; {:?}",
            app.mcp.status,
            app.mcp.error
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn copy(app: &mut Desktop, ctx: &egui::Context, label: &str) -> String {
    // Unlike click(), retain the release frame in case the operation is immediate.
    frame(app, ctx, SIZE, vec![]);
    let output = frame(app, ctx, SIZE, vec![]);
    let pos = text_rect(
        &output,
        if label == "Copy URL" {
            "\u{e8c8}"
        } else {
            label
        },
    )
    .center();
    let mut copied = None;
    for pressed in [true, false] {
        let output = frame(
            app,
            ctx,
            SIZE,
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
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(text) = copied {
            return text;
        }
        let output = frame(app, ctx, SIZE, vec![]);
        for command in output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                copied = Some(text);
            }
        }
        assert!(Instant::now() < deadline, "Nothing copied from {label}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn rpc(config: &Value, name: &str, args: Value) -> Value {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let response=reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(8)).build().unwrap()
            .post(config["url"].as_str().unwrap())
            .header("Authorization",config["headers"]["Authorization"].as_str().unwrap())
            .header("Content-Type","application/json").header("Accept","application/json, text/event-stream")
            .header("MCP-Protocol-Version","2026-07-28").header("Mcp-Method","tools/call").header("Mcp-Name",name)
            .body(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args,"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}).to_string())
            .send().await.unwrap().error_for_status().unwrap().text().await.unwrap();
        serde_json::from_str(response.lines().find_map(|l|l.strip_prefix("data: ")).unwrap_or(&response)).unwrap()
    })
}

#[test]
fn mcp_controls_copy_working_config_enforce_access_and_use_the_scheduler() {
    let (_root, mut app, ctx) = fixture();
    let selected = app.selected.clone();
    click(&mut app, &ctx, "MCP");
    assert_eq!(app.view, View::Downloads);
    assert_eq!(app.dialog, Some(Dialog::Mcp));
    assert_eq!(app.mcp.status, ServerStatus::Stopped);
    assert!(app.mcp.read_only && !app.mcp.start_with_app);
    app.mcp.port = "0".into();
    click(&mut app, &ctx, "Start server");
    assert_eq!(
        app.mcp.error.as_deref(),
        Some("Enter a port from 1 to 65535")
    );
    let error_frame = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
    assert!(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(860.0, 600.0))
            .contains_rect(text_rect(&error_frame, "Enter a port from 1 to 65535"))
    );
    assert!(!app.store.paths.data.join("mcp-token.dpapi").exists());
    let port = port();
    app.mcp.port = port.to_string();
    click(&mut app, &ctx, "Start server");
    settle(&mut app, &ctx, |s| s.status == ServerStatus::Running);
    click(&mut app, &ctx, "Allow changes");
    assert!(
        app.mcp.read_only,
        "Running server settings must stay locked"
    );
    click(&mut app, &ctx, "Check connection");
    settle(&mut app, &ctx, |s| {
        s.pending.is_none()
            && s.notice
                .as_deref()
                .is_some_and(|s| s.starts_with("Connection checked"))
    });
    let copied = copy(&mut app, &ctx, "Copy client config");
    let config: Value = serde_json::from_str(&copied).unwrap();
    let config = &config["mcpServers"]["download-engine"];
    assert_eq!(config["url"], format!("http://127.0.0.1:{port}/mcp"));
    assert_eq!(copy(&mut app, &ctx, "Copy URL"), config["url"]);
    assert_eq!(
        copy(&mut app, &ctx, "Copy access header"),
        config["headers"]["Authorization"]
    );
    let reply = rpc(config, "create_queue", json!({"name":"AI control test"}));
    assert_eq!(reply["result"]["isError"], true);
    assert!(
        !app.store
            .queues()
            .unwrap()
            .iter()
            .any(|q| q.name == "AI control test")
    );
    let out = frame(&mut app, &ctx, SIZE, vec![]);
    assert!(!out.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text.contains(config["headers"]["Authorization"].as_str().unwrap()))));
    click(&mut app, &ctx, "Stop server");
    settle(&mut app, &ctx, |s| s.status == ServerStatus::Stopped);
    assert!(mcp::check_connection(&app.store.paths, port).is_err());
    click(&mut app, &ctx, "Allow changes");
    click(&mut app, &ctx, "Start with Download Engine");
    click(&mut app, &ctx, "Start server");
    settle(&mut app, &ctx, |s| s.status == ServerStatus::Running);
    assert_eq!(
        app.store.mcp_settings().unwrap(),
        Settings {
            port,
            read_only: false,
            permissions: Default::default(),
            start_with_app: true
        }
    );
    let reply = rpc(
        config,
        "create_queue",
        json!({"name":"AI control test","request_id":Uuid::new_v4().to_string(),"wait_seconds":5}),
    );
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    assert!(
        app.store
            .queues()
            .unwrap()
            .iter()
            .any(|q| q.name == "AI control test" && !q.running)
    );
    click(&mut app, &ctx, "Stop server");
    settle(&mut app, &ctx, |s| s.status == ServerStatus::Stopped);
    app.mcp = State::new(&app.store);
    settle(&mut app, &ctx, |s| s.status == ServerStatus::Running);
    assert_eq!(mcp::check_connection(&app.store.paths, port).unwrap(), 19);
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none());
    assert_eq!(app.view, View::Downloads);
    assert_eq!(app.selected, selected);
    assert_eq!(app.mcp.status, ServerStatus::Running);
    drop(app);
    let _released = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
}

#[test]
fn mcp_errors_do_not_claim_running_or_save_success_and_can_be_retried() {
    let (_root, mut app, ctx) = fixture();
    click(&mut app, &ctx, "Tools");
    click(&mut app, &ctx, "MCP control");
    assert_eq!(app.dialog, Some(Dialog::Mcp));
    assert_eq!(app.view, View::Downloads);
    let reserved = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = reserved.local_addr().unwrap().port();
    app.mcp.port = port.to_string();
    click(&mut app, &ctx, "Start server");
    settle(&mut app, &ctx, |s| {
        matches!(s.status, ServerStatus::Failed(_))
    });
    let out = frame(&mut app, &ctx, SIZE, vec![]);
    text_rect(&out, "Could not start");
    drop(reserved);
    click(&mut app, &ctx, "Start server");
    settle(&mut app, &ctx, |s| s.status == ServerStatus::Running);
    click(&mut app, &ctx, "Check connection");
    click(&mut app, &ctx, "Stop server");
    settle(&mut app, &ctx, |s| {
        s.pending.is_none() && s.status == ServerStatus::Stopped
    });
    assert!(
        app.mcp.error.is_none(),
        "Stopping a check must not report a connection failure"
    );
    assert!(
        !app.mcp
            .notice
            .as_deref()
            .is_some_and(|n| n.starts_with("Connection checked"))
    );
    // A real database failure must not enable startup or launch an unsaved server.
    crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3"))
        .unwrap()
        .execute("DROP TABLE mcp_settings", [])
        .unwrap();
    click(&mut app, &ctx, "Start with Download Engine");
    click(&mut app, &ctx, "Save settings");
    settle(&mut app, &ctx, |s| s.pending.is_none() && s.error.is_some());
    assert!(!app.mcp.saved.start_with_app);
    assert_eq!(app.mcp.status, ServerStatus::Stopped);
}

#[test]
fn localized_mcp_status_and_actions_fit_without_overlapping() {
    use download_engine::i18n::Language;
    let (_root, mut app, ctx) = fixture();
    app.open_mcp();
    for language in [
        Language::English,
        Language::SimplifiedChinese,
        Language::TraditionalChinese,
        Language::Malay,
    ] {
        app.interface.language = language;
        let _scope = download_engine::i18n::scope(language);
        for running in [false, true] {
            app.mcp.read_only = !running;
            if running {
                app.mcp.port = port().to_string();
                app.mcp.read_only = false;
                app.mcp.act(Action::Start, &app.store, &ctx);
                settle(&mut app, &ctx, |s| s.status == ServerStatus::Running);
            }
            // Use a real server on an isolated loopback port; page polling must
            // see the same state being rendered. No AI clients are connected.
            for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
                frame(&mut app, &ctx, size, vec![]);
                let out = frame(&mut app, &ctx, size, vec![]);
                let detail = text_rect(
                    &out,
                    tr(if running {
                        "·  Allow changes  ·  This computer only"
                    } else {
                        "·  View only  ·  This computer only"
                    }),
                );
                let action = text_rect(
                    &out,
                    tr(if running {
                        "Stop server"
                    } else {
                        "Start server"
                    }),
                );
                assert!(
                    detail.right() < action.left(),
                    "{language:?}: MCP status overlaps its action"
                );
                assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(action));
            }
            if running {
                app.mcp.act(Action::Stop, &app.store, &ctx);
                settle(&mut app, &ctx, |s| s.status == ServerStatus::Stopped);
            }
        }
    }
}

#[test]
fn mcp_popup_and_toolbar_fit_minimum_windows_in_both_themes() {
    let (_root, mut app, ctx) = fixture();
    app.open_mcp();
    for dark in [true, false] {
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
            app.mcp.advanced_open = false;
            for _ in 0..3 {
                frame(&mut app, &ctx, size, vec![]);
            }
            let out = frame(&mut app, &ctx, size, vec![]);
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            for label in [
                "Start server",
                "Server URL",
                "\u{e8c8}",
                "Advanced settings",
                "Setup guide",
                "Open logs",
            ] {
                let rect = text_rect(&out, label);
                assert!(viewport.contains_rect(rect), "Clipped: {label}");
                assert!(
                    out.shapes
                        .iter()
                        .any(|s| matches!(&s.shape, egui::Shape::Text(t)
                    if t.galley.job.text.trim() == label && s.clip_rect.contains_rect(rect))),
                    "Not visible: {label}"
                );
            }
            // Explicitly expand and scroll to the separately grouped configuration.
            app.mcp.advanced_open = true;
            frame(&mut app, &ctx, size, vec![]);
            frame(
                &mut app,
                &ctx,
                size,
                vec![
                    egui::Event::PointerMoved(egui::pos2(size.x / 2.0, size.y / 2.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(0.0, -1500.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            for _ in 0..20 {
                frame(&mut app, &ctx, size, vec![]);
            }
            let out = frame(&mut app, &ctx, size, vec![]);
            for label in [
                "Start server",
                "Port",
                "View only",
                "Allow changes",
                "Save settings",
            ] {
                assert!(
                    viewport.contains_rect(text_rect(&out, label)),
                    "Advanced control unreachable: {label}"
                );
            }
            for label in ["View only", "Allow changes"] {
                assert!(
                    (text_rect(&out, label).center().y - text_rect(&out, "AI access").center().y)
                        .abs()
                        <= 1.0
                );
            }
            for clipped in &out.shapes {
                if let egui::Shape::Text(text) = &clipped.shape {
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    if rect.intersects(clipped.clip_rect) {
                        assert!(
                            rect.right() <= size.x + 1.0,
                            "Horizontal overflow: {}",
                            text.galley.job.text
                        );
                    }
                }
            }
        }
    }
    click(&mut app, &ctx, "Setup guide");
    assert_eq!(app.view, View::Guide);
    assert!(app.dialog.is_none());
}

#[test]
fn mcp_url_and_first_prompt_copy_before_start_without_creating_credentials() {
    let (_root, mut app, ctx) = fixture();
    app.open_mcp();
    let saved = app.mcp.saved;
    let port = port();
    app.mcp.port = port.to_string();
    assert_eq!(
        copy(&mut app, &ctx, "Copy URL"),
        format!("http://127.0.0.1:{port}/mcp")
    );
    assert_eq!(copy(&mut app, &ctx, FIRST_PROMPT), FIRST_PROMPT);
    assert_eq!(app.mcp.status, ServerStatus::Stopped);
    assert_eq!(app.store.mcp_settings().unwrap(), saved);
    assert!(!app.store.paths.data.join("mcp-token.dpapi").exists());
    app.mcp.port = "0".into();
    let out = frame(&mut app, &ctx, SIZE, vec![]);
    text_rect(&out, "Enter a valid port in Advanced settings");
    app.mcp.notice = None;
    click(&mut app, &ctx, "\u{e8c8}");
    assert!(
        app.mcp.notice.is_none(),
        "An invalid port must disable URL copying"
    );
}

#[test]
fn mcp_popup_keeps_the_current_page_draft_and_pending_server_when_closed() {
    let (_root, mut app, ctx) = fixture();
    app.view = View::Queues;
    let selected = app.selected.clone();
    let saved = app.store.mcp_settings().unwrap();
    click(&mut app, &ctx, "MCP");
    assert_eq!(app.view, View::Queues);
    assert_eq!(app.dialog, Some(Dialog::Mcp));
    let port = port();
    app.mcp.port = port.to_string();
    app.mcp.read_only = false;
    for pressed in [true, false] {
        frame(
            &mut app,
            &ctx,
            SIZE,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    assert!(app.dialog.is_none());
    assert_eq!(app.view, View::Queues);
    assert_eq!(app.selected, selected);
    assert_eq!(app.store.mcp_settings().unwrap(), saved);
    assert!(!app.store.paths.data.join("mcp-token.dpapi").exists());
    click(&mut app, &ctx, "Tools");
    click(&mut app, &ctx, "MCP control");
    assert_eq!(app.mcp.port, port.to_string());
    assert!(!app.mcp.read_only);
    // Close during startup, before the background worker is collected by the UI.
    app.mcp.act(Action::Start, &app.store, &ctx);
    assert!(app.mcp.pending.is_some());
    app.dialog = None;
    let until = Instant::now() + Duration::from_secs(8);
    while app.mcp.pending.is_some() || app.mcp.status != ServerStatus::Running {
        app.background(&ctx);
        frame(&mut app, &ctx, SIZE, vec![]);
        assert!(
            Instant::now() < until,
            "Closed popup lost the startup worker"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(mcp::check_connection(&app.store.paths, port).unwrap(), 19);
    assert_eq!(app.view, View::Queues);
    assert!(app.dialog.is_none());
    click(&mut app, &ctx, "MCP");
    click(&mut app, &ctx, "Open logs");
    assert_eq!(app.view, View::Logs);
    assert!(app.dialog.is_none());
    assert_eq!(app.mcp.status, ServerStatus::Running);
    click(&mut app, &ctx, "MCP");
    click(&mut app, &ctx, "Stop server");
    settle(&mut app, &ctx, |s| {
        s.pending.is_none() && s.status == ServerStatus::Stopped
    });
    assert!(mcp::check_connection(&app.store.paths, port).is_err());
}

#[test]
fn mcp_popup_returns_to_the_unsaved_preferences_draft() {
    use crate::desktop::preferences_window::Category;
    let (_root, mut app, ctx) = fixture();
    app.view = View::Scheduler;
    let saved = app.store.interface_preferences().unwrap();
    app.open_preferences_window();
    let draft = app.preferences_window.as_mut().unwrap();
    draft.interface.compact_rows = !saved.compact_rows;
    draft.category = Category::Browser;
    click(&mut app, &ctx, "AI connection (MCP)");
    assert_eq!(app.dialog, Some(Dialog::Mcp));
    app.mcp.port = port().to_string();
    click(&mut app, &ctx, "Save settings");
    settle(&mut app, &ctx, |s| {
        s.pending.is_none() && s.draft().ok() == Some(s.saved)
    });
    let mcp_saved = app.store.mcp_settings().unwrap();
    assert_eq!(mcp_saved.port.to_string(), app.mcp.port);
    click(&mut app, &ctx, "×");
    frame(&mut app, &ctx, SIZE, vec![]);
    assert_eq!(app.dialog, Some(Dialog::PreferencesWindow));
    let draft = app.preferences_window.as_ref().unwrap();
    assert_eq!(draft.category, Category::Browser);
    assert_eq!(draft.interface.compact_rows, !saved.compact_rows);
    assert_eq!(app.store.interface_preferences().unwrap(), saved);
    click(&mut app, &ctx, "Cancel");
    assert!(app.dialog.is_none());
    assert_eq!(app.view, View::Scheduler);
    assert_eq!(app.interface.compact_rows, saved.compact_rows);
    assert_eq!(app.store.mcp_settings().unwrap(), mcp_saved);
}
