use super::*;

fn script() -> Script {
    let mut state = super::super::State::default();
    state.new_script();
    let mut script = state.draft.unwrap();
    script.body.clear();
    script
}

fn frame(
    ctx: &egui::Context,
    script: &mut Script,
    state: &mut State,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(720.0, 500.0),
            )),
            events,
            focused: true,
            ..Default::default()
        },
        |ui| {
            show(ui, script, state, Palette::new(ui.visuals().dark_mode));
        },
    );
    output.textures_delta.clear();
    output
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
fn focus(ctx: &egui::Context, script: &Script, cursor: usize) -> egui::Id {
    let id = egui::Id::new(("script-editor", &script.id));
    let mut edit = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    edit.cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(cursor))));
    edit.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
    id
}
fn wait_checked(ctx: &egui::Context, script: &mut Script, state: &mut State) {
    let deadline = Instant::now() + Duration::from_secs(12);
    while !state.checked {
        assert!(Instant::now() < deadline, "syntax checker did not finish");
        std::thread::sleep(Duration::from_millis(30));
        frame(ctx, script, state, vec![]);
    }
    assert!(state.check_error.is_none(), "{:?}", state.check_error);
}

#[test]
fn typing_checks_automatically_and_clears_errors_after_fixing_the_buffer() {
    let ctx = egui::Context::default();
    let mut script = script();
    let mut state = State::default();
    frame(&ctx, &mut script, &mut state, vec![]);
    focus(&ctx, &script, 0);
    frame(
        &ctx,
        &mut script,
        &mut state,
        vec![egui::Event::Text("local value = )".into())],
    );
    assert!(!state.checked);
    wait_checked(&ctx, &mut script, &mut state);
    assert_eq!(state.diagnostics[0].line, 1);
    assert_eq!(script.revision, 0, "checking must not save the draft");
    script.body = "while true do end".into();
    frame(&ctx, &mut script, &mut state, vec![]);
    assert!(
        state.diagnostics.is_empty(),
        "old errors must clear on edit"
    );
    wait_checked(&ctx, &mut script, &mut state);
    assert!(
        state.diagnostics.is_empty(),
        "syntax checking must not execute an infinite loop"
    );
}

#[test]
fn tab_completes_in_place_and_undo_restores_the_typed_prefix() {
    let ctx = egui::Context::default();
    let mut script = script();
    script.body = "-- 中文 🦀\n".into();
    let mut state = State::default();
    frame(&ctx, &mut script, &mut state, vec![]);
    focus(&ctx, &script, script.body.chars().count());
    frame(
        &ctx,
        &mut script,
        &mut state,
        vec![egui::Event::Text("xd.que".into())],
    );
    assert_eq!(state.choices.as_ref().unwrap().items[0].name, "xd.queues");
    frame(
        &ctx,
        &mut script,
        &mut state,
        vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
    );
    assert_eq!(script.body, "-- 中文 🦀\nxd.queues()");
    frame(
        &ctx,
        &mut script,
        &mut state,
        vec![key(egui::Key::Z, egui::Modifiers::COMMAND)],
    );
    assert_eq!(script.body, "-- 中文 🦀\nxd.que");
}

#[test]
fn escape_dismisses_completion_before_releasing_editor_focus() {
    let ctx = egui::Context::default();
    let mut script = script();
    let mut state = State::default();
    frame(&ctx, &mut script, &mut state, vec![]);
    let id = focus(&ctx, &script, 0);
    frame(
        &ctx,
        &mut script,
        &mut state,
        vec![egui::Event::Text("xd.".into())],
    );
    assert!(state.choices.is_some());
    frame(
        &ctx,
        &mut script,
        &mut state,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert!(state.choices.is_none());
    assert!(ctx.memory(|m| m.has_focus(id)));
    frame(
        &ctx,
        &mut script,
        &mut state,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert!(!ctx.memory(|m| m.has_focus(id)));
}

#[test]
fn explicit_completion_keyboard_selection_and_mouse_insertion_work() {
    let ctx = egui::Context::default();
    let mut script = script();
    script.body = "xd.".into();
    let mut state = State::default();
    frame(&ctx, &mut script, &mut state, vec![]);
    focus(&ctx, &script, 3);
    frame(
        &ctx,
        &mut script,
        &mut state,
        vec![key(egui::Key::Space, egui::Modifiers::CTRL)],
    );
    frame(
        &ctx,
        &mut script,
        &mut state,
        vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)],
    );
    assert_eq!(state.selected, 1);
    let chosen = state.choices.as_ref().unwrap().items[1].name.clone();
    frame(
        &ctx,
        &mut script,
        &mut state,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
    );
    assert!(script.body.starts_with(&chosen));
    script.body = "xd.que".into();
    frame(&ctx, &mut script, &mut state, vec![]);
    focus(&ctx, &script, 6);
    frame(
        &ctx,
        &mut script,
        &mut state,
        vec![key(egui::Key::Space, egui::Modifiers::CTRL)],
    );
    frame(&ctx, &mut script, &mut state, vec![]);
    let output = frame(&ctx, &mut script, &mut state, vec![]);
    let pos = crate::desktop::interaction_tests::text_rect(&output, "xd.queues()").center();
    for pressed in [true, false] {
        frame(
            &ctx,
            &mut script,
            &mut state,
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
    assert_eq!(script.body, "xd.queues()");
}

#[test]
fn comments_and_ime_do_not_trigger_suggestions() {
    let ctx = egui::Context::default();
    let mut script = script();
    let mut state = State::default();
    frame(&ctx, &mut script, &mut state, vec![]);
    focus(&ctx, &script, 0);
    frame(
        &ctx,
        &mut script,
        &mut state,
        vec![egui::Event::Text("-- xd.".into())],
    );
    assert!(state.choices.is_none());
    script.body.clear();
    frame(&ctx, &mut script, &mut state, vec![]);
    focus(&ctx, &script, 0);
    frame(
        &ctx,
        &mut script,
        &mut state,
        vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "xd.".into(),
            active_range_chars: None,
        })],
    );
    assert!(state.composing);
    assert!(state.choices.is_none());
}

#[test]
fn edits_clear_old_diagnostics_and_document_changes_cancel_completions() {
    let mut scripts = super::super::State::default();
    scripts.new_script();
    let draft = scripts.draft.as_mut().unwrap();
    let mut state = State::default();
    state.observe(draft);
    let old_revision = state.revision;
    state.diagnostics.push(Diagnostic {
        line: 1,
        column: None,
        message: "old error".into(),
    });
    state.checked = true;
    draft.body = "print('new')".into();
    state.observe(draft);
    assert!(state.diagnostics.is_empty());
    assert!(!state.checked);
    assert!(state.revision > old_revision);
    state.choices = completion::choices("xd.", 3, Language::Lua, false, false);
    draft.language = Language::PowerShell;
    state.observe(draft);
    assert!(state.choices.is_none());
}

#[test]
fn clicking_a_diagnostic_moves_to_its_line_without_changing_code() {
    let ctx = egui::Context::default();
    let mut script = script();
    script.body = format!("{}local value = )", "-- another line\n".repeat(30));
    let original = script.body.clone();
    let mut state = State::default();
    frame(&ctx, &mut script, &mut state, vec![]);
    wait_checked(&ctx, &mut script, &mut state);
    let out = frame(&ctx, &mut script, &mut state, vec![]);
    let label = format!("Line 31: {}", state.diagnostics[0].message);
    let pos = crate::desktop::interaction_tests::text_rect(&out, &label).center();
    for pressed in [true, false] {
        frame(
            &ctx,
            &mut script,
            &mut state,
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
    frame(&ctx, &mut script, &mut state, vec![]);
    let id = egui::Id::new(("script-editor", &script.id));
    let caret = egui::TextEdit::load_state(&ctx, id)
        .unwrap()
        .cursor
        .char_range()
        .unwrap()
        .primary
        .index;
    assert_eq!(usize::from(caret), "-- another line\n".chars().count() * 30);
    assert_eq!(script.body, original);
}
