//! The starter pack: five small example extensions built into the app. Choosing
//! one opens the normal install review with the same bytes as the file shipped
//! in examples/extensions, so it installs turned off like any other file.
use super::*;
use download_engine::extensions::{examples::FILES, version};
use std::sync::LazyLock;

/// Decoded once. The library tests check that every file decodes and installs.
static EXAMPLES: LazyLock<Vec<(Package, &'static [u8])>> = LazyLock::new(|| {
    FILES
        .iter()
        .filter_map(|bytes| Some((Package::decode(bytes).ok()?, *bytes)))
        .collect()
});

/// What an example can reach, in a few words for the list.
fn reach(package: &Package) -> (String, bool) {
    if package.jobs.iter().any(|job| job.change_files) {
        (tr("Asks to change files").into(), true)
    } else if package
        .jobs
        .iter()
        .any(|job| matches!(job.trigger, Trigger::QueueCompleted { .. }))
    {
        (tr("You pick the queue in the review").into(), false)
    } else if package.permissions.network.is_empty() {
        (String::new(), false)
    } else {
        (
            tf!("Reads {hosts}", hosts = package.permissions.network.join(", ")),
            false,
        )
    }
}

impl Desktop {
    pub(in crate::desktop) fn open_extension_examples(&mut self) {
        self.extensions.error = None;
        self.dialog = Some(Dialog::ExtensionExamples);
    }

    pub(in crate::desktop) fn extension_examples_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        // An example installed at its version or newer has nothing left to review.
        let installed: BTreeMap<&str, (u64, u64, u64)> = self
            .extensions
            .entries
            .iter()
            .filter_map(|entry| {
                let package = &entry.installed.package;
                Some((package.id.as_str(), version(&package.version).ok()?))
            })
            .collect();
        let mut close = false;
        let mut chosen = None;
        let mut guide = false;
        let response = form_dialog::modal("extension-examples", p).show(ctx, |ui| {
            close |= form_dialog::header(
                ui,
                tr("Examples"),
                tr("Small extensions that come with Download Engine"),
                form_dialog::Icon::Symbol(PUZZLE),
                p,
                true,
            );
            form_dialog::body(ui, "extension-examples-body", |ui| {
                let intro = tr("Each one opens the normal install review and installs turned off. Their code is short and commented, so they are also a good start for your own.");
                ui.add(egui::Label::new(RichText::new(intro).color(p.muted)).wrap());
                for (index, (package, _)) in EXAMPLES.iter().enumerate() {
                    ui.add_space(6.0);
                    ui.separator();
                    let current = installed
                        .get(package.id.as_str())
                        .is_some_and(|at| version(&package.version).is_ok_and(|v| *at >= v));
                    let action = tr("Review…");
                    let font = egui::TextStyle::Button.resolve(ui.style());
                    let button = ui
                        .painter()
                        .layout_no_wrap(action.to_owned(), font, p.text)
                        .size()
                        .x
                        + 2.0 * ui.spacing().button_padding.x
                        + 2.0 * ui.spacing().item_spacing.x;
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width((ui.available_width() - button).max(160.0));
                            ui.add(egui::Label::new(RichText::new(&package.name).strong()).wrap());
                            ui.add(
                                egui::Label::new(RichText::new(&package.description).small().color(p.muted))
                                    .wrap(),
                            );
                            ui.horizontal_wrapped(|ui| {
                                let kind = package.jobs.first().map_or("", job_kind);
                                egui::Frame::new()
                                    .fill(p.track)
                                    .corner_radius(5)
                                    .inner_margin(egui::Margin::symmetric(6, 0))
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::Label::new(RichText::new(kind).small().strong())
                                                .extend(),
                                        );
                                    });
                                let (text, asks) = reach(package);
                                if !text.is_empty() {
                                    ui.label(RichText::new(text).small().color(if asks {
                                        p.orange
                                    } else {
                                        p.muted
                                    }));
                                }
                            });
                        });
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            if current {
                                ui.label(RichText::new(tr("Installed")).small().color(p.muted));
                            } else if ui.button(action).clicked() {
                                chosen = Some(index);
                            }
                        });
                    });
                }
                ui.add_space(6.0);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(tr("Make your own: start from an example in the app's examples folder."))
                            .small()
                            .color(p.muted),
                    );
                    guide = ui.button(tr("Extension guide")).clicked();
                });
            });
        });
        close |= response.should_close();
        if let Some(index) = chosen {
            let bytes = EXAMPLES[index].1.to_vec();
            match self.store.review_extension(&bytes) {
                // The review replaces this dialog, as if the file had been picked.
                Ok(review) => self.review_extension(bytes, review),
                Err(error) => {
                    self.extensions.error = Some(format!("{error:#}"));
                    self.dialog = None;
                }
            }
        } else if guide {
            self.dialog = None;
            self.open_guide("Extensions");
        } else if close {
            self.dialog = None;
        }
    }
}
