//! Inline diagnostics and completion. Typing affects only the draft; parsing is
//! debounced, cancellable, and never saves or executes the code.
use super::*;
use download_engine::script_automation::syntax::{Checker, Diagnostic};
use egui::text::{CCursor, CCursorRange};
use std::time::{Duration, Instant};
mod completion;
mod highlight;
#[cfg(test)]
mod tests;

const DEBOUNCE: Duration = Duration::from_millis(400);

#[derive(Default)]
pub(super) struct State {
    identity: Option<(String, Language, bool)>,
    source: String,
    revision: u64,
    changed_at: Option<Instant>,
    submitted: bool,
    checker: Option<Checker>,
    diagnostics: Vec<Diagnostic>,
    check_error: Option<String>,
    checked: bool,
    choices: Option<completion::Choices>,
    selected: usize,
    cursor: Option<usize>,
    dismissed: bool,
    composing: bool,
    jump: Option<usize>,
    layout: Option<(String, Language, bool, egui::text::LayoutJob)>,
    popup_rect: Option<egui::Rect>,
    #[cfg(test)]
    pub(super) viewport: Option<egui::Rect>,
    #[cfg(feature = "screenshot")]
    capture_started: bool,
    #[cfg(feature = "screenshot")]
    capture_rect: Option<egui::Rect>,
    #[cfg(feature = "screenshot")]
    capture_stable: u8,
}
impl State {
    pub(super) fn completion_open(&self) -> bool {
        self.choices.is_some()
    }
    #[cfg(feature = "screenshot")]
    pub(super) fn capture_ready(&self) -> bool {
        self.checked && (std::env::var("IDM_EX_TEST_EDITOR").is_err() || self.capture_stable >= 3)
    }
    fn observe(&mut self, draft: &Script) {
        let identity = (
            draft.id.clone(),
            draft.language,
            matches!(draft.trigger, Trigger::SiteReader { .. }),
        );
        if self.identity.as_ref() != Some(&identity) || self.source != draft.body {
            let other = self.identity.as_ref() != Some(&identity);
            if let Some(worker) = &self.checker {
                worker.cancel();
            }
            self.identity = Some(identity);
            self.source.clone_from(&draft.body);
            self.revision += 1;
            self.changed_at = Some(Instant::now());
            self.submitted = false;
            self.checked = false;
            self.check_error = None;
            self.diagnostics.clear();
            self.choices = None;
            self.dismissed = false;
            if other {
                self.cursor = None;
                self.jump = None;
            }
        }
    }
    fn poll(&mut self, ctx: &egui::Context, language: Language) {
        if let Some(worker) = &self.checker {
            while let Some(reply) = worker.try_recv() {
                if reply.revision != self.revision {
                    continue;
                }
                self.checked = true;
                match reply.result {
                    Ok(errors) => self.diagnostics = errors,
                    Err(error) => self.check_error = Some(error.to_string()),
                }
            }
        }
        if self.checked {
            return;
        }
        if !self.submitted
            && self.changed_at.is_some_and(|at| at.elapsed() >= DEBOUNCE)
            && !self.composing
        {
            if self.checker.is_none() {
                match Checker::new() {
                    Ok(worker) => self.checker = Some(worker),
                    Err(error) => {
                        self.check_error = Some(error.to_string());
                        self.checked = true;
                        return;
                    }
                }
            }
            if let Some(worker) = &mut self.checker {
                worker.submit(self.revision, language, self.source.clone());
            }
            self.submitted = true;
        }
        ctx.request_repaint_after(Duration::from_millis(60));
    }
}

fn replace_with_completion(
    ctx: &egui::Context,
    id: egui::Id,
    text: &mut String,
    options: &completion::Choices,
    selected: usize,
) {
    let mut edit = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let before = edit
        .cursor
        .char_range()
        .unwrap_or_else(|| CCursorRange::one(CCursor::new(0)));
    let mut undoer = edit.undoer();
    undoer.add_undo(&(before, text.clone()));
    if let Some(cursor) = completion::insert(text, options, selected) {
        let range = CCursorRange::one(CCursor::new(cursor));
        edit.cursor.set_char_range(Some(range));
        undoer.add_undo(&(range, text.clone()));
        edit.set_undoer(undoer);
        edit.store(ctx, id);
        ctx.memory_mut(|memory| memory.request_focus(id));
    }
}

pub(super) fn show(ui: &mut egui::Ui, draft: &mut Script, state: &mut State, p: Palette) {
    show_sized(ui, draft, state, p, None);
}

pub(super) fn show_sized(
    ui: &mut egui::Ui,
    draft: &mut Script,
    state: &mut State,
    p: Palette,
    height: Option<f32>,
) {
    let id = egui::Id::new(("script-editor", &draft.id));
    #[cfg(feature = "screenshot")]
    if std::env::var("IDM_EX_TEST_EDITOR").is_ok() {
        // Native preview windows can open in the background. The interaction
        // tests cover real key/pointer events; give this isolated capture focus.
        ui.input_mut(|input| input.focused = true);
    }
    state.observe(draft);
    ui.input(|input| {
        for event in &input.events {
            if let egui::Event::Ime(ime) = event {
                state.composing = matches!(ime, egui::ImeEvent::Preedit { .. });
            }
        }
    });
    state.poll(ui.ctx(), draft.language);
    #[cfg(feature = "screenshot")]
    let capture_completion = if !state.capture_started
        && std::env::var("IDM_EX_TEST_EDITOR").as_deref() == Ok("completion")
    {
        state.capture_started = true;
        let mut edit = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        edit.cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(
                draft.body.chars().count(),
            ))));
        edit.store(ui.ctx(), id);
        ui.memory_mut(|m| m.request_focus(id));
        true
    } else {
        false
    };
    let focused = ui.memory(|m| m.has_focus(id));
    let over_popup = state.popup_rect.is_some_and(|rect| {
        ui.input(|i| i.pointer.hover_pos().is_some_and(|pos| rect.contains(pos)))
    });
    let mut explicit = false;
    let mut accepted = false;
    // A batched text/paste event must reach TextEdit before using a completion
    // range computed for the previous buffer (for example fast typing + Tab).
    let edits_pending = ui.input(|i| {
        i.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Cut
            )
        })
    });
    if focused && !state.composing && !edits_pending {
        explicit = ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Space));
        #[cfg(feature = "screenshot")]
        {
            explicit |= capture_completion;
        }
        if explicit {
            state.dismissed = false;
        }
        if let Some(options) = &state.choices {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                state.dismissed = true;
            } else {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
                    state.selected = (state.selected + 1) % options.items.len();
                }
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
                    state.selected =
                        (state.selected + options.items.len() - 1) % options.items.len();
                }
                if ui.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                        || i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                }) {
                    replace_with_completion(ui.ctx(), id, &mut draft.body, options, state.selected);
                    accepted = true;
                }
            }
        }
    }
    if (!focused && !over_popup)
        || !ui.is_enabled()
        || state.dismissed
        || state.composing
        || accepted
    {
        state.choices = None;
    }
    if accepted {
        state.observe(draft);
        state.dismissed = true;
    }
    let jumping = state.jump.is_some();
    if let Some(character) = state.jump.take() {
        let mut edit = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        edit.cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(character))));
        edit.store(ui.ctx(), id);
        ui.memory_mut(|m| m.request_focus(id));
        state.choices = None;
        state.dismissed = true;
    }
    let mut output = None;
    let dark = ui.visuals().dark_mode;
    let editor_frame = egui::Frame::new()
        .fill(p.background)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(8)
        .inner_margin(8)
        .show(ui, |ui| {
            let resize = egui::Resize::default()
                .id_salt(("script-editor-resize", height.is_some()))
                .default_width(ui.available_width())
                .default_height(130.0)
                .min_height(80.0)
                .max_height(480.0)
                .resizable([false, true])
                .with_stroke(false);
            let resize = if let Some(height) = height {
                resize.fixed_size(egui::vec2(ui.available_width(), height))
            } else {
                resize
            };
            resize.show(ui, |ui| {
                let _scroll = egui::ScrollArea::both()
                    .id_salt("script-code-scroll")
                    .max_height(ui.available_height())
                    .min_scrolled_height(80.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            let left = ui.cursor().min.x;
                            let gutter = 16.0
                                + (draft.body.bytes().filter(|b| *b == b'\n').count() + 1)
                                    .to_string()
                                    .len() as f32
                                    * 8.0;
                            ui.add_space(gutter);
                            let mut layouter =
                                |ui: &egui::Ui, text: &dyn egui::TextBuffer, _: f32| {
                                    let text = text.as_str();
                                    if state.layout.as_ref().is_none_or(
                                        |(old, language, theme, _)| {
                                            old != text
                                                || *language != draft.language
                                                || *theme != dark
                                        },
                                    ) {
                                        state.layout = Some((
                                            text.into(),
                                            draft.language,
                                            dark,
                                            highlight::layout(text, draft.language, dark),
                                        ));
                                    }
                                    ui.fonts_mut(|f| {
                                        f.layout_job(state.layout.as_ref().unwrap().3.clone())
                                    })
                                };
                            let edit = egui::TextEdit::multiline(&mut draft.body)
                                .id(id)
                                .code_editor()
                                .font(egui::FontId::monospace(13.0))
                                .frame(egui::Frame::NONE)
                                .margin(egui::Margin::ZERO)
                                .desired_width(ui.available_width())
                                .desired_rows(7)
                                .char_limit(MAX_SCRIPT_BYTES)
                                .layouter(&mut layouter)
                                .show(ui);
                            if jumping && let Some(range) = edit.cursor_range {
                                ui.scroll_to_rect(
                                    edit.galley
                                        .pos_from_cursor(range.primary)
                                        .translate(edit.galley_pos.to_vec2()),
                                    Some(egui::Align::Center),
                                );
                            }
                            // Both gutter and markers use the actual galley positions,
                            // so horizontal/vertical scrolling and font fallback align.
                            let mut character = 0;
                            for (row, line) in draft.body.split('\n').enumerate() {
                                let rect = edit
                                    .galley
                                    .pos_from_cursor(CCursor::new(character))
                                    .translate(edit.galley_pos.to_vec2());
                                if rect.bottom() >= edit.text_clip_rect.top()
                                    && rect.top() <= edit.text_clip_rect.bottom()
                                {
                                    let error = state.diagnostics.iter().any(|d| d.line == row + 1);
                                    let color = if error { p.warning } else { p.muted };
                                    ui.painter().text(
                                        egui::pos2(left + gutter - 6.0, rect.center().y),
                                        egui::Align2::RIGHT_CENTER,
                                        (row + 1).to_string(),
                                        egui::FontId::monospace(12.0),
                                        color,
                                    );
                                    if error {
                                        let right = edit
                                            .galley
                                            .pos_from_cursor(CCursor::new(
                                                character + line.chars().count(),
                                            ))
                                            .translate(edit.galley_pos.to_vec2());
                                        let mark = egui::Rect::from_min_max(
                                            egui::pos2(rect.left(), rect.bottom() - 1.0),
                                            egui::pos2(
                                                right.left().max(rect.left() + 8.0),
                                                rect.bottom(),
                                            ),
                                        );
                                        ui.painter()
                                            .with_clip_rect(edit.text_clip_rect)
                                            .rect_filled(mark, 0, p.warning);
                                    }
                                }
                                character += line.chars().count() + 1;
                            }
                            output = Some(edit);
                        });
                    });
                #[cfg(test)]
                {
                    state.viewport = Some(_scroll.inner_rect);
                }
            });
        });
    let edit = output.expect("script editor output");
    let changed = state.source != draft.body;
    state.observe(draft);
    let cursor = edit
        .cursor_range
        .filter(|r| r.is_empty())
        .map(|r| usize::from(r.primary.index));
    let moved = cursor != state.cursor;
    if moved && !changed && !explicit {
        state.choices = None;
        state.dismissed = false;
    }
    if edit.response.has_focus() && !state.composing && !state.dismissed && (changed || explicit) {
        state.choices = cursor.and_then(|c| {
            completion::choices(
                &draft.body,
                c,
                draft.language,
                matches!(draft.trigger, Trigger::SiteReader { .. }),
                explicit,
            )
        });
        state.selected = 0;
    }
    state.cursor = cursor;
    if !edit.response.has_focus() && !over_popup {
        state.choices = None;
    }
    // A compact overlay follows the caret and is constrained to the viewport.
    if let Some(options) = state.choices.clone()
        && let Some(cursor) = cursor
    {
        let caret = edit
            .galley
            .pos_from_cursor(CCursor::new(cursor))
            .translate(edit.galley_pos.to_vec2());
        if edit.text_clip_rect.intersects(caret) {
            let screen = ui.ctx().content_rect().shrink(8.0);
            let width = 320.0_f32.min(screen.width());
            let height = (options.items.len().min(6) as f32 * 25.0 + 45.0).min(screen.height());
            let top = if caret.bottom() + height > screen.bottom() {
                caret.top() - height
            } else {
                caret.bottom() + 3.0
            };
            let position = egui::pos2(
                caret.left().clamp(screen.left(), screen.right() - width),
                top.clamp(screen.top(), screen.bottom() - height),
            );
            let mut chosen = None;
            let popup = egui::Area::new(id.with("completion"))
                .order(egui::Order::Foreground)
                .fixed_pos(position)
                .movable(false)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(width - 20.0);
                        egui::ScrollArea::vertical()
                            .max_height(height - 36.0)
                            .show(ui, |ui| {
                                for (index, item) in options.items.iter().enumerate() {
                                    let response = ui.add(
                                        egui::Button::selectable(
                                            index == state.selected,
                                            RichText::new(&item.signature).monospace().size(12.0),
                                        )
                                        .truncate(),
                                    );
                                    if index == state.selected {
                                        response.scroll_to_me(Some(egui::Align::Center));
                                    }
                                    if response.clicked() {
                                        chosen = Some(index);
                                    }
                                    response.on_hover_text(&item.signature);
                                }
                            });
                        ui.label(
                            RichText::new(tr("↑↓ choose · Tab / Enter insert · Esc dismiss"))
                                .small()
                                .color(p.muted),
                        );
                    });
                });
            state.popup_rect = Some(popup.response.rect);
            if let Some(index) = chosen {
                replace_with_completion(ui.ctx(), id, &mut draft.body, &options, index);
                state.observe(draft);
                state.dismissed = true;
            }
        } else {
            state.choices = None;
        }
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let (label, color) = if state.check_error.is_some() { (tr("Syntax check unavailable"), p.warning) }
            else if !state.checked { (tr("Checking syntax…"), p.muted) }
            else if state.diagnostics.is_empty() { (tr("No syntax errors"), p.accent) }
            else { (tr("Syntax errors"), p.warning) };
        ui.label(RichText::new(label).small().color(color)).on_hover_text(tr("Checks syntax automatically without running your script. Runtime errors still appear in Run logs."));
        ui.label(RichText::new("·").color(p.muted));
        ui.label(RichText::new(tr("Ctrl+Space: suggestions")).small().color(p.muted));
    });
    if let Some(error) = &state.check_error {
        ui.label(RichText::new(error).small().color(p.warning));
    }
    for error in &state.diagnostics {
        let location = error.column.map_or_else(
            || format!("{}", error.line),
            |c| format!("{}:{c}", error.line),
        );
        let label = tf!(
            "Line {line}: {message}",
            line = location,
            message = &error.message
        );
        if ui
            .add(
                egui::Button::new(RichText::new(label).small().color(p.warning))
                    .frame(false)
                    .wrap(),
            )
            .clicked()
        {
            let line = error.line.saturating_sub(1);
            let start: usize = draft
                .body
                .split_inclusive('\n')
                .take(line)
                .map(|l| l.chars().count())
                .sum();
            let length = draft
                .body
                .split('\n')
                .nth(line)
                .unwrap_or("")
                .chars()
                .count();
            state.jump = Some(start + error.column.unwrap_or(1).saturating_sub(1).min(length));
            ui.ctx().request_repaint();
        }
    }
    state.poll(ui.ctx(), draft.language);
    ui.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            id,
            egui::EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: state.choices.is_some(),
            },
        )
    });
    #[cfg(feature = "screenshot")]
    if std::env::var("IDM_EX_TEST_EDITOR").is_ok() && state.checked {
        // Inspect the scrollable editor, rather than just the settings above it.
        let target = egui::Rect::from_min_max(
            editor_frame.response.rect.min,
            egui::pos2(editor_frame.response.rect.right(), ui.cursor().min.y),
        );
        ui.scroll_to_rect(target, Some(egui::Align::Center));
        let rect = editor_frame.response.rect;
        state.capture_stable = if state.capture_rect == Some(rect) {
            state.capture_stable.saturating_add(1)
        } else {
            0
        };
        state.capture_rect = Some(rect);
        if state.capture_stable < 3 {
            ui.ctx().request_repaint();
        }
    }
    #[cfg(not(feature = "screenshot"))]
    let _ = editor_frame;
}
