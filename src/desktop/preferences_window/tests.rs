use super::*;
use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};

fn click_torrent_network_option(app: &mut Desktop, ctx: &egui::Context, label: &str) {
    let size = vec2(1080.0, 800.0);
    for _ in 0..40 {
        frame(app, ctx, size, vec![]);
        let output = frame(app, ctx, size, vec![]);
        let visible = output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text)
                if text.galley.job.text.trim() == label
                    && shape.clip_rect.contains_rect(text.galley.rect.translate(text.pos.to_vec2())))
        });
        if visible {
            click(app, ctx, label);
            return;
        }
        frame(
            app,
            ctx,
            size,
            vec![
                egui::Event::PointerMoved(egui::pos2(650.0, 400.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: vec2(0.0, -100.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    panic!("Torrent option never became visible: {label}");
}

#[test]
fn torrent_warning_cancel_close_escape_and_backdrop_preserve_the_draft() {
    for dismissal in ["Cancel", "×", "escape", "backdrop"] {
        let (_temp, mut app, ctx) = fixture();
        let original = app.store.torrent_settings().unwrap();
        let saved_interface = app.store.interface_preferences().unwrap();
        app.open_preferences_window();
        let draft = app.preferences_window.as_mut().unwrap();
        draft.category = Category::Torrents;
        draft.torrents.enabled = true;
        draft.interface.compact_rows = !app.interface.compact_rows;
        click_torrent_network_option(&mut app, &ctx, "Allow incoming connections");
        let draft = app.preferences_window.as_ref().unwrap();
        assert_eq!(draft.network_warning, Some(TorrentNetworkWarning::Incoming));
        assert!(draft.torrents.incoming_port.is_none());
        // A shortcut or another caller must not save behind the warning.
        app.save_preferences_window(&ctx);
        assert_eq!(app.store.interface_preferences().unwrap(), saved_interface);
        let size = vec2(1080.0, 800.0);
        frame(&mut app, &ctx, size, vec![]);
        let output = frame(&mut app, &ctx, size, vec![]);
        text_rect(&output, "Allow incoming connections?");
        if dismissal == "escape" {
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
        } else if dismissal == "backdrop" {
            let pos = egui::pos2(20.0, 20.0);
            for pressed in [true, false] {
                frame(
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
            }
        } else {
            click(&mut app, &ctx, dismissal);
        }
        let draft = app.preferences_window.as_ref().unwrap();
        assert!(draft.network_warning.is_none(), "{dismissal}");
        assert!(draft.torrents.incoming_port.is_none());
        assert!(!draft.torrents.upnp);
        assert_ne!(draft.interface.compact_rows, app.interface.compact_rows);
        assert_eq!(app.dialog, Some(Dialog::PreferencesWindow));
        assert_eq!(app.store.torrent_settings().unwrap(), original);
        assert!(!app.store.torrents.is_running());
    }
}

#[test]
fn torrent_network_confirmation_requires_save_and_does_not_enable_uploads() {
    let (_temp, mut app, ctx) = fixture();
    let original = app.store.torrent_settings().unwrap();
    app.open_preferences_window();
    let draft = app.preferences_window.as_mut().unwrap();
    draft.category = Category::Torrents;
    draft.torrents.enabled = true;
    click_torrent_network_option(&mut app, &ctx, "Allow incoming connections");
    click(&mut app, &ctx, "Enable");
    assert_eq!(
        app.preferences_window
            .as_ref()
            .unwrap()
            .torrents
            .incoming_port,
        Some(6881)
    );
    assert_eq!(app.store.torrent_settings().unwrap(), original);
    click_torrent_network_option(&mut app, &ctx, "Set up my router (UPnP)");
    assert_eq!(
        app.preferences_window.as_ref().unwrap().network_warning,
        Some(TorrentNetworkWarning::Upnp)
    );
    assert!(!app.preferences_window.as_ref().unwrap().torrents.upnp);
    click(&mut app, &ctx, "Cancel");
    assert!(!app.preferences_window.as_ref().unwrap().torrents.upnp);
    click_torrent_network_option(&mut app, &ctx, "Set up my router (UPnP)");
    click(&mut app, &ctx, "Enable");
    assert!(app.preferences_window.as_ref().unwrap().torrents.upnp);
    assert_eq!(app.store.torrent_settings().unwrap(), original);
    click(&mut app, &ctx, "Save");
    let saved = app.store.torrent_settings().unwrap();
    assert_eq!(saved.incoming_port, Some(6881));
    assert!(saved.upnp && saved.enabled);
    assert!(!saved.upload);
    assert!(
        !app.store.torrents.is_running(),
        "No torrent jobs: no helper, listener or router mapping"
    );
    app.open_preferences_window();
    app.preferences_window.as_mut().unwrap().category = Category::Torrents;
    assert!(
        app.preferences_window
            .as_ref()
            .unwrap()
            .network_warning
            .is_none()
    );
    // Turning options off never asks; turning them on again must ask again.
    click_torrent_network_option(&mut app, &ctx, "Set up my router (UPnP)");
    assert!(!app.preferences_window.as_ref().unwrap().torrents.upnp);
    assert!(
        app.preferences_window
            .as_ref()
            .unwrap()
            .network_warning
            .is_none()
    );
    click_torrent_network_option(&mut app, &ctx, "Set up my router (UPnP)");
    click(&mut app, &ctx, "Enable");
    click_torrent_network_option(&mut app, &ctx, "Allow incoming connections");
    let draft = app.preferences_window.as_ref().unwrap();
    assert!(draft.torrents.incoming_port.is_none() && !draft.torrents.upnp);
    assert!(draft.network_warning.is_none());
    click_torrent_network_option(&mut app, &ctx, "Allow incoming connections");
    click(&mut app, &ctx, "Enable");
    assert!(!app.preferences_window.as_ref().unwrap().torrents.upnp);
    click(&mut app, &ctx, "×");
    assert_eq!(
        app.store.torrent_settings().unwrap(),
        saved,
        "Closing discards confirmed but unsaved edits"
    );
}

#[test]
fn torrent_warning_rechecks_prerequisites_and_reset_disables_sharing() {
    let (_temp, mut app, _ctx) = fixture();
    app.open_preferences_window();
    let draft = app.preferences_window.as_mut().unwrap();
    draft.category = Category::Torrents;
    draft.network_warning = Some(TorrentNetworkWarning::Incoming);
    draft.confirm_network_warning();
    assert!(
        draft.torrents.incoming_port.is_none(),
        "Torrents remain disabled"
    );
    draft.torrents.enabled = true;
    draft.network_warning = Some(TorrentNetworkWarning::Upnp);
    draft.confirm_network_warning();
    assert!(!draft.torrents.upnp, "UPnP requires incoming connections");
    draft.torrents.incoming_port = Some(6881);
    draft.torrents.binding = Some(de_torrent::network::Binding {
        interface: "offline-test-adapter".into(),
        address: "192.0.2.2".parse().unwrap(),
        dns: "192.0.2.53".parse().unwrap(),
    });
    draft.network_warning = Some(TorrentNetworkWarning::Upnp);
    draft.confirm_network_warning();
    assert!(!draft.torrents.upnp, "A bound interface cannot enable UPnP");
    draft.torrents.upload = true;
    draft.torrents.seed_completed = true;
    draft.network_warning = Some(TorrentNetworkWarning::Incoming);
    draft.reset_category();
    assert_eq!(
        draft.torrents,
        download_engine::torrent::Settings::default()
    );
    assert!(!draft.torrents.upload && !draft.torrents.seed_completed && !draft.torrents.upnp);
    assert!(draft.torrents.incoming_port.is_none() && draft.network_warning.is_none());
}

#[test]
fn torrent_binding_validates_dns_before_saving_and_cancel_preserves_policy() {
    let (_temp, mut app, ctx) = fixture();
    let original = app.store.torrent_settings().unwrap();
    let interface = app.interface;
    app.open_preferences_window();
    let draft = app.preferences_window.as_mut().unwrap();
    draft.category = Category::Torrents;
    draft.torrents.enabled = true;
    draft.torrents.binding = Some(de_torrent::network::Binding {
        interface: "offline-test-adapter".into(),
        address: "192.0.2.2".parse().unwrap(),
        dns: "192.0.2.53".parse().unwrap(),
    });
    draft.interface.compact_rows = !interface.compact_rows;
    for invalid in ["dns.example.test", "", "::1", "0.0.0.0"] {
        app.preferences_window.as_mut().unwrap().binding_dns = invalid.into();
        app.save_preferences_window(&ctx);
        assert!(app.preferences_window.as_ref().unwrap().error.is_some());
        assert_eq!(app.store.torrent_settings().unwrap(), original);
        assert_eq!(
            app.interface, interface,
            "Invalid DNS must not partially save preferences"
        );
    }
    app.preferences_window.as_mut().unwrap().binding_dns = "192.0.2.53".into();
    app.save_preferences_window(&ctx);
    let saved = app.store.torrent_settings().unwrap();
    assert_eq!(
        saved.binding.as_ref().unwrap().dns.to_string(),
        "192.0.2.53"
    );
    assert!(
        !app.store.torrents.is_running(),
        "Saving a missing interface must never start a helper or fall back"
    );
    app.open_preferences_window();
    app.preferences_window.as_mut().unwrap().torrents.binding = None;
    app.close_preferences_window(&ctx);
    assert_eq!(app.store.torrent_settings().unwrap(), saved);
}

#[cfg(unix)]
#[test]
fn native_preferences_offer_updates_without_unavailable_windows_integrations() {
    let (_temp, mut app, ctx) = fixture();
    app.open_preferences_window();
    for category in [Category::Startup, Category::Browser] {
        app.preferences_window.as_mut().unwrap().category = category;
        frame(&mut app, &ctx, vec2(1080.0, 800.0), vec![]);
        let output = frame(&mut app, &ctx, vec2(1080.0, 800.0), vec![]);
        for unavailable in [
            "Start with Windows",
            "Start in the tray",
            "Clipboard links",
            "Close to tray",
            "Notify when a download finishes",
        ] {
            assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == unavailable)), "Unavailable native control: {unavailable}");
        }
        text_rect(
            &output,
            if category == Category::Startup {
                "Check for updates on launch"
            } else {
                "Browser integration"
            },
        );
    }
}

#[test]
fn launch_update_preference_saves_cancels_and_resets_without_changing_windows_startup() {
    let (_temp, mut app, ctx) = fixture();
    let windows_startup = app.startup.enabled();
    assert!(app.interface.check_updates_on_launch);
    app.open_preferences_window();
    click(&mut app, &ctx, Category::Startup.label());
    click(&mut app, &ctx, "Check for updates on launch");
    click(&mut app, &ctx, "×");
    assert!(app.interface.check_updates_on_launch);
    app.open_preferences_window();
    click(&mut app, &ctx, Category::Startup.label());
    click(&mut app, &ctx, "Check for updates on launch");
    click(&mut app, &ctx, "Save");
    assert!(!app.interface.check_updates_on_launch);
    assert!(
        !app.store
            .interface_preferences()
            .unwrap()
            .check_updates_on_launch
    );
    assert_eq!(app.startup.enabled(), windows_startup);
    app.open_preferences_window();
    app.preferences_window.as_mut().unwrap().category = Category::Startup;
    app.preferences_window.as_mut().unwrap().reset_category();
    assert!(
        app.preferences_window
            .as_ref()
            .unwrap()
            .interface
            .check_updates_on_launch
    );
    click(&mut app, &ctx, "×");
    assert!(!app.interface.check_updates_on_launch);
}

#[test]
fn independently_saved_folder_rules_are_reported_and_survive_parent_cancel() {
    let (_root, mut app, ctx) = fixture();
    app.open_preferences_window();
    click(&mut app, &ctx, "Compact rows");
    click(&mut app, &ctx, "Downloads");
    click(&mut app, &ctx, "Download folders");
    click(&mut app, &ctx, "Save folder rules");
    let saved = app.store.download_folders().unwrap();
    let output = frame(&mut app, &ctx, vec2(1080.0, 800.0), vec![]);
    assert!(app.preferences_window.as_ref().unwrap().advanced_saved);
    text_rect(
        &output,
        "Advanced settings saved. Closing Preferences will not undo those changes.",
    );
    click(&mut app, &ctx, "×");
    assert!(!app.interface.compact_rows);
    assert_eq!(app.store.download_folders().unwrap(), saved);
    app.open_preferences_window();
    click(&mut app, &ctx, "Downloads");
    click(&mut app, &ctx, "Download folders");
    click(&mut app, &ctx, "×");
    frame(&mut app, &ctx, vec2(1080.0, 800.0), vec![]);
    assert!(
        !app.preferences_window.as_ref().unwrap().advanced_saved,
        "Closing without saving must not report a save"
    );
}

#[test]
fn cancel_and_escape_discard_drafts_and_restore_theme_on_the_same_page() {
    for escape in [false, true] {
        let (_temp, mut app, ctx) = fixture();
        // A real queue refresh must see the selected download in storage too.
        // The shared layout fixture otherwise contains in-memory rows only.
        for job in &app.jobs {
            app.store.enqueue(job).unwrap();
        }
        app.view = View::Queues;
        app.save_interface(
            &ctx,
            InterfacePreferences {
                appearance: Appearance::Light,
                ..app.interface
            },
        );
        let before = app.interface;
        let selected = app.selected.clone();
        app.open_preferences_window();
        click(&mut app, &ctx, "Dark");
        click(&mut app, &ctx, "Compact rows");
        click(&mut app, &ctx, "Browser & AI");
        #[cfg(windows)]
        click(&mut app, &ctx, "Clipboard links");
        assert_eq!(
            app.preferences_window.as_ref().unwrap().category,
            Category::Browser
        );
        assert!(
            app.preferences_window
                .as_ref()
                .unwrap()
                .interface
                .compact_rows
        );
        assert_eq!(app.interface, before);
        assert_eq!(app.store.interface_preferences().unwrap(), before);
        assert_eq!(ctx.theme(), egui::Theme::Dark, "Draft theme should preview");
        if escape {
            frame(
                &mut app,
                &ctx,
                vec2(1080.0, 800.0),
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        } else {
            click(&mut app, &ctx, "×");
        }
        frame(&mut app, &ctx, vec2(1080.0, 800.0), vec![]);
        assert!(app.preferences_window.is_none() && app.dialog.is_none());
        assert_eq!(app.view, View::Queues);
        assert_eq!(app.selected, selected);
        assert_eq!(
            ctx.theme(),
            egui::Theme::Light,
            "Closing should restore the saved theme"
        );
        assert_eq!(app.store.interface_preferences().unwrap(), before);
    }
}

#[test]
fn columns_and_reset_stay_in_the_parent_draft_until_save() {
    let (_temp, mut app, ctx) = fixture();
    app.store.save_interface_preferences(app.interface).unwrap();
    let before = app.interface;
    app.open_preferences_window();
    click(&mut app, &ctx, "Compact rows");
    click(&mut app, &ctx, "Table columns");
    assert_eq!(app.dialog, Some(Dialog::Columns));
    click(&mut app, &ctx, "Size");
    click(&mut app, &ctx, "Save");
    frame(&mut app, &ctx, vec2(1080.0, 800.0), vec![]);
    let draft = app.preferences_window.as_ref().unwrap().interface;
    assert!(draft.compact_rows);
    assert_ne!(draft.columns, before.columns);
    assert_eq!(app.interface, before);
    assert_eq!(app.store.interface_preferences().unwrap(), before);
    click(&mut app, &ctx, "Reset");
    assert_eq!(app.store.interface_preferences().unwrap(), before);
    assert!(
        !app.preferences_window
            .as_ref()
            .unwrap()
            .interface
            .compact_rows
    );
    click(&mut app, &ctx, "×");
    assert_eq!(app.interface, before);
    app.open_preferences_window();
    click(&mut app, &ctx, "Compact rows");
    // Simulate an independently saved window position while Preferences is open.
    app.interface.drop_target_position = Some([120, 180]);
    app.store.save_interface_preferences(app.interface).unwrap();
    click(&mut app, &ctx, "Save");
    assert!(app.interface.compact_rows);
    assert_eq!(app.interface.drop_target_position, Some([120, 180]));
    assert_eq!(app.store.interface_preferences().unwrap(), app.interface);
}

#[test]
fn atomic_save_error_keeps_both_records_and_the_draft_for_retry() {
    let (_temp, mut app, ctx) = fixture();
    app.store.save_interface_preferences(app.interface).unwrap();
    let before = app.interface;
    let behavior = app.preferences;
    app.open_preferences_window();
    click(&mut app, &ctx, "Compact rows");
    app.preferences_window
        .as_mut()
        .unwrap()
        .behavior
        .notify_complete = false;
    let conn =
        crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3")).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_preferences BEFORE UPDATE ON interface_preferences BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
    click(&mut app, &ctx, "Save");
    assert_eq!(app.interface, before);
    assert_eq!(app.preferences, behavior);
    assert_eq!(app.store.interface_preferences().unwrap(), before);
    assert_eq!(app.store.preferences().unwrap(), behavior);
    let draft = app.preferences_window.as_ref().unwrap();
    assert!(draft.dirty() && draft.interface.compact_rows);
    assert!(
        draft
            .error
            .as_ref()
            .unwrap()
            .contains("fixture write failure")
    );
    conn.execute_batch("DROP TRIGGER reject_preferences")
        .unwrap();
    click(&mut app, &ctx, "Save");
    assert!(app.dialog.is_none());
    assert!(app.interface.compact_rows);
    assert!(!app.preferences.notify_complete);
}

#[test]
fn startup_failure_reports_partial_save_and_can_be_retried_without_real_registry() {
    let (_temp, mut app, ctx) = fixture();
    app.open_preferences_window();
    click(&mut app, &ctx, "Compact rows");
    app.preferences_window.as_mut().unwrap().startup = true;
    let conn =
        crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3")).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_startup BEFORE UPDATE ON windows_startup BEGIN SELECT RAISE(ABORT, 'fixture startup failure'); END;").unwrap();
    click(&mut app, &ctx, "Save");
    assert!(app.interface.compact_rows && app.store.interface_preferences().unwrap().compact_rows);
    let draft = app.preferences_window.as_ref().unwrap();
    assert!(draft.dirty());
    assert_eq!(draft.category, Category::Startup);
    assert!(
        draft
            .error
            .as_ref()
            .unwrap()
            .contains("Preferences saved, but Windows startup")
    );
    conn.execute_batch("DROP TRIGGER reject_startup").unwrap();
    click(&mut app, &ctx, "Save");
    assert!(app.dialog.is_none());
    assert!(app.startup.enabled());
    assert!(
        app.store
            .startup_preferences()
            .unwrap()
            .registered
            .is_some()
    );
}

#[test]
fn all_languages_categories_and_footer_fit_the_minimum_window() {
    let size = vec2(860.0, 600.0);
    let (_temp, mut app, ctx) = fixture();
    for language in download_engine::i18n::Language::ALL {
        app.interface.language = language;
        let _scope = download_engine::i18n::scope(language);
        app.open_preferences_window();
        for category in Category::ALL {
            app.preferences_window.as_mut().unwrap().category = category;
            frame(&mut app, &ctx, size, vec![]);
            let output = frame(&mut app, &ctx, size, vec![]);
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            for label in [tr("Save"), tr("Reset"), "×"] {
                assert!(
                    viewport.contains_rect(text_rect(&output, label)),
                    "{language:?} {category:?}: {label}"
                );
            }
            for clipped in &output.shapes {
                if let egui::epaint::Shape::Text(text) = &clipped.shape {
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    if rect.intersects(clipped.clip_rect) {
                        assert!(
                            rect.right() <= size.x + 1.0,
                            "{language:?} {category:?} overflow: {}",
                            text.galley.job.text
                        );
                    }
                }
            }
        }
        app.close_preferences_window(&ctx);
    }
}

const SIZE: egui::Vec2 = vec2(1080.0, 800.0);

/// Runs one frame at an explicit input time, so a press can be held across frames.
fn frame_at(
    app: &mut Desktop,
    ctx: &egui::Context,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    output.textures_delta.clear();
    output
}

/// The dialog's copy of a label, drawn above anything its backdrop covers.
fn topmost_text(output: &egui::FullOutput, label: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .rev()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text.trim() == label => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("No rendered text: {label}"))
}

/// The middle of the dialog's header ×. Its glyph is centred on the button by its
/// visible shape, so use that rather than the text's line box.
fn close_button(output: &egui::FullOutput) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .rev()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text == "×" => Some(
                text.galley
                    .mesh_bounds
                    .translate(text.pos.to_vec2())
                    .center(),
            ),
            _ => None,
        })
        .expect("No header ×")
}

/// Presses the header ×, then lets go `held` seconds later and `moved` points to the
/// right. Returns whether Preferences closed.
fn cancel_with(held: f64, moved: f32) -> bool {
    let (_temp, mut app, ctx) = fixture();
    app.open_preferences_window();
    frame(&mut app, &ctx, SIZE, vec![]);
    let output = frame(&mut app, &ctx, SIZE, vec![]);
    let press = close_button(&output);
    let start = ctx.input(|input| input.time) + 0.1;
    for (time, pos, pressed) in [
        (start, press, true),
        (start + held, press + vec2(moved, 0.0), false),
    ] {
        frame_at(
            &mut app,
            &ctx,
            time,
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
    app.preferences_window.is_none()
}

#[test]
fn slow_and_slightly_moving_clicks_still_cancel() {
    // egui samples time once per frame, so a busy frame between press and release counts
    // as holding the button. Dragging well away from the button must still not click it.
    let failures: Vec<_> = [
        (0.1, 0.0, true, "an ordinary click"),
        (1.5, 0.0, true, "a press held for 1.5 s"),
        (0.1, 8.0, true, "a press that moved 8 points"),
        (0.1, 40.0, false, "a drag 40 points away"),
    ]
    .into_iter()
    .filter(|&(held, moved, closes, _)| cancel_with(held, moved) != closes)
    .map(|(.., case)| case)
    .collect();
    assert!(failures.is_empty(), "Wrong result for: {failures:?}");
}

/// The fill of the smallest rectangle drawn under `point`, which is a button's background.
fn fill_under(output: &egui::FullOutput, point: egui::Pos2) -> egui::Color32 {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(shape) if shape.rect.contains(point) => Some(shape),
            _ => None,
        })
        .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
        .map_or(egui::Color32::TRANSPARENT, |shape| shape.fill)
}

#[test]
fn turned_off_save_looks_like_the_other_turned_off_buttons_and_says_why() {
    let (_temp, mut app, ctx) = fixture();
    app.open_preferences_window();
    // Connection has nothing to reset, so Reset is turned off beside an unchanged Save.
    app.preferences_window.as_mut().unwrap().category = Category::Connection;
    frame(&mut app, &ctx, SIZE, vec![]);
    let output = frame(&mut app, &ctx, SIZE, vec![]);
    let save = topmost_text(&output, "Save").center();
    let reset = topmost_text(&output, "Reset").center();
    assert_eq!(fill_under(&output, save), fill_under(&output, reset));

    let start = ctx.input(|input| input.time) + 0.1;
    frame_at(&mut app, &ctx, start, vec![egui::Event::PointerMoved(save)]);
    frame_at(&mut app, &ctx, start + 1.0, vec![]);
    let output = frame_at(&mut app, &ctx, start + 1.1, vec![]);
    let hints = output
        .shapes
        .iter()
        .filter(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text)
                if text.galley.job.text.trim() == "No unsaved changes")
        })
        .count();
    assert!(
        hints >= 2,
        "The footer note and a hint on Save, found {hints}"
    );
    app.close_preferences_window(&ctx);

    // A live Save keeps its own primary look.
    app.open_preferences_window();
    click(&mut app, &ctx, "Compact rows");
    let output = frame(&mut app, &ctx, SIZE, vec![]);
    let save = topmost_text(&output, "Save").center();
    let reset = topmost_text(&output, "Reset").center();
    assert_ne!(fill_under(&output, save), fill_under(&output, reset));
}

/// A corner of the dimmed area around the dialog.
const OUTSIDE: egui::Pos2 = egui::pos2(12.0, 788.0);

fn primary(pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: OUTSIDE,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Presses and lets go outside the dialog in two frames, as a quick click does.
fn click_outside(app: &mut Desktop, ctx: &egui::Context) {
    frame(
        app,
        ctx,
        SIZE,
        vec![egui::Event::PointerMoved(OUTSIDE), primary(true)],
    );
    frame(app, ctx, SIZE, vec![primary(false)]);
}

#[test]
fn a_click_outside_keeps_unsaved_edits_and_closes_when_nothing_is_unsaved() {
    let (_temp, mut app, ctx) = fixture();
    let saved = app.interface;
    app.open_preferences_window();
    click(&mut app, &ctx, "Compact rows");
    click_outside(&mut app, &ctx);
    frame(&mut app, &ctx, SIZE, vec![]);
    assert_eq!(
        app.dialog,
        Some(Dialog::PreferencesWindow),
        "A click outside threw away an unsaved edit"
    );
    let draft = app.preferences_window.as_ref().unwrap();
    assert!(draft.dirty() && draft.interface.compact_rows);
    assert_eq!(app.interface, saved);

    // With the edit undone there is nothing to lose, so a click outside closes it.
    click(&mut app, &ctx, "Compact rows");
    assert!(!app.preferences_window.as_ref().unwrap().dirty());
    click_outside(&mut app, &ctx);
    assert!(app.preferences_window.is_none() && app.dialog.is_none());
    assert_eq!(app.interface, saved);
}

#[test]
fn esc_still_closes_in_the_same_frame_as_an_ignored_click_outside() {
    let (_temp, mut app, ctx) = fixture();
    let saved = app.interface;
    app.open_preferences_window();
    click(&mut app, &ctx, "Compact rows");
    frame(
        &mut app,
        &ctx,
        SIZE,
        vec![egui::Event::PointerMoved(OUTSIDE), primary(true)],
    );
    let escape = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&mut app, &ctx, SIZE, vec![primary(false), escape]);
    assert!(app.preferences_window.is_none() && app.dialog.is_none());
    assert_eq!(app.interface, saved);
}
