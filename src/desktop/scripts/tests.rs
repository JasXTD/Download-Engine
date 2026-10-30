use super::super::interaction_tests::{click, fixture, frame, text_rect};
use super::*;

#[test]
fn provider_editor_uses_download_actions_in_compact_and_full_views() {
    let (_root, mut app, ctx) = fixture();
    app.view = View::Automation;
    app.automation.show_scripts();
    app.scripts.new_script();
    let draft = app.scripts.draft.as_mut().unwrap();
    draft.trigger = Trigger::Provider;
    draft.body = download_engine::provider::EXAMPLE.into();
    let saved = app.store.save_script(draft).unwrap();
    app.scripts.select(saved);
    app.scripts.test_url = "http://127.0.0.1:9000/fixture.bin".into();
    for full in [false, true] {
        app.scripts.full_editor = full;
        for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
            frame(&mut app, &ctx, size, vec![]);
            let out = frame(&mut app, &ctx, size, vec![]);
            for label in ["Source URL", "File name", "Add provider download"] {
                let rect = text_rect(&out, label);
                assert!(rect.bottom() < size.y && rect.right() < size.x);
            }
            assert!(rendered(&out, "Run script").is_empty());
            assert!(rendered(&out, "Run logs").is_empty());
            if full {
                assert!(rendered(&out, "Loaded scripts (1)").is_empty());
            }
        }
    }
    assert!(app.store.list().unwrap().is_empty());
    assert!(app.store.script_runs(None).unwrap().is_empty());
}

fn editor_menu(app: &mut Desktop, ctx: &egui::Context, label: &str) {
    click(app, ctx, "Editor actions");
    click(app, ctx, label);
}

#[test]
fn provider_redownload_keeps_its_frozen_script_instead_of_adding_a_plain_download() {
    let (_root, mut app, ctx) = fixture();
    app.scripts.new_script();
    let draft = app.scripts.draft.as_mut().unwrap();
    draft.trigger = Trigger::Provider;
    draft.body = download_engine::provider::EXAMPLE.into();
    let saved = app.store.save_script(draft).unwrap();
    let mut original = app
        .store
        .new_download(
            "http://127.0.0.1:1/sample.bin",
            Some("sample.bin"),
            None,
            "original".into(),
        )
        .unwrap();
    original.provider = Some(download_engine::provider::State {
        api_version: 1,
        script: download_engine::provider::FrozenScript::new(saved.clone()).unwrap(),
        plan: None,
    });
    app.store.remove_script(&saved.id).unwrap();
    assert!(!supports_authentication(&original));
    app.duplicate_download(&ctx, &original);
    let jobs = app.store.list().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_ne!(jobs[0].id, original.id);
    let provider = jobs[0]
        .provider
        .as_ref()
        .expect("must not bypass the transform");
    assert_eq!(provider.script, saved);
    assert!(provider.plan.is_none());
    assert!(!jobs[0].server_name_pending);
}

#[test]
fn compact_editor_is_visible_and_collapsing_settings_preserves_the_draft() {
    let (_root, mut app, ctx) = fixture();
    app.view = View::Automation;
    app.automation.show_scripts();
    app.scripts.new_script();
    app.scripts.draft.as_mut().unwrap().body = "print('visible editor')".into();
    let before = app.scripts.draft.clone();
    let size = egui::vec2(860.0, 600.0);
    frame(&mut app, &ctx, size, vec![]);
    let output = frame(&mut app, &ctx, size, vec![]);
    let code = rendered(&output, "print('visible editor')");
    assert!(!code.is_empty());
    for (rect, clip) in code {
        assert!(clip.contains_rect(rect) && rect.bottom() < size.y);
    }
    assert!(rendered(&output, "All downloads").is_empty());
    click(&mut app, &ctx, "Script settings");
    assert!(app.scripts.settings_open);
    click(&mut app, &ctx, "Script settings");
    assert!(!app.scripts.settings_open);
    assert_eq!(app.scripts.draft, before);
}

#[test]
fn loaded_scripts_search_switches_saved_scripts_and_protects_unsaved_edits() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    app.automation.show_scripts();
    for name in ["Alpha report", "Beta reader"] {
        app.scripts.new_script();
        app.scripts.settings_open = true;
        let draft = app.scripts.draft.as_mut().unwrap();
        draft.name = name.into();
        draft.body = "print('test')".into();
        if name == "Beta reader" {
            draft.trigger = Trigger::SiteReader {
                pages: vec!["example.com/*".into()],
            };
        }
        app.store.save_script(draft).unwrap();
    }
    app.scripts.draft = None;
    app.scripts.baseline = None;
    app.refresh_automation();
    click(&mut app, &ctx, "Loaded scripts (2)");
    app.scripts.library_search = "SITE READER".into();
    let size = egui::vec2(1080.0, 800.0);
    let out = frame(&mut app, &ctx, size, vec![]);
    text_rect(&out, "Beta reader");
    click(&mut app, &ctx, "Beta reader");
    assert_eq!(app.scripts.draft.as_ref().unwrap().name, "Beta reader");
    assert!(app.store.script_runs(None).unwrap().is_empty());

    app.scripts
        .draft
        .as_mut()
        .unwrap()
        .body
        .push_str("\n-- unsaved");
    app.scripts.library_search.clear();
    click(&mut app, &ctx, "Loaded scripts (2)");
    click(&mut app, &ctx, "Alpha report");
    assert_eq!(app.scripts.draft.as_ref().unwrap().name, "Beta reader");
    assert!(
        app.scripts
            .draft
            .as_ref()
            .unwrap()
            .body
            .ends_with("-- unsaved")
    );
    text_rect(
        &frame(&mut app, &ctx, size, vec![]),
        "Save or discard edits before switching scripts.",
    );
    app.scripts.library_search = "not found".into();
    text_rect(&frame(&mut app, &ctx, size, vec![]), "No matching scripts.");
    click(&mut app, &ctx, "Editor");
    click(&mut app, &ctx, "Save script");
    app.scripts.library_search.clear();
    click(&mut app, &ctx, "Loaded scripts (2)");
    click(&mut app, &ctx, "Alpha report");
    assert_eq!(app.scripts.draft.as_ref().unwrap().name, "Alpha report");
    assert_eq!(app.store.scripts().unwrap().len(), 2);
    assert!(app.store.script_runs(None).unwrap().is_empty());
}

#[test]
fn library_has_its_own_tab_and_new_script_never_appears_in_the_editor() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    app.automation.show_scripts();
    app.scripts.new_script();
    app.scripts.draft.as_mut().unwrap().name = "Saved report".into();
    let saved = app
        .store
        .save_script(app.scripts.draft.as_ref().unwrap())
        .unwrap();
    app.scripts.select(saved);
    app.refresh_automation();
    for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
        frame(&mut app, &ctx, size, vec![]);
        let out = frame(&mut app, &ctx, size, vec![]);
        text_rect(&out, "Lua editor");
        assert!(rendered(&out, "+ New script").is_empty());
        assert!(rendered(&out, "Search scripts").is_empty());
        let title = text_rect(&out, "Lua editor");
        let expand = text_rect(&out, "Full editor");
        assert!((title.center().y - expand.center().y).abs() < 2.0);
        click(&mut app, &ctx, "Loaded scripts (1)");
        let out = frame(&mut app, &ctx, size, vec![]);
        assert!(rendered(&out, "Lua editor").is_empty());
        text_rect(&out, "+ New script");
        text_rect(&out, "Saved report");
        click(&mut app, &ctx, "Editor");
        assert!(app.store.script_runs(None).unwrap().is_empty());
    }
    click(&mut app, &ctx, "Full editor");
    let out = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
    for hidden in [
        "Loaded scripts (1)",
        "+ New script",
        "Editor",
        "Back to scripts",
    ] {
        assert!(
            rendered(&out, hidden).is_empty(),
            "full editor showed {hidden}"
        );
    }
    text_rect(&out, "Exit full editor");
}

#[test]
fn full_editor_keeps_the_buffer_cursor_and_unsaved_settings_on_return() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    app.automation.show_scripts();
    app.scripts.new_script();
    app.scripts.settings_open = true;
    let draft = app.scripts.draft.as_mut().unwrap();
    draft.name = "My draft".into();
    draft.body = "print('hello')\n".into();
    draft.trigger = Trigger::Interval { minutes: 17 };
    let before = draft.clone();
    let id = egui::Id::new(("script-editor", &draft.id));
    let size = egui::vec2(1080.0, 800.0);
    frame(&mut app, &ctx, size, vec![]);
    let mut edit = egui::TextEdit::load_state(&ctx, id).unwrap();
    edit.cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(7),
        )));
    edit.store(&ctx, id);
    click(&mut app, &ctx, "Full editor");
    assert!(app.scripts.full_editor());
    assert_eq!(app.scripts.draft.as_ref().unwrap(), &before);
    assert_eq!(
        usize::from(
            egui::TextEdit::load_state(&ctx, id)
                .unwrap()
                .cursor
                .char_range()
                .unwrap()
                .primary
                .index
        ),
        7
    );
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&mut app, &ctx, size, vec![egui::Event::Text("new ".into())]);
    assert_eq!(
        app.scripts.draft.as_ref().unwrap().body,
        "print('new hello')\n"
    );
    click(&mut app, &ctx, "Run logs");
    assert!(app.scripts.show_full_logs);
    click(&mut app, &ctx, "Exit full editor");
    assert!(!app.scripts.full_editor());
    assert_eq!(
        app.scripts.draft.as_ref().unwrap().body,
        "print('new hello')\n"
    );
    assert_eq!(app.scripts.draft.as_ref().unwrap().trigger, before.trigger);
    assert!(app.scripts.dirty());
    assert!(app.store.scripts().unwrap().is_empty());
    assert!(app.store.script_runs(None).unwrap().is_empty());
}

#[test]
fn full_editor_fits_controls_and_gives_code_more_room_in_every_language() {
    for language in download_engine::i18n::Language::ALL {
        let (_temp, mut app, ctx) = fixture();
        app.interface.language = language;
        app.configure_appearance(&ctx);
        let _scope = download_engine::i18n::scope(language);
        app.view = View::Automation;
        app.automation.show_scripts();
        app.scripts.new_script();
        app.scripts.settings_open = true;
        for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
            app.scripts.full_editor = true;
            frame(&mut app, &ctx, size, vec![]);
            let out = frame(&mut app, &ctx, size, vec![]);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            for label in [
                "Exit full editor",
                "Save script",
                "Run script",
                "Core API",
                "Run logs",
            ] {
                let rendered = rendered(&out, tr(label));
                assert!(!rendered.is_empty(), "{language:?}: missing {label}");
                for (rect, clip) in rendered {
                    assert!(
                        screen.contains_rect(rect) && clip.expand(1.0).contains_rect(rect),
                        "{language:?} {size}: {label} clipped"
                    );
                }
            }
            // The actual code viewport, not a surrounding card, must grow.
            let viewport = app.scripts.editor.viewport.unwrap();
            assert!(
                viewport.height() >= 280.0,
                "{language:?} {size}: {viewport:?}; controls: {:?}",
                out.shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.pos.y < viewport.top() =>
                            Some((&t.galley.job.text, t.galley.rect.translate(t.pos.to_vec2()))),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            );
            assert!(screen.contains_rect(viewport));
        }
    }
}

#[test]
fn core_api_reference_search_matches_the_script_role_without_editing_code() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    let code = app.scripts.draft.as_ref().unwrap().body.clone();
    app.scripts.api_search = "html_select".into();
    click(&mut app, &ctx, "Core API");
    let out = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
    text_rect(&out, "xd.html_select(html, selector, attribute)");
    assert_eq!(app.scripts.draft.as_ref().unwrap().body, code);
    app.scripts.api_search = "xd.fetch".into();
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    assert!(rendered(&out, "xd.fetch(url, options)").is_empty());
    app.scripts.draft.as_mut().unwrap().trigger = Trigger::SiteReader {
        pages: vec!["example.com/*".into()],
    };
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    text_rect(&out, "xd.fetch(url, options)");
}

#[test]
fn packages_import_as_disabled_drafts_export_and_preserve_unsaved_edits() {
    use download_engine::script_automation::package;
    let (temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    let original = Script {
        id: uuid::Uuid::new_v4().to_string(),
        revision: 8,
        name: "Shared reader".into(),
        language: Language::Lua,
        body: "return {items={}}".into(),
        enabled: true,
        trigger: Trigger::SiteReader {
            pages: vec!["example.com/*".into()],
        },
        timeout_seconds: 30,
        change_files: false,
        extension: None,
    };
    let path = temp.path().join("reader.de-script.json");
    std::fs::write(&path, package::encode(&original).unwrap()).unwrap();
    app.folder_picker.response = Some(Ok(Some(path.to_string_lossy().into_owned())));
    click(&mut app, &ctx, "Import script…");
    assert!(app.dialog.is_none());
    let imported = app.scripts.draft.clone().unwrap();
    assert_eq!(imported.body, original.body);
    assert_eq!(imported.trigger, original.trigger);
    assert_ne!(imported.id, original.id);
    assert!(!imported.enabled && !imported.change_files);
    assert!(app.scripts.dirty());
    assert!(app.store.scripts().unwrap().is_empty());
    click(&mut app, &ctx, "Save script");
    assert_eq!(app.store.scripts().unwrap().len(), 1);
    let export = temp.path().join("export.de-script.json");
    app.folder_picker.response = Some(Ok(Some(export.to_string_lossy().into_owned())));
    click(&mut app, &ctx, "Export script…");
    let roundtrip = package::decode(&std::fs::read(&export).unwrap()).unwrap();
    assert_eq!(roundtrip.script.body, original.body);
    assert_eq!(roundtrip.script.trigger, original.trigger);
    app.scripts.draft.as_mut().unwrap().name = "Unsaved edit".into();
    let before = app.scripts.draft.clone();
    app.folder_picker.response = Some(Ok(Some(path.to_string_lossy().into_owned())));
    click(&mut app, &ctx, "Import script…");
    assert!(matches!(app.dialog, Some(Dialog::ScriptReplace)));
    click(&mut app, &ctx, "×");
    assert_eq!(app.scripts.draft, before);
    app.folder_picker.response = Some(Ok(Some(path.to_string_lossy().into_owned())));
    click(&mut app, &ctx, "Import script…");
    click(&mut app, &ctx, "Replace script");
    assert_ne!(app.scripts.draft.as_ref().unwrap().id, imported.id);
    assert_eq!(app.store.scripts().unwrap().len(), 1);
    std::fs::write(&path, "{\"format_version\":99}").unwrap();
    let before = app.scripts.draft.clone();
    app.folder_picker.response = Some(Ok(Some(path.to_string_lossy().into_owned())));
    click(&mut app, &ctx, "Import script…");
    assert_eq!(app.scripts.draft, before);
    assert!(app.scripts.error.is_some());
}

#[test]
fn script_guides_follow_language_and_preserve_unsaved_code() {
    let (_temp, mut app, ctx) = fixture();
    click(&mut app, &ctx, "Extensions");
    click(&mut app, &ctx, "Scripts");
    // The Lua help is also reachable before any script exists.
    click(&mut app, &ctx, "Lua guide");
    assert_eq!(app.view, View::Guide);
    click(&mut app, &ctx, "Extensions");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    app.scripts.draft.as_mut().unwrap().body = "print('My unsaved script')".into();
    let draft = app.scripts.draft.clone();
    click(&mut app, &ctx, "Lua guide");
    let out = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
    text_rect(&out, "Run your first Lua script");
    click(&mut app, &ctx, "Extensions");
    assert_eq!(app.scripts.draft, draft);
    click(&mut app, &ctx, "Lua");
    click(&mut app, &ctx, "PowerShell");
    click(&mut app, &ctx, "Script guide");
    let out = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
    text_rect(&out, "Route new downloads");
    click(&mut app, &ctx, "Extensions");
    assert_eq!(
        app.scripts.draft.as_ref().unwrap().body,
        draft.unwrap().body
    );
    assert_eq!(
        app.scripts.draft.as_ref().unwrap().language,
        Language::PowerShell
    );
}

#[test]
fn language_picker_replaces_only_the_untouched_example() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    assert_eq!(app.scripts.draft.as_ref().unwrap().language, Language::Lua);
    click(&mut app, &ctx, "Lua");
    click(&mut app, &ctx, "PowerShell");
    assert_eq!(
        app.scripts.draft.as_ref().unwrap().body,
        Language::PowerShell.example()
    );
    app.scripts.draft.as_mut().unwrap().body = "custom code".into();
    click(&mut app, &ctx, "PowerShell");
    click(&mut app, &ctx, "Lua");
    assert_eq!(app.scripts.draft.as_ref().unwrap().body, "custom code");
    assert_eq!(app.scripts.draft.as_ref().unwrap().language, Language::Lua);
    click(&mut app, &ctx, "Save script");
    assert_eq!(app.store.scripts().unwrap()[0].language, Language::Lua);
}

#[test]
fn output_rejoins_pipe_chunks_and_colours_warning_lines() {
    let logs = vec![
        Log {
            at: 0,
            level: "info".into(),
            text: "hel".into(),
        },
        Log {
            at: 0,
            level: "info".into(),
            text: "lo\nWARN".into(),
        },
        Log {
            at: 0,
            level: "info".into(),
            text: "ING: be careful\n".into(),
        },
        Log {
            at: 0,
            level: "error".into(),
            text: "error\n".into(),
        },
    ];
    let layout = output_layout(&logs, true);
    assert_eq!(layout.text, "hello\nWARNING: be careful\nerror\n");
    assert_eq!(layout.text, log_text(&logs));
    assert_eq!(layout.sections.len(), 3);
    assert_ne!(
        layout.sections[0].format.color,
        layout.sections[1].format.color
    );
    assert_ne!(
        layout.sections[1].format.color,
        layout.sections[2].format.color
    );
}

#[test]
fn editor_accepts_keyboard_input_and_escape_releases_focus() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    let id = egui::Id::new(("script-editor", &app.scripts.draft.as_ref().unwrap().id));
    ctx.memory_mut(|m| m.request_focus(id));
    frame(
        &mut app,
        &ctx,
        egui::vec2(1080.0, 800.0),
        vec![egui::Event::Text("print('typed')".into())],
    );
    assert!(
        app.scripts
            .draft
            .as_ref()
            .unwrap()
            .body
            .contains("print('typed')")
    );
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
    assert!(!ctx.memory(|m| m.has_focus(id)));
}

#[test]
fn editor_saves_runs_stops_and_keeps_the_control_bar_visible() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    assert!(app.scripts.dirty());
    assert!(!app.scripts.draft.as_ref().unwrap().enabled);
    assert_eq!(app.scripts.draft.as_ref().unwrap().language, Language::Lua);
    // This interaction test runs from the Rust test binary, beside no CLI worker.
    // Packaged integration tests exercise actual Lua execution.
    app.scripts.draft.as_mut().unwrap().language = Language::PowerShell;
    app.scripts.draft.as_mut().unwrap().body = "Write-Output 'ready'; Start-Sleep 60".into();
    click(&mut app, &ctx, "Run script");
    assert!(app.store.script_runs(None).unwrap().is_empty());
    click(&mut app, &ctx, "Save script");
    assert!(!app.scripts.dirty());
    assert_eq!(app.store.scripts().unwrap().len(), 1);
    click(&mut app, &ctx, "Full editor");
    click(&mut app, &ctx, "Run script");
    let runs = app.store.script_runs(None).unwrap();
    assert_eq!(runs.len(), 1);
    assert!(runs[0].active());
    click(&mut app, &ctx, "Stop script");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.store.script_run(&runs[0].id).unwrap().active() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert_eq!(app.store.script_run(&runs[0].id).unwrap().state, "stopped");
    click(&mut app, &ctx, "Exit full editor");
    app.refresh_automation();
    for dark in [true, false] {
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
            frame(&mut app, &ctx, size, vec![]);
            let out = frame(&mut app, &ctx, size, vec![]);
            for label in ["Save script", "Run script", "Full editor", "Editor actions"] {
                assert!(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                        .contains_rect(text_rect(&out, label)),
                    "{label} {size}"
                );
            }
        }
    }
    editor_menu(&mut app, &ctx, "Remove script");
    assert!(app.store.scripts().unwrap().is_empty());
    assert_eq!(app.store.script_runs(None).unwrap().len(), 1);
}

#[test]
fn editor_preserves_unsaved_text_rejects_stale_save_and_reloads_the_new_revision() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    click(&mut app, &ctx, "Save script");
    let mut saved = app.store.scripts().unwrap().remove(0);
    app.scripts.draft.as_mut().unwrap().body = "Write-Output 'my unsaved work'".into();
    saved.body = "Write-Output 'external edit'".into();
    saved = app.store.save_script(&saved).unwrap();
    app.refresh_automation();
    assert_eq!(
        app.scripts.draft.as_ref().unwrap().body,
        "Write-Output 'my unsaved work'"
    );
    click(&mut app, &ctx, "Save script");
    assert!(
        app.scripts
            .error
            .as_ref()
            .unwrap()
            .contains("changed elsewhere")
    );
    assert_eq!(app.store.script(&saved.id).unwrap().body, saved.body);
    editor_menu(&mut app, &ctx, "Reload / discard edits");
    assert!(!app.scripts.dirty());
    assert_eq!(app.scripts.draft.as_ref().unwrap(), &saved);
    click(&mut app, &ctx, "Loaded scripts (1)");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    editor_menu(&mut app, &ctx, "Reload / discard edits");
    assert_eq!(app.store.scripts().unwrap().len(), 1);
    assert!(!app.scripts.dirty());
}

#[test]
fn log_refresh_follows_latest_unless_an_older_run_is_pinned() {
    let (_temp, mut app, _ctx) = fixture();
    app.scripts.new_script();
    app.scripts.settings_open = true;
    let saved = app
        .store
        .save_script(app.scripts.draft.as_ref().unwrap())
        .unwrap();
    app.scripts.select(saved.clone());
    let first = app
        .store
        .run_script(&saved.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    app.store.stop_script(&first.id).unwrap();
    // The scheduler may have claimed the run before Stop. In that case Stop
    // requests cancellation asynchronously; wait for completion before starting
    // another run of the same script, just as the real UI requires.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app
        .store
        .script_run(&first.id)
        .unwrap()
        .finished_at
        .is_none()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "Stopped script did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    refresh(&app.store, &mut app.scripts);
    assert_eq!(app.scripts.selected_run.as_deref(), Some(first.id.as_str()));
    let second = app
        .store
        .run_script(&saved.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    app.store.stop_script(&second.id).unwrap();
    refresh(&app.store, &mut app.scripts);
    assert_eq!(
        app.scripts.selected_run.as_deref(),
        Some(second.id.as_str())
    );
    app.scripts.pinned_run = true;
    app.scripts.selected_run = Some(first.id.clone());
    refresh(&app.store, &mut app.scripts);
    assert_eq!(app.scripts.selected_run.as_deref(), Some(first.id.as_str()));
}

#[test]
fn script_form_labels_share_control_centers_and_leading_columns() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    for dark in [true, false] {
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
            frame(&mut app, &ctx, size, vec![]);
            let out = frame(&mut app, &ctx, size, vec![]);
            for labels in [
                ["Name", "New script", "Timeout", "300"],
                ["Language", "Lua", "Trigger", "Manual only"],
            ] {
                let center = text_rect(&out, labels[0]).center().y;
                for label in labels {
                    let rect = rendered(&out, label)
                        .into_iter()
                        .map(|(rect, _)| rect)
                        .min_by(|a, b| {
                            (a.center().y - center)
                                .abs()
                                .total_cmp(&(b.center().y - center).abs())
                        })
                        .unwrap_or_else(|| panic!("Missing form value {label}"));
                    assert!(
                        (rect.center().y - center).abs() <= 1.0,
                        "{label} is off its row center: {rect:?}, expected {center}"
                    );
                }
            }
            let name_center = text_rect(&out, "Name").center().y;
            let name_field = rendered(&out, "New script")
                .into_iter()
                .map(|(rect, _)| rect)
                .find(|rect| (rect.center().y - name_center).abs() <= 1.0)
                .unwrap();
            assert!(
                (name_field.left() - text_rect(&out, "Lua").left()).abs() <= 5.0,
                "Leading fields do not share a column"
            );
        }
    }
}

fn choose_visible_popup_option(app: &mut Desktop, ctx: &egui::Context, label: &str) {
    let size = egui::vec2(1080.0, 800.0);
    let out = frame(app, ctx, size, vec![]);
    let pos = out
        .shapes
        .iter()
        .rev()
        .find_map(|shape| {
            if let egui::epaint::Shape::Text(text) = &shape.shape
                && text.galley.job.text == label
            {
                Some(text.galley.rect.translate(text.pos.to_vec2()).center())
            } else {
                None
            }
        })
        .unwrap();
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
fn choosing_the_current_trigger_preserves_its_interval_and_queue() {
    let (_temp, mut app, ctx) = fixture();
    let queue = app.store.create_queue("Work downloads").unwrap();
    app.queues = app.store.queues().unwrap();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    for trigger in [
        Trigger::Interval { minutes: 17 },
        Trigger::DownloadCompleted {
            queue: Some(queue.id.clone()),
        },
        Trigger::QueueCompleted { queue: queue.id },
    ] {
        app.scripts.draft.as_mut().unwrap().trigger = trigger.clone();
        click(&mut app, &ctx, trigger_name(&trigger));
        choose_visible_popup_option(&mut app, &ctx, trigger_name(&trigger));
        assert_eq!(app.scripts.draft.as_ref().unwrap().trigger, trigger);
    }
}

#[test]
fn removed_download_trigger_queue_is_not_shown_as_all_queues() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    app.scripts.draft.as_mut().unwrap().trigger = Trigger::DownloadCompleted {
        queue: Some("removed-queue".into()),
    };
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    text_rect(&out, "Queue removed — choose another");
}

#[test]
fn long_script_and_queue_names_stay_within_the_workbench() {
    let (_temp, mut app, ctx) = fixture();
    let queue = app
        .store
        .create_queue(&format!("Long queue {}", "W".repeat(48)))
        .unwrap();
    app.queues = app.store.queues().unwrap();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    let draft = app.scripts.draft.as_mut().unwrap();
    draft.name = format!("Long script {}", "W".repeat(48));
    draft.trigger = Trigger::DownloadCompleted {
        queue: Some(queue.id.clone()),
    };
    click(&mut app, &ctx, "Save script");
    assert!(app.scripts.error.is_none(), "{:?}", app.scripts.error);
    assert_eq!(app.store.scripts().unwrap().len(), 1);
    for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
        frame(&mut app, &ctx, size, vec![]);
        let out = frame(&mut app, &ctx, size, vec![]);
        for text in [&app.scripts.draft.as_ref().unwrap().name, &queue.name] {
            // Ignore the global queue sidebar and the horizontally scrolling
            // text editor; inspect the selected values in the two pickers.
            let shape = if *text == queue.name {
                out.shapes.iter().rfind(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(t) if t.galley.job.text == *text))
            } else {
                out.shapes.iter().find(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(t) if t.galley.job.text == *text))
            }.unwrap();
            let egui::epaint::Shape::Text(rendered) = &shape.shape else {
                unreachable!()
            };
            let rect = rendered.galley.rect.translate(rendered.pos.to_vec2());
            assert!(
                shape.clip_rect.expand(1.0).contains_rect(rect),
                "Picker text is clipped: {text}, {rect:?}"
            );
            assert_eq!(
                rendered.galley.rows.len(),
                1,
                "Picker grows into a multiline control: {text}"
            );
        }
    }
}

/// Gives the newest run the outcome a Lua worker would: a result, or an error in
/// its log. The test app's own scheduler also claims the run and, with no worker
/// next to the test binary, fails it at once; wait for that, then replace it.
fn finish_run(app: &Desktop, result: Option<serde_json::Value>, error: Option<&str>) {
    let conn =
        crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3")).unwrap();
    let id: String = conn
        .query_row(
            "SELECT id FROM script_runs ORDER BY rowid DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let state: String = conn
            .query_row("SELECT state FROM script_runs WHERE id=?1", [&id], |r| {
                r.get(0)
            })
            .unwrap();
        if !matches!(state.as_str(), "queued" | "running") || std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    conn.execute("DELETE FROM script_logs WHERE run_id=?1", [&id])
        .unwrap();
    let state = if result.is_some() {
        "succeeded"
    } else {
        "failed"
    };
    conn.execute(
        "UPDATE script_runs SET state=?1,data=json_set(data,'$.state',?1) WHERE id=?2",
        rusqlite::params![state, id],
    )
    .unwrap();
    if let Some(result) = result {
        conn.execute(
            "INSERT INTO script_results(run_id,data) VALUES(?1,?2)",
            rusqlite::params![id, result.to_string()],
        )
        .unwrap();
    }
    if let Some(error) = error {
        conn.execute(
            "INSERT INTO script_logs(run_id,at,level,text) VALUES(?1,0,'error',?2)",
            rusqlite::params![id, error],
        )
        .unwrap();
    }
}

/// Type into a text field the editor identifies by ID.
fn type_into(app: &mut Desktop, ctx: &egui::Context, id: &str, text: &str) {
    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(id)));
    frame(
        app,
        ctx,
        egui::vec2(1280.0, 860.0),
        vec![egui::Event::Text(text.into())],
    );
}

#[test]
fn a_site_reader_is_built_tested_and_explained_in_the_editor() {
    let (_temp, mut app, ctx) = fixture();
    let size = egui::vec2(1280.0, 2000.0);
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    click(&mut app, &ctx, "Manual only");
    click(&mut app, &ctx, "Site reader");
    let draft = app.scripts.draft.clone().unwrap();
    assert_eq!(draft.trigger, Trigger::SiteReader { pages: vec![] });
    assert_eq!(
        draft.body, READER_EXAMPLE,
        "the untouched example follows the trigger"
    );
    let out = frame(&mut app, &ctx, size, vec![]);
    text_rect(
        &out,
        "Site readers can read web pages, but cannot see or change your downloads.",
    );

    // Typed pages are checked and stored without their scheme.
    type_into(&mut app, &ctx, "site-reader-page", "localhost/*");
    click(&mut app, &ctx, "Add page");
    assert!(app.scripts.pages.error.is_some());
    app.scripts.pages.text.clear();
    type_into(
        &mut app,
        &ctx,
        "site-reader-page",
        "https://Example.com/watch/*",
    );
    click(&mut app, &ctx, "Add page");
    assert_eq!(
        app.scripts.draft.as_ref().unwrap().trigger,
        Trigger::SiteReader {
            pages: vec!["example.com/watch/*".into()]
        }
    );
    assert!(app.scripts.pages.error.is_none() && app.scripts.pages.text.is_empty());
    click(&mut app, &ctx, "Save script");
    assert!(app.scripts.error.is_none(), "{:?}", app.scripts.error);
    let saved = app.store.scripts().unwrap().remove(0);

    // Testing runs the saved reader on a page; nothing is downloaded.
    app.scripts.test_url = "https://example.com/watch/a1".into();
    click(&mut app, &ctx, "Test on URL");
    let run = app.store.script_runs(Some(&saved.id)).unwrap().remove(0);
    assert_eq!(
        (run.trigger.as_str(), run.context["url"].as_str()),
        ("site_reader", Some("https://example.com/watch/a1"))
    );
    let out = frame(&mut app, &ctx, size, vec![]);
    if app.store.script_run(&run.id).unwrap().active() {
        text_rect(&out, "Reading the page…");
    }
    finish_run(
        &app,
        Some(serde_json::json!({
            "title": "Nature documentary",
            "items": [{"kind": "hls", "url": "https://cdn.example.com/a1/master.m3u8", "label": "All qualities"}]
        })),
        None,
    );
    refresh(&app.store, &mut app.scripts);
    let out = frame(&mut app, &ctx, size, vec![]);
    for text in ["FOUND 1", "Nature documentary", "HLS", "All qualities"] {
        text_rect(&out, text);
    }
    assert!(app.store.list().unwrap().is_empty());

    // A failed test shows the script's own message, not the stack trace.
    click(&mut app, &ctx, "Test on URL");
    finish_run(
        &app,
        None,
        Some("runtime error: reader.lua:4: Not a video page\nstack traceback:\n"),
    );
    refresh(&app.store, &mut app.scripts);
    let out = frame(&mut app, &ctx, size, vec![]);
    text_rect(
        &out,
        "Test failed: runtime error: reader.lua:4: Not a video page",
    );

    // Readers are Lua only, and leaving the trigger restores the plain example.
    app.scripts.draft.as_mut().unwrap().body = READER_EXAMPLE.into();
    click(&mut app, &ctx, "Lua");
    click(&mut app, &ctx, "PowerShell");
    assert_eq!(app.scripts.draft.as_ref().unwrap().language, Language::Lua);
    frame(
        &mut app,
        &ctx,
        size,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    click(&mut app, &ctx, "Site reader");
    click(&mut app, &ctx, "Manual only");
    assert_eq!(app.scripts.draft.as_ref().unwrap().body, LUA_EXAMPLE);
}

fn rendered(out: &egui::FullOutput, label: &str) -> Vec<(egui::Rect, egui::Rect)> {
    out.shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == label => Some((
                text.galley.rect.translate(text.pos.to_vec2()),
                shape.clip_rect,
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn failure_trigger_swaps_only_the_starter_example_and_every_trigger_name_fits() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    click(&mut app, &ctx, trigger_name(&Trigger::Manual));
    choose_visible_popup_option(&mut app, &ctx, "Download failed");
    let draft = app.scripts.draft.as_ref().unwrap();
    assert_eq!(draft.trigger, Trigger::DownloadFailed { queue: None });
    assert_eq!(draft.body, FAILED_EXAMPLE);
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    text_rect(
        &out,
        "event.downloads lists the downloads that need attention, with their errors. Use xd.retry or xd.restart to try again.",
    );
    click(&mut app, &ctx, "Download failed");
    choose_visible_popup_option(&mut app, &ctx, trigger_name(&Trigger::Manual));
    assert_eq!(app.scripts.draft.as_ref().unwrap().body, LUA_EXAMPLE);
    // An edited script keeps its code.
    app.scripts.draft.as_mut().unwrap().body = "print('mine')".into();
    click(&mut app, &ctx, trigger_name(&Trigger::Manual));
    choose_visible_popup_option(&mut app, &ctx, "Download failed");
    assert_eq!(app.scripts.draft.as_ref().unwrap().body, "print('mine')");
    // Switching to PowerShell replaces the untouched failure example too.
    app.scripts.draft.as_mut().unwrap().body = FAILED_EXAMPLE.into();
    click(&mut app, &ctx, "Lua");
    click(&mut app, &ctx, "PowerShell");
    assert_eq!(
        app.scripts.draft.as_ref().unwrap().body,
        Language::PowerShell.example()
    );
    // No trigger name or "All queues" is cut off in any language.
    for language in download_engine::i18n::Language::ALL {
        app.interface.language = language;
        let _scope = download_engine::i18n::scope(language);
        for trigger in [
            Trigger::DownloadFailed { queue: None },
            Trigger::DownloadCompleted { queue: None },
            Trigger::QueueCompleted {
                queue: MAIN_QUEUE.into(),
            },
            Trigger::Interval { minutes: 5 },
        ] {
            app.scripts.draft.as_mut().unwrap().trigger = trigger.clone();
            frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
            let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
            let mut labels = vec![trigger_name(&trigger)];
            if matches!(trigger, Trigger::DownloadFailed { .. }) {
                labels.push(tr("All queues"));
            }
            for label in labels {
                let found = rendered(&out, label);
                assert!(!found.is_empty(), "{language:?}: {label} is not shown");
                for (rect, clip) in found {
                    assert!(
                        clip.expand(1.0).contains_rect(rect),
                        "{language:?}: {label} is cut off"
                    );
                }
            }
        }
    }
}

#[test]
fn sample_runs_start_from_event_scripts_and_are_labelled_in_run_logs() {
    let (_temp, mut app, ctx) = fixture();
    let mut job = app
        .store
        .new_download(
            "https://example.com/textures.zip",
            None,
            None,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    // Publish the terminal snapshot atomically: the fixture's live scheduler
    // must never claim this synthetic job and overwrite its sample error.
    job.status = download_engine::model::Status::Failed;
    job.error = Some("Server returned 404".into());
    let job = app.store.enqueue(&job).unwrap();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    // A manual script has no event to sample.
    click(&mut app, &ctx, "Save script");
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    assert!(rendered(&out, "Run with sample…").is_empty());
    click(&mut app, &ctx, trigger_name(&Trigger::Manual));
    choose_visible_popup_option(&mut app, &ctx, "Download failed");
    click(&mut app, &ctx, "Save script");
    editor_menu(&mut app, &ctx, "Run with sample…");
    assert!(matches!(app.dialog, Some(Dialog::ScriptSample)));
    let out = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
    text_rect(&out, &job.name);
    text_rect(&out, "Server returned 404");
    text_rect(&out, "1 of 1 chosen");
    click(&mut app, &ctx, "Run sample");
    assert!(app.dialog.is_none(), "{:?}", app.scripts.error);
    let runs = app.store.script_runs(None).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].context["sample"], true);
    assert_eq!(runs[0].context["downloads"][0]["id"], job.id);
    assert_eq!(
        app.scripts.selected_run.as_deref(),
        Some(runs[0].id.as_str())
    );
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    text_rect(&out, "SAMPLE");
    // The picker closes with ×, and nothing runs. The app's scheduler may already
    // own the sample run; wait until Stop has ended it so the button is enabled.
    app.store.stop_script(&runs[0].id).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while app.store.script_run(&runs[0].id).unwrap().active() {
        assert!(
            std::time::Instant::now() < until,
            "The sample run did not stop"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    scripts::refresh(&app.store, &mut app.scripts);
    editor_menu(&mut app, &ctx, "Run with sample…");
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none() && app.scripts.sample.is_none());
    assert_eq!(app.store.script_runs(None).unwrap().len(), 1);
}

#[test]
fn stored_values_and_the_files_switch_live_in_one_row_and_readers_have_neither() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    text_rect(&out, "none");
    click(&mut app, &ctx, "Can change downloaded files");
    assert!(app.scripts.draft.as_ref().unwrap().change_files);
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    text_rect(
        &out,
        "Lets this script rename, move and delete its downloads' files. Moves stay inside your download folders.",
    );
    click(&mut app, &ctx, "Save script");
    let saved = app.store.scripts().unwrap().remove(0);
    assert!(saved.change_files);
    app.store
        .set_script_value(&saved.id, "count", &serde_json::json!(2))
        .unwrap();
    app.store
        .set_script_value(&saved.id, "hosts", &serde_json::json!(["a.example"]))
        .unwrap();
    scripts::refresh(&app.store, &mut app.scripts);
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    text_rect(&out, "2 · 24 B");
    assert!(rendered(&out, "hosts").is_empty());
    click(&mut app, &ctx, "Show");
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    let key = text_rect(&out, "hosts");
    let value = text_rect(&out, "[\"a.example\"]");
    assert!(key.right() < value.left(), "Keys and values share one row");
    assert!((key.center().y - value.center().y).abs() < 1.0);
    click(&mut app, &ctx, "Clear");
    assert!(app.store.script_values(&saved.id).unwrap().is_empty());
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    text_rect(&out, "none");
    app.scripts.draft.as_mut().unwrap().trigger = Trigger::SiteReader {
        pages: vec!["example.com/watch/*".into()],
    };
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    assert!(rendered(&out, "Can change downloaded files").is_empty());
    assert!(rendered(&out, "Stored values").is_empty());
}

#[test]
fn clicking_a_script_notification_opens_its_run_log() {
    let (_temp, mut app, ctx) = fixture();
    let first = app
        .store
        .save_script(&Script {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            name: "Notify me".into(),
            language: Language::Lua,
            body: "print('hi')".into(),
            enabled: false,
            trigger: Trigger::Manual,
            timeout_seconds: 30,
            change_files: false,
            extension: None,
        })
        .unwrap();
    let run = app
        .store
        .run_script(&first.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    // The app's scheduler may already own the run; wait until Stop has ended it.
    app.store.stop_script(&run.id).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while app.store.script_run(&run.id).unwrap().active() {
        assert!(std::time::Instant::now() < until, "The run did not stop");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(app.view, View::Downloads);
    app.tray_action(crate::tray::Action::ScriptRun(run.id.clone()), &ctx);
    assert_eq!(app.view, View::Automation);
    assert_eq!(app.scripts.selected_run.as_deref(), Some(run.id.as_str()));
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    // Stopped while queued, or failed to start if the test scheduler claimed it first.
    let badge = match app.store.script_run(&run.id).unwrap().state.as_str() {
        "stopped" => "STOPPED",
        "failed" => "FAILED",
        other => panic!("Unexpected run state: {other}"),
    };
    text_rect(&out, badge);
    app.scripts.full_editor = true;
    app.scripts.show_full_logs = false;
    app.tray_action(crate::tray::Action::ScriptRun(run.id), &ctx);
    assert!(app.scripts.show_full_logs);
    frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    let out = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
    for (rect, clip) in rendered(&out, badge) {
        assert!(clip.expand(1.0).contains_rect(rect));
    }
}

#[test]
fn sample_run_lines_are_muted_but_the_script_output_is_not() {
    let logs = vec![Log {
        at: 0,
        level: "info".into(),
        text: "Failed: a.zip\nWould retry a.zip (sample run, nothing changed)\n".into(),
    }];
    let layout = output_layout(&logs, true);
    assert_eq!(layout.sections.len(), 2);
    assert_ne!(
        layout.sections[0].format.color,
        layout.sections[1].format.color
    );
    assert_eq!(layout.sections[1].format.color, Palette::new(true).muted);
}

#[test]
fn script_files_decode_utf8_and_utf16_and_reject_binary() {
    let utf16 = |text: &str, little: bool| {
        let mut bytes = if little {
            vec![0xFF, 0xFE]
        } else {
            vec![0xFE, 0xFF]
        };
        for unit in text.encode_utf16() {
            bytes.extend(if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        bytes
    };
    let text = "print('héllo 🌍')\r\nprint(2)\r\n";
    let expected = "print('héllo 🌍')\nprint(2)\n";
    assert_eq!(file::decode(text.as_bytes()).as_deref(), Some(expected));
    let mut bom = b"\xEF\xBB\xBF".to_vec();
    bom.extend_from_slice(text.as_bytes());
    assert_eq!(file::decode(&bom).as_deref(), Some(expected));
    assert_eq!(file::decode(&utf16(text, true)).as_deref(), Some(expected));
    assert_eq!(file::decode(&utf16(text, false)).as_deref(), Some(expected));
    for binary in [
        &b"\xFF\xD8\xFF\xE0 jpeg"[..],
        b"text\0with nul",
        b"\xFF\xFEa",
        b"\xFF\xFE\x00\xD8",
    ] {
        assert!(file::decode(binary).is_none(), "{binary:?}");
    }
    assert_eq!(file::encode("x", Language::Lua), b"x");
    assert_eq!(file::encode("x", Language::PowerShell), b"\xEF\xBB\xBFx");
}

#[test]
fn open_file_loads_code_names_new_scripts_and_asks_before_replacing_unsaved_code() {
    let (temp, mut app, ctx) = fixture();
    let folder = temp.path().join("my scripts");
    std::fs::create_dir_all(&folder).unwrap();
    let ps1 = folder.join("retry-failed.ps1");
    let mut bytes = vec![0xFF, 0xFE];
    for unit in "Write-Output 'hi'\r\n".encode_utf16() {
        bytes.extend(unit.to_le_bytes());
    }
    std::fs::write(&ps1, bytes).unwrap();
    let lua = folder.join("other.lua");
    std::fs::write(&lua, "print('other')\n").unwrap();
    let choose = |app: &mut Desktop, path: &std::path::Path| {
        app.folder_picker.response = Some(Ok(Some(path.to_string_lossy().into_owned())));
    };
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    // An untouched starter example is replaced without a question.
    choose(&mut app, &ps1);
    editor_menu(&mut app, &ctx, "Open file…");
    assert!(app.dialog.is_none());
    let draft = app.scripts.draft.clone().unwrap();
    assert_eq!(draft.language, Language::PowerShell);
    assert_eq!(draft.body, "Write-Output 'hi'\n");
    assert_eq!(draft.name, "retry-failed");
    assert!(app.scripts.dirty());
    assert_eq!(
        app.notice.as_ref().unwrap().message,
        "Loaded retry-failed.ps1. Choose Save script to keep it."
    );
    assert!(
        app.store.scripts().unwrap().is_empty(),
        "Nothing is saved yet"
    );
    // Unsaved code asks first; × keeps it.
    choose(&mut app, &lua);
    editor_menu(&mut app, &ctx, "Open file…");
    assert!(matches!(app.dialog, Some(Dialog::ScriptReplace)));
    let out = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
    text_rect(&out, "Replace the code in the editor?");
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none() && app.scripts.replace.is_none());
    assert_eq!(
        app.scripts.draft.as_ref().unwrap().body,
        "Write-Output 'hi'\n"
    );
    choose(&mut app, &lua);
    editor_menu(&mut app, &ctx, "Open file…");
    click(&mut app, &ctx, "Replace code");
    let draft = app.scripts.draft.clone().unwrap();
    assert_eq!(draft.body, "print('other')\n");
    assert_eq!(draft.language, Language::Lua);
    assert_eq!(draft.name, "retry-failed", "A named script keeps its name");
    // Saved code is replaced without a question; Reload brings it back.
    click(&mut app, &ctx, "Save script");
    assert_eq!(app.store.scripts().unwrap()[0].body, "print('other')\n");
    std::fs::write(&lua, "print('edited elsewhere')\n").unwrap();
    choose(&mut app, &lua);
    editor_menu(&mut app, &ctx, "Open file…");
    assert!(app.dialog.is_none());
    assert_eq!(
        app.scripts.draft.as_ref().unwrap().body,
        "print('edited elsewhere')\n"
    );
    assert_eq!(app.store.scripts().unwrap()[0].body, "print('other')\n");
    editor_menu(&mut app, &ctx, "Reload / discard edits");
    assert_eq!(app.scripts.draft.as_ref().unwrap().body, "print('other')\n");
    // A failed open changes nothing and says why.
    let large = folder.join("big-script.lua");
    std::fs::write(&large, "-".repeat(MAX_SCRIPT_BYTES + 1)).unwrap();
    choose(&mut app, &large);
    editor_menu(&mut app, &ctx, "Open file…");
    assert_eq!(
        app.scripts.error.as_deref(),
        Some("big-script.lua is larger than 64 KiB.")
    );
    let binary = folder.join("notes.lua");
    std::fs::write(&binary, b"\x89PNG\r\n\x1a\n\0\0").unwrap();
    choose(&mut app, &binary);
    editor_menu(&mut app, &ctx, "Open file…");
    assert_eq!(
        app.scripts.error.as_deref(),
        Some("notes.lua is not a text file. Save it as UTF-8 and try again.")
    );
    app.scripts.draft.as_mut().unwrap().trigger = Trigger::SiteReader {
        pages: vec!["example.com/watch/*".into()],
    };
    choose(&mut app, &ps1);
    editor_menu(&mut app, &ctx, "Open file…");
    assert_eq!(
        app.scripts.error.as_deref(),
        Some("Site readers are Lua only. Open a .lua file.")
    );
    assert_eq!(app.scripts.draft.as_ref().unwrap().language, Language::Lua);
    // Cancelling the picker does nothing.
    app.folder_picker.response = Some(Ok(None));
    editor_menu(&mut app, &ctx, "Open file…");
    assert_eq!(app.scripts.draft.as_ref().unwrap().body, "print('other')\n");
}

#[test]
fn save_to_file_writes_the_editor_code_with_the_right_encoding() {
    let (temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    let draft = app.scripts.draft.as_mut().unwrap();
    draft.language = Language::PowerShell;
    draft.body = "Write-Output 'héllo'\n".into();
    let target = temp.path().join("Retry failed.ps1");
    app.folder_picker.response = Some(Ok(Some(target.to_string_lossy().into_owned())));
    editor_menu(&mut app, &ctx, "Save to file…");
    let mut expected = b"\xEF\xBB\xBF".to_vec();
    expected.extend_from_slice("Write-Output 'héllo'\n".as_bytes());
    assert_eq!(std::fs::read(&target).unwrap(), expected);
    assert_eq!(
        app.notice.as_ref().unwrap().message,
        "Saved Retry failed.ps1."
    );
    assert!(
        app.store.scripts().unwrap().is_empty(),
        "The script itself is not saved"
    );
    // Lua has no byte-order mark, and the confirmed file is replaced.
    let draft = app.scripts.draft.as_mut().unwrap();
    draft.language = Language::Lua;
    draft.body = "print('two')\n".into();
    let lua = temp.path().join("two.lua");
    std::fs::write(&lua, "old").unwrap();
    app.folder_picker.response = Some(Ok(Some(lua.to_string_lossy().into_owned())));
    editor_menu(&mut app, &ctx, "Save to file…");
    assert_eq!(std::fs::read(&lua).unwrap(), b"print('two')\n");
    let leftovers: Vec<_> = std::fs::read_dir(temp.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
    app.folder_picker.response = Some(Ok(Some(
        temp.path()
            .join("missing")
            .join("x.lua")
            .to_string_lossy()
            .into_owned(),
    )));
    editor_menu(&mut app, &ctx, "Save to file…");
    assert!(
        app.scripts
            .error
            .as_deref()
            .unwrap()
            .starts_with("Cannot save the script file:")
    );
}

#[test]
fn file_results_show_on_the_scripting_page() {
    let (temp, mut app, ctx) = fixture();
    let size = egui::vec2(1280.0, 860.0);
    let shown = |out: &egui::FullOutput, label: &str| {
        let found = rendered(out, label);
        assert!(!found.is_empty(), "{label} is not shown");
        let (rect, clip) = found[0];
        assert!(
            clip.contains_rect(rect)
                && egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
            "{label} is cut off: {rect:?} in {clip:?}"
        );
    };
    let lua = temp.path().join("other.lua");
    std::fs::write(&lua, "print('other')\n").unwrap();
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    if !app.scripts.settings_open {
        click(&mut app, &ctx, "Script settings");
    }
    app.folder_picker.response = Some(Ok(Some(lua.to_string_lossy().into_owned())));
    editor_menu(&mut app, &ctx, "Open file…");
    let loaded = "Loaded other.lua. Choose Save script to keep it.";
    shown(&frame(&mut app, &ctx, size, vec![]), loaded);
    click(&mut app, &ctx, "Dismiss");
    assert!(app.notice.is_none());
    assert!(rendered(&frame(&mut app, &ctx, size, vec![]), loaded).is_empty());
    let target = temp.path().join("copy.lua");
    app.folder_picker.response = Some(Ok(Some(target.to_string_lossy().into_owned())));
    editor_menu(&mut app, &ctx, "Save to file…");
    shown(&frame(&mut app, &ctx, size, vec![]), "Saved copy.lua.");
}

#[test]
fn editor_header_and_file_menu_fit_in_every_language() {
    for language in download_engine::i18n::Language::ALL {
        let (_temp, mut app, ctx) = fixture();
        app.interface.language = language;
        app.configure_appearance(&ctx);
        let _scope = download_engine::i18n::scope(language);
        app.view = View::Automation;
        app.automation.show_scripts();
        app.scripts.new_script();
        for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
            frame(&mut app, &ctx, size, vec![]);
            let out = frame(&mut app, &ctx, size, vec![]);
            let title = text_rect(&out, &tf!("{} editor", "Lua"));
            for label in [
                "Save script",
                "Run script",
                "Full editor",
                "Core API",
                "Editor actions",
            ] {
                let controls = rendered(&out, tr(label));
                assert!(!controls.is_empty(), "{language:?}: {label}");
                for (rect, clip) in controls {
                    assert!(
                        clip.expand(1.0).contains_rect(rect) && rect.bottom() < size.y,
                        "{language:?}: {label} clipped"
                    );
                    assert!(
                        rect.top() >= title.top() - 3.0,
                        "{label} left the editor header"
                    );
                }
            }
            assert!(rendered(&out, tr("Open file…")).is_empty());
            click(&mut app, &ctx, tr("Editor actions"));
            let out = frame(&mut app, &ctx, size, vec![]);
            for label in [
                "Open file…",
                "Save to file…",
                "Reload / discard edits",
                "Remove script",
            ] {
                let controls = rendered(&out, tr(label));
                assert!(
                    !controls.is_empty(),
                    "{language:?}: missing menu action {label}"
                );
                for (rect, clip) in controls {
                    assert!(
                        clip.expand(1.0).contains_rect(rect)
                            && rect.right() <= size.x
                            && rect.bottom() <= size.y,
                        "{language:?}: menu clipped {label}"
                    );
                }
            }
            egui::Popup::close_all(&ctx);
        }
    }
}

#[test]
fn escape_exits_full_editor_and_preserves_the_unsaved_draft() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    app.automation.show_scripts();
    app.scripts.new_script();
    app.scripts.draft.as_mut().unwrap().body = "print('keep my draft')".into();
    let draft = app.scripts.draft.clone();
    app.scripts.full_editor = true;
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
    assert!(!app.scripts.full_editor());
    assert_eq!(app.scripts.draft, draft);
    assert!(app.scripts.dirty());
    assert!(app.store.scripts().unwrap().is_empty());
    assert!(app.store.script_runs(None).unwrap().is_empty());
}

#[test]
fn escape_dismisses_completion_before_leaving_full_editor() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Automation;
    app.automation.show_scripts();
    app.scripts.new_script();
    let draft = app.scripts.draft.as_mut().unwrap();
    draft.body.clear();
    let id = egui::Id::new(("script-editor", &draft.id));
    app.scripts.full_editor = true;
    let size = egui::vec2(1080.0, 800.0);
    frame(&mut app, &ctx, size, vec![]);
    ctx.memory_mut(|memory| memory.request_focus(id));
    frame(&mut app, &ctx, size, vec![egui::Event::Text("xd.".into())]);
    assert!(app.scripts.editor.completion_open());
    let escape = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&mut app, &ctx, size, vec![escape.clone()]);
    assert!(app.scripts.full_editor());
    assert!(!app.scripts.editor.completion_open());
    frame(&mut app, &ctx, size, vec![escape]);
    assert!(!app.scripts.full_editor());
    assert_eq!(app.scripts.draft.as_ref().unwrap().body, "xd.");
}
