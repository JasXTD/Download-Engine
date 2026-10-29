use super::*;
use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};

#[test]
fn launch_update_preference_saves_cancels_and_resets_without_changing_windows_startup() {
    let (_temp, mut app, ctx) = fixture();
    let windows_startup = app.startup.enabled();
    assert!(app.interface.check_updates_on_launch);
    app.open_preferences_window();
    click(&mut app, &ctx, "Startup & tray");
    click(&mut app, &ctx, "Check for updates on launch");
    click(&mut app, &ctx, "Cancel");
    assert!(app.interface.check_updates_on_launch);
    app.open_preferences_window();
    click(&mut app, &ctx, "Startup & tray");
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
    click(&mut app, &ctx, "Cancel");
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
        "Advanced settings saved. Cancel will not undo those changes.",
    );
    click(&mut app, &ctx, "Cancel");
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
            click(&mut app, &ctx, "Cancel");
        }
        frame(&mut app, &ctx, vec2(1080.0, 800.0), vec![]);
        assert!(app.preferences_window.is_none() && app.dialog.is_none());
        assert_eq!(app.view, View::Queues);
        assert_eq!(app.selected, selected);
        assert_eq!(
            ctx.theme(),
            egui::Theme::Light,
            "Cancel should restore saved theme"
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
    click(&mut app, &ctx, "Cancel");
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
            for label in [tr("Save"), tr("Cancel"), tr("Reset"), "×"] {
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
