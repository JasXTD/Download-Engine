//! Saved scripts have their own tab, separate from the code workspace.
use super::*;

pub(super) fn tabs(ui: &mut egui::Ui, state: &mut State) {
    let library = state.library_visible();
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                state.draft.is_some() || !state.runs.is_empty(),
                egui::Button::selectable(!library, tr("Editor")),
            )
            .clicked()
        {
            state.library_open = false;
        }
        if ui
            .selectable_label(
                library,
                format!("{} ({})", tr("Loaded scripts"), state.items.len()),
            )
            .clicked()
        {
            state.library_open = true;
        }
    });
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(8.0);
}

pub(super) fn page(ui: &mut egui::Ui, store: &Store, state: &mut State, p: Palette) {
    egui::Frame::new()
        .fill(p.background)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(10)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("Loaded scripts")).strong());
                if ui
                    .add_enabled(!state.dirty(), primary(tr("+ New script"), p))
                    .clicked()
                {
                    state.new_script();
                }
            });
            ui.add_space(8.0);
            list(
                ui,
                store,
                state,
                p,
                (ui.available_height() - 20.0).max(100.0),
            );
        });
}

fn list(ui: &mut egui::Ui, store: &Store, state: &mut State, p: Palette, height: f32) -> bool {
    ui.add(
        view::line_edit(&mut state.library_search)
            .id(ui.id().with("script-library-search"))
            .hint_text(tr("Search scripts"))
            .desired_width(ui.available_width()),
    );
    let dirty = state.dirty();
    if dirty {
        ui.label(
            RichText::new(tr("Save or discard edits before switching scripts."))
                .small()
                .color(p.muted),
        );
    }
    let query = state.library_search.trim().to_lowercase();
    let mut chosen = None;
    let mut count = 0;
    egui::ScrollArea::vertical()
        .id_salt("loaded-script-list")
        .max_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for item in &state.items {
                let detail = format!(
                    "{} · {}",
                    item.language.label(),
                    trigger_name(&item.trigger)
                );
                if !query.is_empty()
                    && !format!("{} {detail}", item.name)
                        .to_lowercase()
                        .contains(&query)
                {
                    continue;
                }
                count += 1;
                ui.push_id(&item.id, |ui| {
                    let selected = state.draft.as_ref().is_some_and(|d| d.id == item.id);
                    if ui
                        .add_enabled(
                            !dirty,
                            egui::Button::selectable(selected, RichText::new(&item.name).strong())
                                .min_size(egui::vec2(ui.available_width(), 28.0))
                                .truncate(),
                        )
                        .on_hover_text(&item.name)
                        .clicked()
                    {
                        chosen = Some(item.clone());
                    }
                    ui.add(
                        egui::Label::new(RichText::new(&detail).small().color(p.muted)).truncate(),
                    )
                    .on_hover_text(&detail);
                    ui.horizontal(|ui| {
                        let active = state
                            .runs
                            .iter()
                            .find(|r| r.script_id == item.id && r.active());
                        let label = if let Some(run) = active {
                            if run.state == "queued" {
                                tr("Queued")
                            } else {
                                tr("Running")
                            }
                        } else if matches!(item.trigger, Trigger::Manual | Trigger::Provider) {
                            tr("Manual")
                        } else if item.enabled {
                            tr("Automatic trigger on")
                        } else {
                            tr("Automatic trigger off")
                        };
                        ui.add(
                            egui::Label::new(
                                RichText::new(label).small().color(if active.is_some() {
                                    p.accent
                                } else {
                                    p.muted
                                }),
                            )
                            .truncate(),
                        )
                        .on_hover_text(label);
                    });
                    ui.add_space(6.0);
                });
            }
            if count == 0 {
                ui.label(
                    RichText::new(tr(if state.items.is_empty() {
                        "No saved scripts yet."
                    } else {
                        "No matching scripts."
                    }))
                    .color(p.muted),
                );
            }
        });
    if let Some(item) = chosen {
        state.select(item);
        state.library_open = false;
        refresh(store, state);
        true
    } else {
        false
    }
}
