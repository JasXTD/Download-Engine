//! The Installed tab before anything is installed: what do you want to do, and
//! where does that happen. It answers which tab is for what by goal, not by name.
use super::*;

/// What a goal's buttons ask for.
#[derive(Clone, Copy)]
enum Goal {
    Examples,
    Script,
    Rule,
}

struct Card {
    glyph: &'static str,
    title: &'static str,
    text: &'static str,
    hint: &'static str,
    buttons: Vec<(&'static str, Goal)>,
}

fn cards() -> [Card; 3] {
    [
        Card {
            glyph: "\u{e774}",
            title: tr("Get files from a website"),
            text: tr(
                "A site reader extension reads a page you open and lists its files. You choose what to download.",
            ),
            hint: tr(
                "For example: Internet Archive items, Wikimedia Commons originals, GitHub releases.",
            ),
            buttons: vec![(tr("See examples"), Goal::Examples)],
        },
        Card {
            glyph: "\u{e945}",
            title: tr("Do something after a download"),
            text: tr(
                "An automation runs when a download or a queue finishes: sort files into folders, show a notification, start the next queue.",
            ),
            hint: tr("Install one, or write your own in Lua."),
            buttons: vec![
                (tr("See examples"), Goal::Examples),
                (tr("Write a script"), Goal::Script),
            ],
        },
        Card {
            glyph: "\u{e8fd}",
            title: tr("Send downloads to the right queue"),
            text: tr(
                "A rule picks a queue by site or file type, and can start a queue when another finishes. No code.",
            ),
            hint: tr("Rules live on the Rules tab."),
            buttons: vec![(tr("+ New rule"), Goal::Rule)],
        },
    ]
}

fn card(ui: &mut egui::Ui, card: &Card, p: Palette) -> Option<Goal> {
    let mut goal = None;
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(10)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Columns justify wrapped text, which stretches the gaps between words.
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                // The icon sits beside the title, so the buttons stay in view in a
                // small window.
                ui.horizontal_top(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 7, p.accent.gamma_multiply(0.14));
                    view::icon(ui, rect.center(), card.glyph, 15.0, p.accent);
                    ui.add(egui::Label::new(RichText::new(card.title).size(15.0).strong()).wrap());
                });
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new(card.text).color(p.muted)).wrap());
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(card.hint).small().color(p.muted)).wrap());
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    for (index, (label, choice)) in card.buttons.iter().enumerate() {
                        let button = if index == 0 {
                            ui.add(primary(*label, p))
                        } else {
                            ui.button(*label)
                        };
                        if button.clicked() {
                            goal = Some(*choice);
                        }
                    }
                });
            });
        });
    goal
}

impl Desktop {
    pub(super) fn goals(&mut self, ui: &mut egui::Ui, p: Palette) {
        let mut goal = None;
        panel(p).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            egui::ScrollArea::vertical()
                .id_salt("extension-goals")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(tr("What do you want Download Engine to do?"))
                                .size(18.0)
                                .strong(),
                        );
                        ui.label(RichText::new(tr("Pick a goal to see where to start.")).color(p.muted));
                    });
                    ui.add_space(12.0);
                    let cards = cards();
                    let width = ui.available_width().min(1080.0);
                    let margin = (ui.available_width() - width) / 2.0;
                    // Side by side when each card gets room for its buttons, else stacked.
                    let columns = if width >= 3.0 * 250.0 { 3 } else { 1 };
                    ui.horizontal(|ui| {
                        ui.add_space(margin);
                        ui.vertical(|ui| {
                            ui.set_width(width);
                            if columns == 3 {
                                ui.columns(3, |columns| {
                                    for (ui, card) in columns.iter_mut().zip(&cards) {
                                        goal = goal.or(self::card(ui, card, p));
                                    }
                                });
                            } else {
                                for card in &cards {
                                    goal = goal.or(self::card(ui, card, p));
                                    ui.add_space(10.0);
                                }
                            }
                        });
                    });
                    ui.add_space(18.0);
                    ui.vertical_centered(|ui| {
                        for line in [
                            tr("Extensions install from a file and run only with the access you approve."),
                            tr("Not the browser extension: that is the add-on in your web browser. Set it up in Preferences › Browser & AI."),
                        ] {
                            ui.add(egui::Label::new(RichText::new(line).small().color(p.muted)).wrap());
                        }
                    });
                });
        });
        match goal {
            Some(Goal::Examples) => self.open_extension_examples(),
            Some(Goal::Script) => {
                // Never replace someone's unsaved script: they land on it instead.
                if self.scripts.untouched() {
                    self.scripts.new_script();
                }
                self.automation.show_scripts();
            }
            Some(Goal::Rule) => self.open_new_rule(),
            None => {}
        }
    }
}
