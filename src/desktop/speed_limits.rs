//! Global and per-download speed settings.
use super::*;

impl Desktop {
    pub(super) fn open_speed_limits(&mut self) {
        match self.store.bandwidth_limits() {
            Ok(limits) => self.limits = limits,
            Err(error) => {
                self.notice = Some(Notice::error(error.to_string()));
                return;
            }
        }
        self.speed_limits = Some(SpeedDialog {
            global: SpeedField::new(self.limits.global),
            download: self
                .jobs
                .iter()
                .find(|j| self.selected.as_deref() == Some(&j.id) && j.status != Status::Complete)
                .map(|job| {
                    (
                        job.id.clone(),
                        job.name.clone(),
                        SpeedField::new(self.limits.downloads.get(&job.id).copied()),
                    )
                }),
            request: None,
        });
        self.form_error = None;
        self.dialog = Some(Dialog::SpeedLimits);
    }
    pub(super) fn save_speed_limits(&mut self) {
        let Some(form) = &mut self.speed_limits else {
            return;
        };
        if form.request.is_some() {
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            let global = form.global.rate()?;
            let download = form
                .download
                .as_ref()
                .map(|(id, _, field)| Ok::<_, anyhow::Error>((id.clone(), field.rate()?)))
                .transpose()?;
            let request = Uuid::new_v4();
            self.scheduler.send(Command::Bandwidth(bandwidth::Update {
                request,
                global,
                download,
            }))?;
            form.request = Some(request);
            Ok(())
        })();
        self.form_error = result.err().map(|e| e.to_string());
    }
    pub(super) fn bandwidth_event(&mut self, event: bandwidth::Event) {
        if self.status_bandwidth_event(&event) {
            return;
        }
        let Some(form) = &mut self.speed_limits else {
            return;
        };
        if form.request != Some(event.request) {
            return;
        }
        match event.result {
            Ok(limits) => {
                self.note_advanced_preferences_saved();
                self.limits = limits;
                self.speed_limits = None;
                self.dialog = None;
                self.form_error = None;
                self.notice = Some(Notice::success(
                    tr("Speed limits saved and applied to active downloads.").into(),
                ));
            }
            Err(error) => {
                form.request = None;
                self.form_error = Some(error);
            }
        }
    }
    pub(super) fn speed_limits_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(form) = &mut self.speed_limits else {
            self.dialog = None;
            return;
        };
        let saving = form.request.is_some();
        let mut submit = false;
        let mut cancel = false;
        let response = form_dialog::modal("speed-limits", p).show(ctx, |ui| {
            ui.set_width(450.0);
            cancel = form_dialog::title(
                ui,
                tr("Speed limits"),
                form_dialog::Icon::Symbol("\u{e9d9}"),
                p,
                !saving,
            );
            ui.add_space(6.0);
            let labels = [tr("Limit total speed"), tr("Limit this file")];
            let label_width = view::checkbox_column(ui, &labels);
            ui.add_enabled_ui(!saving, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                egui::Frame::new()
                    .fill(p.background)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.strong(tr("All downloads and queues"));
                        ui.push_id("total-speed", |ui| {
                            form.global.draw(ui, labels[0], label_width)
                        });
                    });
                egui::Frame::new()
                    .fill(p.background)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if let Some((_, name, field)) = &mut form.download {
                            ui.add(egui::Label::new(RichText::new(&*name).strong()).truncate())
                                .on_hover_text(&*name);
                            ui.push_id("file-speed", |ui| field.draw(ui, labels[1], label_width));
                        } else {
                            ui.label(
                                RichText::new(tr(
                                    "Select an unfinished file to set its own speed limit.",
                                ))
                                .small()
                                .color(p.muted),
                            );
                        }
                    });
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr("Applies immediately · 1,024 KiB/s = 1 MiB/s"))
                    .small()
                    .color(p.muted),
            );
            if let Some(error) = &self.form_error {
                ui.label(RichText::new(error).color(p.warning));
            }
            form_dialog::controls(ui, |ui| {
                submit = form_dialog::primary_enabled(ui, !saving, tr("Apply limits")).clicked();
                if saving {
                    ui.spinner();
                }
            });
        });
        if !saving && (cancel || response.should_close()) {
            self.speed_limits = None;
            self.dialog = None;
            self.form_error = None;
        } else if submit {
            self.save_speed_limits();
        }
    }
}
