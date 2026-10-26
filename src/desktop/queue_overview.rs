//! Per-queue cards and cached totals. Scheduler commands retain their existing semantics.
use super::*;
use egui::{Align, Layout, Rect, Stroke, pos2, vec2};

#[derive(Clone, Debug, Default)]
pub(super) struct Summary {
    pub files: usize,
    pub active: usize,
    pub complete: usize,
    pub downloaded: u64,
    pub total: u64,
    pub unknown: usize,
}
impl Summary {
    fn from_jobs(jobs: &[Job]) -> Self {
        let mut summary = Self {
            files: jobs.len(),
            ..Self::default()
        };
        for job in jobs {
            summary.active += usize::from(job.status == Status::Downloading);
            summary.complete += usize::from(job.status == Status::Complete);
            let total = job
                .total
                .or_else(|| (job.status == Status::Complete).then_some(job.downloaded));
            summary.unknown += usize::from(total.is_none());
            summary.total = summary.total.saturating_add(total.unwrap_or(0));
            summary.downloaded = summary
                .downloaded
                .saturating_add(total.map_or(job.downloaded, |total| job.downloaded.min(total)));
        }
        summary
    }
}

impl Desktop {
    pub(super) fn refresh_queue_summaries(&mut self) {
        self.queue_summaries
            .retain(|id, _| self.queues.iter().any(|queue| &queue.id == id));
        for queue in &self.queues {
            self.queue_summaries.insert(
                queue.id.clone(),
                self.store
                    .queue_jobs(&queue.id)
                    .map(|jobs| Summary::from_jobs(&jobs))
                    .map_err(|error| error.to_string()),
            );
        }
    }

    pub(super) fn overview(&mut self, ui: &mut egui::Ui, p: Palette) {
        if self
            .queues
            .iter()
            .any(|queue| !self.queue_summaries.contains_key(&queue.id))
        {
            self.refresh_queue_summaries();
        }
        let scheduler = self.view == View::Scheduler;
        ui.horizontal(|ui| {
            // Measured, so the button ends at the cards' edge however long the translation is.
            let new_queue = tr("+ New queue");
            let button = view::button_width(ui, new_queue).max(110.0);
            let width = (ui.available_width() - ui.spacing().item_spacing.x - button).max(260.0);
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.label(
                    RichText::new(if scheduler { tr("Scheduler") } else { tr("Queues") })
                        .size(18.0)
                        .strong(),
                );
                ui.label(
                    RichText::new(if scheduler {
                        tr("Schedule website scans and download queues.")
                    } else {
                        tr("Group downloads, limit how many run at once, and schedule when they start.")
                    })
                    .small()
                    .color(p.muted),
                );
            });
            if ui
                .add_sized([button, view::PRIMARY_HEIGHT], primary(new_queue, p))
                .clicked()
            {
                self.queue_name.clear();
                self.open_queue_dialog(Dialog::NewQueue);
            }
        });
        ui.add_space(14.0);
        self.notice(ui, p);
        egui::ScrollArea::vertical()
            .id_salt("queue-overview")
            .show(ui, |ui| {
                if scheduler {
                    self.grabber_schedule_cards(ui, p);
                }
                for queue in self.queues.clone() {
                    let summary = self.queue_summaries.get(&queue.id).cloned();
                    ui.push_id(&queue.id, |ui| {
                        egui::Frame::new()
                            .fill(p.background)
                            .stroke(Stroke::new(1.0, p.border))
                            .corner_radius(8)
                            .inner_margin(14)
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                let narrow = ui.available_width() < 720.0;
                                let (header, _) = ui.allocate_exact_size(
                                    vec2(ui.available_width(), if narrow { 78.0 } else { 44.0 }),
                                    egui::Sense::hover(),
                                );
                                let icon = Rect::from_min_size(header.min, vec2(34.0, 34.0));
                                ui.painter().rect_filled(icon, 6, p.track);
                                view::icon(
                                    ui,
                                    icon.center(),
                                    if queue.synchronize {
                                        "\u{e895}"
                                    } else {
                                        "layers"
                                    },
                                    17.0,
                                    p.blue,
                                );
                                let label = if queue.running {
                                    tr("Pause")
                                } else if queue.synchronize {
                                    tr("Check now")
                                } else {
                                    tr("Start")
                                };
                                // Measured widths keep the buttons ending at the card's
                                // edge, in reading order, however long the translations are.
                                let gap = 6.0;
                                let widths = [
                                    view::button_width(ui, label).max(76.0),
                                    view::button_width(ui, tr("Schedule")).max(72.0),
                                    view::button_width(ui, tr("View downloads")).max(106.0),
                                    view::button_width(ui, "⋯"),
                                ];
                                let total = widths.iter().sum::<f32>() + gap * 3.0;
                                // Beside the buttons, the name leaves room for its state and a gap.
                                let name_limit =
                                    header.width() - if narrow { 136.0 } else { total + 152.0 };
                                let name_width = ui
                                    .painter()
                                    .layout_no_wrap(
                                        display_queue_name(&queue.id, &queue.name).to_owned(),
                                        egui::FontId::proportional(13.0),
                                        p.text,
                                    )
                                    .size()
                                    .x
                                    .min(name_limit.max(100.0));
                                ui.place(
                                    Rect::from_min_size(
                                        header.min + vec2(46.0, 0.0),
                                        vec2(name_width, 21.0),
                                    ),
                                    egui::Label::new(
                                        RichText::new(display_queue_name(&queue.id, &queue.name))
                                            .strong(),
                                    )
                                    .truncate()
                                    .halign(Align::Min),
                                )
                                .on_hover_text(display_queue_name(&queue.id, &queue.name));
                                let scheduled = !queue.running
                                    && (queue.scheduled_at.is_some() || queue.automatic());
                                let state = if queue.running {
                                    tr("Running")
                                } else if scheduled {
                                    tr("Scheduled")
                                } else {
                                    tr("Paused")
                                };
                                let color = if queue.running {
                                    p.accent
                                } else if scheduled {
                                    p.blue
                                } else {
                                    p.muted
                                };
                                // 2px lower puts the 11px state on the 13px name's baseline.
                                ui.place(
                                    Rect::from_min_size(
                                        header.min + vec2(54.0 + name_width, 2.0),
                                        vec2(76.0, 21.0),
                                    ),
                                    egui::Label::new(
                                        RichText::new(format!("• {state}")).size(11.0).color(color),
                                    ),
                                );
                                let counts = match &summary {
                                    Some(Ok(value)) => tf!(
                                        "{} downloads · {} active · {} completed",
                                        value.files,
                                        value.active,
                                        value.complete
                                    ),
                                    Some(Err(_)) => tr("Cannot read downloads").into(),
                                    None => tr("Loading downloads…").into(),
                                };
                                let counts_response = ui
                                    .new_child(
                                        egui::UiBuilder::new()
                                            .max_rect(Rect::from_min_size(
                                                header.min + vec2(46.0, 22.0),
                                                vec2((header.width() - 46.0).min(340.0), 18.0),
                                            ))
                                            .layout(Layout::top_down(Align::Min)),
                                    )
                                    .add(
                                        egui::Label::new(RichText::new(counts).small().color(
                                            if matches!(summary, Some(Err(_))) {
                                                p.warning
                                            } else {
                                                p.muted
                                            },
                                        ))
                                        .truncate()
                                        .halign(Align::Min),
                                    );
                                if let Some(Err(error)) = &summary {
                                    counts_response.on_hover_text(error);
                                }
                                let action_y = if narrow {
                                    header.top() + 48.0
                                } else {
                                    header.top() + 7.0
                                };
                                {
                                    let mut actions = ui.new_child(
                                        egui::UiBuilder::new()
                                            .max_rect(Rect::from_min_size(
                                                pos2(header.right() - total, action_y),
                                                vec2(total, view::CONTROL_HEIGHT),
                                            ))
                                            .layout(Layout::left_to_right(Align::Center)),
                                    );
                                    let ui = &mut actions;
                                    ui.spacing_mut().item_spacing.x = gap;
                                    // The ⋯ glyph is a line shorter than text; keep every button the same height.
                                    ui.spacing_mut().interact_size.y = view::CONTROL_HEIGHT;
                                    let button = if queue.running {
                                        egui::Button::new(label)
                                    } else {
                                        primary(label, p)
                                    };
                                    if ui
                                        .add_sized([widths[0], view::CONTROL_HEIGHT], button)
                                        .clicked()
                                    {
                                        self.command(if queue.running {
                                            Command::PauseQueue(queue.id.clone())
                                        } else {
                                            Command::StartQueue(queue.id.clone())
                                        });
                                    }
                                    if ui
                                        .add_sized(
                                            [widths[1], view::CONTROL_HEIGHT],
                                            egui::Button::new(tr("Schedule")),
                                        )
                                        .clicked()
                                    {
                                        self.select_queue_for_dialog(&queue);
                                        self.open_queue_dialog(Dialog::Schedule);
                                    }
                                    if ui
                                        .add_sized(
                                            [widths[2], view::CONTROL_HEIGHT],
                                            egui::Button::new(tr("View downloads")),
                                        )
                                        .clicked()
                                    {
                                        self.queue_id.clone_from(&queue.id);
                                        self.view = View::Downloads;
                                        self.set_selection(None);
                                        self.force_refresh();
                                    }
                                    let _menu = egui::containers::menu::MenuButton::new("⋯")
                                        .config(theme::menu_config().close_behavior(
                                            egui::PopupCloseBehavior::CloseOnClickOutside,
                                        ))
                                        .ui(ui, |ui| {
                                            self.queue_card_menu(
                                                ui,
                                                &queue,
                                                summary
                                                    .as_ref()
                                                    .and_then(|value| value.as_ref().ok()),
                                            );
                                            #[cfg(feature = "screenshot")]
                                            shell::capture_menu_contents(ui, "Queue actions");
                                        })
                                        .0
                                        .on_hover_text(tr("More queue actions"));
                                    // Screenshots open the main queue's menu without a real click.
                                    #[cfg(feature = "screenshot")]
                                    if queue.id == MAIN_QUEUE {
                                        shell::capture_target(ui, "Queue actions", &_menu);
                                    }
                                }
                                if let Some(Ok(value)) = &summary {
                                    ui.add_space(8.0);
                                    let known = value.unknown == 0;
                                    let progress = if known && value.total > 0 {
                                        (value.downloaded as f64 / value.total as f64)
                                            .clamp(0.0, 1.0)
                                            as f32
                                    } else {
                                        0.0
                                    };
                                    ui.horizontal(|ui| {
                                        let label = if value.files == 0 {
                                            tr("No downloads yet").into()
                                        } else if known {
                                            tf!(
                                                "{} of {}",
                                                bytes(value.downloaded),
                                                bytes(value.total)
                                            )
                                        } else {
                                            tf!(
                                                "{} downloaded · {} file sizes unknown",
                                                bytes(value.downloaded),
                                                value.unknown
                                            )
                                        };
                                        ui.label(RichText::new(label).small().color(p.muted));
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(if known && value.total > 0 {
                                                        format!("{:.0}%", progress * 100.0)
                                                    } else {
                                                        "—".into()
                                                    })
                                                    .small()
                                                    .color(p.muted),
                                                );
                                            },
                                        );
                                    });
                                    let (track, _) = ui.allocate_exact_size(
                                        vec2(ui.available_width(), 4.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(track, 2, p.track);
                                    if progress > 0.0 {
                                        let mut fill = track;
                                        fill.max.x = fill.min.x + fill.width() * progress;
                                        ui.painter().rect_filled(
                                            fill,
                                            2,
                                            if queue.running { p.accent } else { p.muted },
                                        );
                                    }
                                }
                                ui.add_space(8.0);
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new(tf!("Up to {} at once", queue.max_active))
                                            .small()
                                            .color(p.muted),
                                    );
                                    ui.label(RichText::new("·").color(p.muted));
                                    let schedule = queue
                                        .recurring
                                        .as_ref()
                                        .map(display_schedule)
                                        .unwrap_or_else(|| {
                                            queue
                                                .scheduled_at
                                                .map(download_engine::queue::schedule_label)
                                                .unwrap_or_else(|| tr("No schedule").into())
                                        });
                                    ui.label(RichText::new(schedule).small().color(p.muted));
                                    if queue.synchronize {
                                        ui.label(
                                            RichText::new(
                                                "· Synchronization · previous copies kept",
                                            )
                                            .small()
                                            .color(p.muted),
                                        );
                                    }
                                });
                                if queue.has_schedule() {
                                    ui.label(
                                        RichText::new(
                                            queue.state_label(chrono::Utc::now().timestamp()),
                                        )
                                        .small()
                                        .color(p.blue),
                                    );
                                }
                                if let Some(label) = self.completion_label(&queue.id) {
                                    ui.label(RichText::new(label).small().color(p.muted));
                                }
                            });
                        ui.add_space(12.0);
                    });
                }
            });
    }

    fn select_queue_for_dialog(&mut self, queue: &DownloadQueue) {
        self.queue_id.clone_from(&queue.id);
        self.set_selection(None);
        self.force_refresh();
    }

    fn queue_card_menu(
        &mut self,
        ui: &mut egui::Ui,
        queue: &DownloadQueue,
        summary: Option<&Summary>,
    ) {
        ui.label(tr("Download at once"));
        let id = egui::Id::new(("queue-limit-input", &queue.id));
        let mut draft = ui
            .ctx()
            .data_mut(|data| data.get_temp::<(usize, String)>(id))
            .filter(|(saved, _)| *saved == queue.max_active)
            .unwrap_or_else(|| (queue.max_active, queue.max_active.to_string()));
        let mut apply = false;
        ui.horizontal(|ui| {
            let input = ui.add(
                view::line_edit(&mut draft.1)
                    .id(id.with("editor"))
                    .desired_width(55.0)
                    .char_limit(3),
            );
            let valid = draft
                .1
                .parse::<usize>()
                .is_ok_and(|limit| (1..=download_engine::queue::MAX_ACTIVE).contains(&limit));
            let clicked = ui
                .add_enabled(valid, egui::Button::new(tr("Set")))
                .clicked();
            apply = valid
                && ((input.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                    || clicked);
        });
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, draft.clone()));
        ui.label(
            RichText::new(format!(
                "1–{} files · {} total across queues",
                download_engine::queue::MAX_ACTIVE,
                download_engine::queue::MAX_ACTIVE
            ))
            .small(),
        );
        if apply {
            // The scheduler/store validates again, including calls outside the UI.
            self.command(Command::SetQueueLimit(
                queue.id.clone(),
                draft.1.parse().unwrap(),
            ));
            ui.ctx().data_mut(|data| data.remove::<(usize, String)>(id));
            ui.close();
        }
        ui.separator();
        self.completion_menu(ui, queue);
        if ui.button(tr("Rename queue")).clicked() {
            self.select_queue_for_dialog(queue);
            self.queue_name.clone_from(&queue.name);
            self.open_queue_dialog(Dialog::RenameQueue);
            ui.close();
        }
        if !queue.running
            && (queue.scheduled_at.is_some() || queue.automatic())
            && ui.button(tr("Pause queue")).clicked()
        {
            self.command(Command::PauseQueue(queue.id.clone()));
            ui.close();
        }
        if queue.synchronize && queue.running && ui.button(tr("Check now")).clicked() {
            self.command(Command::StartQueue(queue.id.clone()));
            ui.close();
        }
        if queue
            .recurring
            .as_ref()
            .is_some_and(|schedule| !schedule.enabled)
            && ui.button(tr("Enable schedule")).clicked()
        {
            self.command(Command::EnableSchedule(queue.id.clone()));
            ui.close();
        }
        if queue.has_schedule() && ui.button(tr("Clear schedule")).clicked() {
            self.command(Command::ClearSchedule(queue.id.clone()));
            ui.close();
        }
        if ui
            .add_enabled(
                summary.is_some_and(|value| value.complete > 0),
                egui::Button::new(tr("Clear finished")),
            )
            .clicked()
        {
            match self.store.queue_jobs(&queue.id) {
                Ok(jobs) => {
                    for job in jobs
                        .into_iter()
                        .filter(|job| job.status == Status::Complete)
                    {
                        self.command(Command::Remove(job.id));
                    }
                }
                Err(error) => self.notice = Some(Notice::error(error.to_string())),
            }
            ui.close();
        }
        if ui
            .add_enabled(
                queue.id != MAIN_QUEUE && summary.is_some_and(|value| value.files == 0),
                egui::Button::new(tr("Delete empty queue")),
            )
            .clicked()
        {
            match self.store.delete_queue(&queue.id) {
                Ok(()) => {
                    if self.queue_id == queue.id {
                        self.queue_id = MAIN_QUEUE.into();
                        self.set_selection(None);
                    }
                    self.force_refresh();
                }
                Err(error) => self.notice = Some(Notice::error(error.to_string())),
            }
            ui.close();
        }
    }
}
