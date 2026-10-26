//! Queue creation, batches and schedule forms.
use super::*;
use download_engine::queue::{LinkPattern, Padding};

/// Add batch → From a pattern. Kept apart from the pasted list so switching keeps both.
pub(super) struct PatternDraft {
    pub(super) active: bool,
    pub(super) pattern: String,
    pub(super) from: String,
    pub(super) to: String,
    pub(super) padding: Padding,
}

impl Default for PatternDraft {
    fn default() -> Self {
        Self {
            active: false,
            pattern: String::new(),
            from: "1".into(),
            to: "10".into(),
            padding: Padding::Auto,
        }
    }
}

impl PatternDraft {
    fn links(&self) -> anyhow::Result<LinkPattern> {
        LinkPattern::new(&self.pattern, &self.from, &self.to, self.padding)
    }
}

impl Desktop {
    pub(super) fn current_queue_name(&self) -> &str {
        self.queues
            .iter()
            .find(|q| q.id == self.queue_id)
            .map(|q| display_queue_name(&q.id, &q.name))
            .unwrap_or(tr("Main queue"))
    }

    pub(super) fn open_queue_dialog(&mut self, dialog: Dialog) {
        self.dialog = Some(dialog);
        self.form_error = None;
        if dialog == Dialog::NewQueue {
            self.queue_synchronize = false;
        }
        if dialog == Dialog::Schedule {
            let next = chrono::Local::now() + chrono::Duration::hours(1);
            self.schedule_date = next.format("%Y-%m-%d").to_string();
            self.schedule_time = next.format("%H:%M").to_string();
            self.schedule_stop = "07:00".into();
            self.schedule_mode = ScheduleMode::Once;
            self.schedule_days = [true, true, true, true, true, false, false];
            if let Some(queue) = self.queues.iter().find(|queue| queue.id == self.queue_id) {
                if let Some(schedule) = &queue.recurring {
                    self.schedule_mode = if schedule.days == EVERY_DAY {
                        ScheduleMode::Daily
                    } else {
                        ScheduleMode::SelectedDays
                    };
                    self.schedule_time = clock_label(schedule.start_minute);
                    self.schedule_stop = clock_label(schedule.stop_minute);
                    self.schedule_days = std::array::from_fn(|i| schedule.days & (1 << i) != 0);
                } else if let Some(at) = queue
                    .scheduled_at
                    .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
                {
                    let at = at.with_timezone(&chrono::Local);
                    self.schedule_date = at.format("%Y-%m-%d").to_string();
                    self.schedule_time = at.format("%H:%M").to_string();
                }
            }
        }
    }

    pub(super) fn recurring_schedule_draft(&self) -> anyhow::Result<RecurringSchedule> {
        let days = if self.schedule_mode == ScheduleMode::Daily {
            EVERY_DAY
        } else {
            self.schedule_days
                .iter()
                .enumerate()
                .fold(0, |days, (i, selected)| days | (u8::from(*selected) << i))
        };
        RecurringSchedule::new(days, &self.schedule_time, &self.schedule_stop)
    }

    pub(super) fn submit_queue_dialog(&mut self, dialog: Dialog) {
        let result: anyhow::Result<()> = (|| {
            match dialog {
                Dialog::Batch => {
                    // Generated links take the same validation, folder and queue path as a list.
                    let generated = if self.batch_pattern.active {
                        Some(
                            self.batch_pattern
                                .links()?
                                .links()
                                .collect::<Vec<_>>()
                                .join("\n"),
                        )
                    } else {
                        None
                    };
                    let text = generated.as_deref().unwrap_or(&self.batch);
                    let jobs = self
                        .store
                        .new_batch_downloads(text, self.folder_override().as_deref())?;
                    let jobs = self.store.enqueue_batch(&jobs, &self.queue_id)?;
                    self.set_selection(jobs.first().map(|j| j.id.clone()));
                    self.filter = Filter::All;
                    self.search.clear();
                    self.view = View::Downloads;
                    self.category = None;
                    self.notice = Some(Notice::success(tf!(
                        "Added {} files to {}.",
                        jobs.len(),
                        self.current_queue_name()
                    )));
                    if self.batch_pattern.active {
                        self.batch_pattern.pattern.clear();
                    } else {
                        self.batch.clear();
                    }
                }
                Dialog::NewQueue => {
                    let queue = if self.queue_synchronize {
                        self.store.create_sync_queue(&self.queue_name)?
                    } else {
                        self.store.create_queue(&self.queue_name)?
                    };
                    self.queue_id = queue.id;
                    self.set_selection(None);
                    self.filter = Filter::All;
                    self.search.clear();
                    self.view = View::Downloads;
                    self.category = None;
                }
                Dialog::RenameQueue => self.store.rename_queue(&self.queue_id, &self.queue_name)?,
                Dialog::Schedule => {
                    let command = if self.schedule_mode == ScheduleMode::Once {
                        let at = local_schedule(
                            &self.schedule_date,
                            &self.schedule_time,
                            chrono::Utc::now().timestamp(),
                        )?;
                        Command::ScheduleQueue(self.queue_id.clone(), at)
                    } else {
                        Command::ScheduleRecurringQueue(
                            self.queue_id.clone(),
                            self.recurring_schedule_draft()?,
                        )
                    };
                    self.scheduler.send(command)?;
                }
                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.dialog = None;
                self.form_error = None;
                self.force_refresh();
            }
            Err(error) => self.form_error = Some(format!("{error:#}")),
        }
    }

    fn batch_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let mut submit = false;
        let mut cancel = false;
        let mut preview_error = None;
        let response = form_dialog::modal("batch-queue-dialog", p).show(ctx, |ui| {
            cancel = form_dialog::header(
                ui,
                tr("Add batch"),
                if self.batch_pattern.active {
                    tr("Generate numbered links from one pattern.")
                } else {
                    tr("Paste one file link per line. Duplicate links are skipped.")
                },
                form_dialog::Icon::Batch,
                p,
                true,
            );
            egui::ScrollArea::vertical()
                .id_salt("batch-form")
                .max_height((ctx.content_rect().height() - 220.0).max(150.0))
                .show(ui, |ui| {
                    egui::Frame::new()
                        .fill(p.background)
                        .stroke(egui::Stroke::new(1.0, p.border))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 6.0;
                            ui.horizontal(|ui| {
                                for (active, label) in
                                    [(false, tr("Link list")), (true, tr("From a pattern"))]
                                {
                                    let mode = &mut self.batch_pattern.active;
                                    if ui.selectable_value(mode, active, label).changed() {
                                        self.form_error = None;
                                    }
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(tf!(
                                                "Add to {}",
                                                self.current_queue_name()
                                            ))
                                            .small()
                                            .color(p.muted),
                                        );
                                    },
                                );
                            });
                            if self.batch_pattern.active {
                                preview_error = self.batch_pattern_fields(ui, p);
                            } else {
                                ui.label(RichText::new(tr("File links")).strong());
                                let label_id = egui::Id::new("batch-links-label");
                                egui::ScrollArea::vertical()
                                    .id_salt("batch-links")
                                    .max_height(132.0)
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::TextEdit::multiline(&mut self.batch)
                                                .desired_rows(5)
                                                .desired_width(f32::INFINITY)
                                                .hint_text("https://example.com/file.zip"),
                                        )
                                        .labelled_by(label_id);
                                    });
                            }
                            ui.add_space(2.0);
                            ui.label(RichText::new(tr("Save to (optional override)")).strong());
                            self.folder_picker.field(
                                ui,
                                &mut self.folder,
                                &self.store.paths.downloads,
                                &mut self.form_error,
                            );
                            ui.label(
                                RichText::new(tr("Leave blank to use folder rules for each file."))
                                    .small()
                                    .color(p.muted),
                            );
                        });
                    // The preview already shows a pattern error; do not repeat it below.
                    if let Some(error) = &self.form_error
                        && preview_error.as_ref() != Some(error)
                    {
                        ui.add_space(4.0);
                        ui.label(RichText::new(error).small().color(p.warning));
                    }
                });
            submit = form_dialog::footer(ui, tr("Add files"), None, p);
        });
        if cancel || response.should_close() {
            self.dialog = None;
        } else if submit {
            self.submit_queue_dialog(Dialog::Batch);
        }
    }

    /// Pattern, range and padding fields with a live preview. Returns the preview's error.
    fn batch_pattern_fields(&mut self, ui: &mut egui::Ui, p: Palette) -> Option<String> {
        let draft = &mut self.batch_pattern;
        let label = ui.label(RichText::new(tr("Link pattern")).strong());
        ui.add(
            view::line_edit(&mut draft.pattern)
                .id(egui::Id::new("batch-pattern"))
                .desired_width(f32::INFINITY)
                .vertical_align(egui::Align::Center)
                .hint_text("https://example.com/photos/img*.jpg"),
        )
        .labelled_by(label.id);
        ui.label(
            RichText::new(tr("Put one * where the number or letter goes."))
                .small()
                .color(p.muted),
        );
        let preview = draft.links();
        let letters = preview.as_ref().is_ok_and(LinkPattern::letters);
        ui.horizontal(|ui| {
            for (id, title, value) in [
                ("batch-pattern-from", tr("From"), &mut draft.from),
                ("batch-pattern-to", tr("To"), &mut draft.to),
            ] {
                ui.vertical(|ui| {
                    let label = ui.label(title);
                    ui.add(
                        view::line_edit(value)
                            .id(egui::Id::new(id))
                            .desired_width(96.0)
                            .vertical_align(egui::Align::Center),
                    )
                    .labelled_by(label.id);
                });
            }
            ui.vertical(|ui| {
                ui.label(tr("Zero padding"));
                let name = |padding| match padding {
                    Padding::Auto => tr("Auto").to_owned(),
                    Padding::Width(0) => tr("Off").to_owned(),
                    Padding::Width(width) => width.to_string(),
                };
                // Letters have no digits to pad.
                ui.add_enabled_ui(!letters, |ui| {
                    egui::ComboBox::from_id_salt("batch-pattern-padding")
                        .selected_text(name(draft.padding))
                        .width(110.0)
                        .show_ui(ui, |ui| {
                            for padding in [Padding::Auto, Padding::Width(0)]
                                .into_iter()
                                .chain((2..=8).map(Padding::Width))
                            {
                                ui.selectable_value(&mut draft.padding, padding, name(padding));
                            }
                        });
                });
            });
        });
        if draft.pattern.trim().is_empty() {
            return None;
        }
        let tone = if preview.is_ok() { p.accent } else { p.warning };
        egui::Frame::new()
            .fill(tone.gamma_multiply(0.08))
            .stroke(egui::Stroke::new(1.0, tone.gamma_multiply(0.45)))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                match &preview {
                    Ok(links) => {
                        let count = links.count();
                        let title = if count == 1 {
                            tr("Preview · 1 link").to_owned()
                        } else {
                            tf!("Preview · {} links", count)
                        };
                        ui.label(RichText::new(title).small().strong());
                        let link = |ui: &mut egui::Ui, text: String| {
                            ui.add(
                                egui::Label::new(RichText::new(text).small().monospace())
                                    .truncate(),
                            );
                        };
                        link(ui, links.first());
                        if count > 2 {
                            ui.label(RichText::new("…").small().color(p.muted));
                        }
                        if count > 1 {
                            link(ui, links.last());
                        }
                    }
                    Err(error) => {
                        ui.label(RichText::new(format!("{error:#}")).small().color(p.warning));
                    }
                }
            });
        preview.err().map(|error| format!("{error:#}"))
    }

    pub(super) fn queue_dialog(&mut self, ctx: &egui::Context, p: Palette, dialog: Dialog) {
        if dialog == Dialog::NewQueue {
            self.new_queue_dialog(ctx, p);
            return;
        }
        if dialog == Dialog::Batch {
            self.batch_dialog(ctx, p);
            return;
        }
        let (modal_id, title, action) = match dialog {
            Dialog::RenameQueue => ("rename-queue-dialog", tr("Rename queue"), tr("Save name")),
            Dialog::Schedule => (
                "schedule-queue-dialog",
                tr("Schedule queue"),
                tr("Set schedule"),
            ),
            _ => return,
        };
        let mut submit = false;
        let mut cancel = false;
        let response = form_dialog::modal(modal_id, p).show(ctx, |ui| {
            ui.set_width(if dialog == Dialog::RenameQueue {
                400.0
            } else {
                460.0
            });
            cancel = form_dialog::title(ui, title, form_dialog::Icon::Queue, p, true);
            egui::ScrollArea::vertical()
                .id_salt("queue-settings-body")
                .max_height((ctx.content_rect().height() - 220.0).max(150.0))
                .show(ui, |ui| {
                    match dialog {
                        Dialog::RenameQueue => {
                            egui::Frame::new()
                                .fill(p.background)
                                .stroke(egui::Stroke::new(1.0, p.border))
                                .corner_radius(8)
                                .inner_margin(egui::Margin::symmetric(12, 10))
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.spacing_mut().item_spacing.y = 6.0;
                                    ui.label(RichText::new(tr("Queue name")).strong());
                                    ui.add(
                                        view::line_edit(&mut self.queue_name)
                                            .desired_width(f32::INFINITY),
                                    );
                                });
                        }
                        Dialog::Schedule => {
                            egui::Frame::new()
                                .fill(p.background)
                                .stroke(egui::Stroke::new(1.0, p.border))
                                .corner_radius(8)
                                .inner_margin(egui::Margin::symmetric(12, 10))
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.spacing_mut().item_spacing.y = 8.0;
                                    ui.horizontal(|ui| {
                                        ui.strong(self.current_queue_name());
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.selectable_value(
                                                    &mut self.schedule_mode,
                                                    ScheduleMode::SelectedDays,
                                                    tr("Selected days"),
                                                );
                                                ui.selectable_value(
                                                    &mut self.schedule_mode,
                                                    ScheduleMode::Daily,
                                                    tr("Daily"),
                                                );
                                                ui.selectable_value(
                                                    &mut self.schedule_mode,
                                                    ScheduleMode::Once,
                                                    tr("One time"),
                                                );
                                            },
                                        );
                                    });
                                    if self.schedule_mode == ScheduleMode::SelectedDays {
                                        ui.label(RichText::new(tr("Start on")).small().color(p.muted));
                                        ui.horizontal_wrapped(|ui| {
                                            for (i, day) in DAY_NAMES.iter().enumerate() {
                                                ui.square_checkbox(&mut self.schedule_days[i], tr(day));
                                            }
                                        });
                                    }
                                    ui.horizontal(|ui| {
                                        if self.schedule_mode == ScheduleMode::Once {
                                            ui.vertical(|ui| {
                                                ui.label(tr("Date (YYYY-MM-DD)"));
                                                ui.add(
                                                    view::line_edit(
                                                        &mut self.schedule_date,
                                                    )
                                                    .desired_width(170.0),
                                                );
                                            });
                                        }
                                        ui.vertical(|ui| {
                                            ui.label(tr("Start (HH:MM)"));
                                            ui.add(
                                                view::line_edit(&mut self.schedule_time)
                                                    .desired_width(115.0),
                                            );
                                        });
                                        if self.schedule_mode != ScheduleMode::Once {
                                            ui.vertical(|ui| {
                                                ui.label(tr("Pause (HH:MM)"));
                                                ui.add(
                                                    view::line_edit(
                                                        &mut self.schedule_stop,
                                                    )
                                                    .desired_width(115.0),
                                                );
                                            });
                                        }
                                    });
                                    if self.schedule_mode != ScheduleMode::Once
                                        && let Ok(schedule) = self.recurring_schedule_draft()
                                    {
                                        ui.label(
                                            RichText::new(display_schedule(&schedule))
                                                .strong()
                                                .color(p.accent),
                                        );
                                    }
                                });
                            ui.add_space(6.0);
                            if self.schedule_mode == ScheduleMode::Once {
                                ui.label(
                                    RichText::new(
                                        tr("Local time · Pauses queue until start (runs on reopen if missed)."),
                                    )
                                    .small()
                                    .color(p.muted),
                                );
                            } else {
                                ui.label(
                                    RichText::new(
                                        tr("Local time · Manual Start/Pause suspends schedule until re-enabled in Scheduler."),
                                    )
                                    .small()
                                    .color(p.muted),
                                );
                            }
                        }
                        _ => {}
                    }
                    if let Some(error) = &self.form_error {
                        ui.add_space(4.0);
                        ui.label(RichText::new(error).small().color(p.warning));
                    }
                });
            submit = form_dialog::footer(ui, action, None, p);
        });
        if cancel || response.should_close() {
            self.dialog = None;
        } else if submit {
            self.submit_queue_dialog(dialog);
        }
    }
}
