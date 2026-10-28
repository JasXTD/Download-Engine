//! Keep toolbar labels readable; commands which do not fit remain in More.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Downloads,
    Add,
    Resume,
    Pause,
    Stop,
    StopAll,
    Remove,
    Folder,
    Clear,
    Queues,
    Scheduler,
    Scripts,
    Grabber,
    Mcp,
    Logs,
    Preferences,
}

const OPTIONAL: [Action; 10] = [
    Action::Remove,
    Action::Folder,
    Action::Clear,
    Action::Queues,
    Action::Scheduler,
    Action::Scripts,
    Action::Grabber,
    Action::Mcp,
    Action::Logs,
    Action::Preferences,
];
const DOWNLOADS: [Action; 5] = [
    Action::Add,
    Action::Resume,
    Action::Pause,
    Action::Stop,
    Action::StopAll,
];

impl Action {
    fn label(self) -> &'static str {
        tr(match self {
            Self::Downloads => "Download list",
            Self::Add => "Add URL",
            Self::Resume => "Resume",
            Self::Pause => "Pause",
            Self::Stop => "Stop",
            Self::StopAll => "Stop all",
            Self::Remove => "Remove",
            Self::Folder => "Open folder",
            Self::Clear => "Clear completed history",
            Self::Queues => "Queues",
            Self::Scheduler => "Scheduler",
            Self::Scripts => "Scripting",
            Self::Grabber => "Grabber",
            Self::Mcp => "MCP",
            Self::Logs => "Logs",
            Self::Preferences => "Preferences",
        })
    }
    fn glyph(self) -> &'static str {
        match self {
            Self::Downloads => ALL,
            Self::Add => ADD,
            Self::Resume => PLAY,
            Self::Pause => PAUSE,
            Self::Stop | Self::StopAll => STOP,
            Self::Remove => DELETE,
            Self::Folder => FOLDER,
            Self::Clear => CHECK,
            Self::Queues => QUEUE,
            Self::Scheduler => CALENDAR,
            Self::Scripts => AUTOMATION,
            Self::Grabber => GRABBER,
            Self::Mcp => "\u{e839}",
            Self::Logs => LOGS,
            Self::Preferences => SETTINGS,
        }
    }
    fn hint(self) -> &'static str {
        tr(match self {
            Self::Add => "Add a download (Ctrl+N)",
            Self::Resume => "Resume selected downloads; files without a resume plan start again",
            Self::Stop => {
                "Stop the selected downloads and close their status windows. Saved progress is kept."
            }
            Self::StopAll => {
                "Stop current and waiting downloads in every queue, including files started immediately. Saved progress is kept; resume files when ready."
            }
            Self::Remove => {
                "Remove the selected entry and its partial data. Completed files are kept."
            }
            Self::Clear => "Clear completed entries from the selected queue. Saved files are kept.",
            Self::Scripts => "Rules, scripts, site readers and run history",
            Self::Grabber => "Find website files or save an offline website",
            Self::Mcp => "AI connection settings and server controls",
            _ => return self.label(),
        })
    }
    fn file_action(self) -> bool {
        matches!(self, Self::Remove | Self::Folder | Self::Clear)
    }
}

fn preferred_width(ui: &egui::Ui, action: Action) -> f32 {
    let mut labels = vec![action.label()];
    if action == Action::Resume {
        labels.push(tr("Start again"));
    }
    let base = labels
        .iter()
        .map(|label| whole_word_width(ui, label, LABEL_LARGEST) + LABEL_PADDING + 8.0)
        .fold(68.0_f32, f32::max);
    let mut width = base;
    while width < 180.0
        && labels.iter().any(|label| {
            label_galley(
                ui,
                label,
                LABEL_LARGEST,
                width - LABEL_PADDING,
                Color32::WHITE,
            )
            .1
        })
    {
        width += 8.0;
    }
    width
}

impl Desktop {
    fn toolbar_enabled(&self, action: Action) -> bool {
        match action {
            Action::Resume => self
                .selected_jobs()
                .iter()
                .any(|j| matches!(j.status, Status::Paused | Status::Failed)),
            Action::Pause | Action::Stop => self
                .selected_jobs()
                .iter()
                .any(|j| matches!(j.status, Status::Downloading | Status::Queued)),
            Action::Remove => {
                let jobs = self.selected_jobs();
                !jobs.is_empty() && jobs.iter().all(|j| j.status != Status::Downloading)
            }
            Action::Folder => self.selected_job().is_some(),
            Action::Clear => self.jobs.iter().any(|j| j.status == Status::Complete),
            _ => true,
        }
    }
    fn toolbar_action(&mut self, action: Action) {
        match action {
            Action::Downloads => self.view = View::Downloads,
            Action::Add => self.open_add(),
            Action::Resume => self.selection_command(false),
            Action::Pause => self.selection_command(true),
            Action::Stop => self.stop_selected_downloads(),
            Action::StopAll => self.stop_all_downloads(),
            Action::Remove => {
                self.request_destructive(DestructiveAction::Remove, self.selected_jobs())
            }
            Action::Folder => {
                if let Some(job) = self.selected_job() {
                    self.show_folder(&job);
                }
            }
            Action::Clear => self.clear_finished(),
            Action::Queues => {
                self.view = View::Queues;
                self.refresh_queue_summaries();
            }
            Action::Scheduler => self.open_queue_dialog(Dialog::Schedule),
            Action::Scripts => {
                self.view = View::Automation;
                self.refresh_automation();
            }
            Action::Grabber => self.open_grabber(),
            Action::Mcp => self.open_mcp(),
            Action::Logs => {
                self.view = View::Logs;
                self.logs.request_refresh();
            }
            Action::Preferences => self.open_preferences_window(),
        }
    }

    pub(super) fn responsive_toolbar(&mut self, ui: &mut egui::Ui, p: Palette) {
        let downloads = matches!(self.view, View::Downloads | View::Queues | View::Scheduler);
        let mut actions = if downloads {
            DOWNLOADS.to_vec()
        } else {
            vec![Action::Downloads]
        };
        actions.extend(
            OPTIONAL
                .into_iter()
                .filter(|action| downloads || !action.file_action()),
        );
        egui::Panel::top("download-toolbar")
            .exact_size(if downloads { 78.0 } else { 44.0 })
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.panel(p.background))
                    .inner_margin(egui::Margin::symmetric(10, 7)),
            )
            .show(ui, |ui| {
                let mut remaining = ui.available_width() - 72.0; // More is always available for customization.
                let mut hidden = Vec::new();
                let mut shown = Vec::new();
                for action in actions {
                    let pinned = OPTIONAL
                        .iter()
                        .position(|a| *a == action)
                        .is_none_or(|i| self.interface.toolbar_pins[i]);
                    let width = if downloads {
                        preferred_width(ui, action)
                    } else {
                        ui.painter()
                            .layout_no_wrap(
                                action.label().into(),
                                FontId::proportional(13.0),
                                p.text,
                            )
                            .size()
                            .x
                            .max(42.0)
                            + 20.0
                    };
                    if pinned && width + 4.0 <= remaining {
                        shown.push((action, width));
                        remaining -= width + 4.0;
                    } else {
                        hidden.push(action);
                    }
                }
                let look = row_look(
                    ui,
                    &shown
                        .iter()
                        .map(|(a, w)| (a.label(), *w))
                        .chain(std::iter::once((tr("More"), 68.0)))
                        .collect::<Vec<_>>(),
                );
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (action, width) in shown {
                        let label = if action == Action::Resume {
                            self.selected_job()
                                .as_ref()
                                .map(resume_action)
                                .unwrap_or(action.label())
                        } else {
                            action.label()
                        };
                        let active = match action {
                            Action::Queues => self.view == View::Queues,
                            Action::Scripts => self.view == View::Automation,
                            Action::Logs => self.view == View::Logs,
                            Action::Preferences => self.preferences_window.is_some(),
                            Action::Grabber => self.dialog == Some(Dialog::Grabber),
                            Action::Mcp => self.dialog == Some(Dialog::Mcp),
                            _ => false,
                        };
                        let color = match action {
                            Action::Add | Action::Resume => p.accent,
                            Action::Pause => p.orange,
                            Action::Stop | Action::StopAll => p.warning,
                            _ => p.text,
                        };
                        let response = ui
                            .add_enabled_ui(self.toolbar_enabled(action), |ui| {
                                if downloads {
                                    tool(ui, label, action.glyph(), color, width, active, look)
                                } else {
                                    ui.add_sized(
                                        [width, 30.0],
                                        egui::Button::new(label).selected(active),
                                    )
                                }
                            })
                            .inner
                            .on_hover_text(action.hint())
                            .on_disabled_hover_text(action.hint());
                        if response.clicked() {
                            self.toolbar_action(action);
                        }
                    }
                    let more = if downloads {
                        tool(ui, tr("More"), "\u{e712}", p.text, 68.0, false, look)
                    } else {
                        ui.add_sized([68.0, 30.0], egui::Button::new(tr("More")))
                    };
                    egui::Popup::menu(&more).show(|ui| {
                        for action in hidden {
                            if ui
                                .add_enabled(
                                    self.toolbar_enabled(action),
                                    egui::Button::new(action.label()),
                                )
                                .on_hover_text(action.hint())
                                .clicked()
                            {
                                self.toolbar_action(action);
                                ui.close();
                            }
                        }
                        ui.separator();
                        ui.menu_button(tr("Customize toolbar"), |ui| {
                            ui.label(tr("Pinned actions appear when space allows."));
                            let mut preferences = self.interface;
                            for (i, action) in OPTIONAL.iter().enumerate() {
                                ui.checkbox(&mut preferences.toolbar_pins[i], action.label());
                            }
                            if preferences != self.interface {
                                self.save_interface(ui.ctx(), preferences);
                            }
                        });
                        #[cfg(feature = "screenshot")]
                        capture_menu_contents(ui, tr("More"));
                    });
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::interaction_tests::{fixture, frame, text_rect};

    fn text(output: &egui::FullOutput, label: &str) -> Option<egui::Rect> {
        output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text.trim() == label => {
                Some(t.galley.rect.translate(t.pos.to_vec2()))
            }
            _ => None,
        })
    }

    #[test]
    fn every_toolbar_command_is_visible_or_reachable_in_more_in_every_language() {
        for language in download_engine::i18n::Language::ALL {
            for size in [vec2(860.0, 600.0), vec2(1280.0, 860.0)] {
                let (_root, mut app, ctx) = fixture();
                app.interface.language = language;
                let _scope = download_engine::i18n::scope(language);
                let selected = app.selected.clone();
                frame(&mut app, &ctx, size, vec![]);
                let output = frame(&mut app, &ctx, size, vec![]);
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                let mut missing = Vec::new();
                for action in OPTIONAL {
                    if let Some(rect) = text(&output, action.label()) {
                        assert!(
                            screen.contains_rect(rect),
                            "{language:?}: {}",
                            action.label()
                        );
                    } else {
                        missing.push(action);
                    }
                }
                for shape in &output.shapes {
                    if let egui::Shape::Text(t) = &shape.shape
                        && (30.0..108.0).contains(&t.pos.y)
                        && !t.galley.job.text.trim().is_empty()
                        && t.galley.job.sections[0].format.font_id.size < 20.0
                    {
                        assert_eq!(t.galley.job.sections[0].format.font_id.size, 11.0);
                        assert!(!t.galley.elided, "{language:?}: {}", t.galley.job.text);
                    }
                }
                let pos = text_rect(&output, tr("More")).center();
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
                let output = frame(&mut app, &ctx, size, vec![]);
                for action in missing {
                    let rect = text_rect(&output, action.label());
                    assert!(
                        screen.contains_rect(rect),
                        "{language:?}: overflow {}",
                        action.label()
                    );
                }
                text_rect(&output, tr("Customize toolbar"));
                assert_eq!(app.selected, selected);
            }
        }
    }

    #[test]
    fn toolbar_pin_choice_persists_and_preferences_cancel_preserves_it() {
        use crate::desktop::interaction_tests::click;
        let (_root, mut app, ctx) = fixture();
        click(&mut app, &ctx, "More");
        click(&mut app, &ctx, "Customize toolbar");
        click(&mut app, &ctx, "Grabber");
        assert!(!app.interface.toolbar_pins[6]);
        assert!(!app.store.interface_preferences().unwrap().toolbar_pins[6]);
        egui::Popup::close_all(&ctx);
        app.open_preferences_window();
        app.preferences_window
            .as_mut()
            .unwrap()
            .interface
            .compact_rows = true;
        click(&mut app, &ctx, "Cancel");
        assert!(!app.interface.toolbar_pins[6]);
        assert!(!app.interface.compact_rows);
    }
}
