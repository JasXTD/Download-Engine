//! Run with sample: choose what an event script receives. App changes are simulated.
use super::*;

/// The most downloads one event lists.
const MAX_SAMPLE: usize = 100;

pub(super) fn is_sample(run: &Run) -> bool {
    run.context["sample"].as_bool() == Some(true)
}

pub(super) struct Picker {
    script: Script,
    candidates: Vec<SampleCandidate>,
    chosen: std::collections::BTreeSet<String>,
    filter: String,
    error: Option<String>,
}
impl Picker {
    pub(super) fn open(store: &Store, script: &Script) -> anyhow::Result<Self> {
        let candidates = store.sample_candidates(script)?;
        // The newest is ticked, so trying a script on the latest event is one click.
        let chosen = candidates
            .first()
            .map(|c| c.id.clone())
            .into_iter()
            .collect();
        Ok(Self {
            script: script.clone(),
            candidates,
            chosen,
            filter: String::new(),
            error: None,
        })
    }
    /// The chosen downloads, in list order.
    fn downloads(&self) -> Vec<String> {
        self.candidates
            .iter()
            .filter(|c| self.chosen.contains(&c.id))
            .map(|c| c.id.clone())
            .collect()
    }
}

impl Desktop {
    pub(in crate::desktop) fn script_sample_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let queues = &self.queues;
        let Some(picker) = &mut self.scripts.sample else {
            self.dialog = None;
            return;
        };
        let failed = matches!(picker.script.trigger, Trigger::DownloadFailed { .. });
        let queue = match &picker.script.trigger {
            Trigger::QueueCompleted { queue } => Some(automation::queue_label(queues, queue)),
            _ => None,
        };
        let name = picker.script.name.clone();
        let subtitle = match &queue {
            Some(queue) => tf!(
                "{script} runs as if {queue} had just finished.",
                script = name,
                queue = queue
            ),
            None if failed => tf!(
                "{script} gets the downloads you tick in event.downloads, as if they had just failed.",
                script = name
            ),
            None => tf!(
                "{script} gets the downloads you tick in event.downloads, as if they had just finished.",
                script = name
            ),
        };
        let mut close = false;
        let mut run = false;
        let response = form_dialog::modal("script-sample", p).show(ctx, |ui| {
            close = form_dialog::header(
                ui,
                tr("Run with a sample event"),
                &subtitle,
                form_dialog::Icon::Symbol("\u{e945}"),
                p,
                true,
            );
            if queue.is_none() {
                choose(ui, picker, failed, queues, p);
                ui.add_space(6.0);
            }
            notice(
                ui,
                p.blue,
                tr("Changes are only simulated"),
                tr("Commands that change something, such as xd.retry, xd.rename and xd.notify, only write a line starting with Would to the log. Commands that only read, such as xd.downloads, run normally."),
                p,
            );
            if picker.script.language == Language::PowerShell {
                ui.add_space(4.0);
                notice(
                    ui,
                    p.orange,
                    tr("PowerShell commands run for real"),
                    tr("Download Engine commands are simulated, but Remove-Item, Move-Item and programs the script starts are not."),
                    p,
                );
            }
            if let Some(error) = &picker.error {
                ui.add_space(4.0);
                ui.label(RichText::new(error).size(12.0).color(p.warning));
            }
            let ready = queue.is_some() || !picker.chosen.is_empty();
            run = form_dialog::controls(ui, |ui| {
                form_dialog::primary_enabled(ui, ready, tr("Run sample")).clicked()
            });
        });
        if close || response.should_close() {
            self.scripts.sample = None;
            self.dialog = None;
        } else if run {
            self.start_sample();
        }
    }

    fn start_sample(&mut self) {
        let Some(picker) = &mut self.scripts.sample else {
            return;
        };
        match self.store.run_script_sample(
            &picker.script.id,
            &uuid::Uuid::new_v4().to_string(),
            &picker.downloads(),
        ) {
            Ok(run) => {
                self.scripts.sample = None;
                self.dialog = None;
                self.scripts.selected_run = Some(run.id);
                self.scripts.pinned_run = false;
                self.scripts.all_history = false;
                self.scripts.error = None;
                refresh(&self.store, &mut self.scripts);
            }
            Err(e) => picker.error = Some(format!("{e:#}")),
        }
    }
}

/// The filter, the downloads to tick and how many are ticked.
fn choose(
    ui: &mut egui::Ui,
    picker: &mut Picker,
    failed: bool,
    queues: &[DownloadQueue],
    p: Palette,
) {
    let count = tf!(
        "{chosen} of {total} chosen",
        chosen = picker.chosen.len(),
        total = picker.candidates.len()
    );
    ui.horizontal(|ui| {
        let width = view::button_width(ui, &count) + ui.spacing().item_spacing.x;
        ui.add(
            view::line_edit(&mut picker.filter)
                .id(egui::Id::new("script-sample-filter"))
                .hint_text(tr("Filter downloads"))
                .desired_width((ui.available_width() - width).max(120.0))
                .char_limit(200),
        );
        ui.label(RichText::new(&count).small().color(p.muted));
    });
    ui.add_space(4.0);
    if picker.candidates.is_empty() {
        ui.label(
            RichText::new(if failed {
                tr("No downloads need attention right now.")
            } else {
                tr("No finished downloads yet.")
            })
            .color(p.muted),
        );
        return;
    }
    let filter = picker.filter.trim().to_lowercase();
    let indent = view::CHECKBOX_SIZE + ui.spacing().icon_spacing;
    egui::Frame::new()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("script-sample-list")
                .max_height(260.0)
                .show(ui, |ui| {
                    let shown = picker
                        .candidates
                        .iter()
                        .filter(|c| filter.is_empty() || c.name.to_lowercase().contains(&filter));
                    for (index, candidate) in shown.enumerate() {
                        if index > 0 {
                            ui.separator();
                        }
                        let mut on = picker.chosen.contains(&candidate.id);
                        let full = picker.chosen.len() >= MAX_SAMPLE && !on;
                        let changed = ui
                            .add_enabled_ui(!full, |ui| {
                                ui.square_checkbox(&mut on, RichText::new(&candidate.name).strong())
                                    .on_disabled_hover_text(tr(
                                        "An event lists at most 100 downloads",
                                    ))
                                    .changed()
                            })
                            .inner;
                        if changed {
                            if on {
                                picker.chosen.insert(candidate.id.clone());
                            } else {
                                picker.chosen.remove(&candidate.id);
                            }
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(indent);
                            let queue = automation::queue_label(queues, &candidate.queue);
                            let meta = if failed {
                                tf!(
                                    "{queue} · Attempts: {attempts}",
                                    queue = queue,
                                    attempts = candidate.attempts
                                )
                            } else {
                                tf!(
                                    "{queue} · Added {time}",
                                    queue = queue,
                                    time = time(candidate.created as i64)
                                )
                            };
                            ui.label(RichText::new(meta).small().color(p.muted));
                            if failed && let Some(error) = &candidate.error {
                                ui.label(RichText::new("·").small().color(p.muted));
                                ui.label(RichText::new(error).small().color(p.warning));
                            }
                        });
                    }
                });
        });
}

fn notice(ui: &mut egui::Ui, color: Color32, title: &str, text: &str, p: Palette) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.30)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong());
            ui.label(RichText::new(text).small().color(p.muted));
        });
}
