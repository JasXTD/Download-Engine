use super::*;
use download_engine::grabber_schedule::{MAX_INTERVAL_HOURS, Summary};

pub(super) struct Form {
    pub date: String,
    pub time: String,
    pub repeat: bool,
    pub hours: u16,
    pub visible: bool,
    pub confirm_reset: bool,
}
impl Default for Form {
    fn default() -> Self {
        let next = chrono::Local::now() + chrono::Duration::minutes(5);
        Self {
            date: next.format("%Y-%m-%d").to_string(),
            time: next.format("%H:%M").to_string(),
            repeat: true,
            hours: 24,
            visible: false,
            confirm_reset: false,
        }
    }
}

fn caption(summary: &Summary) -> String {
    if summary.running {
        return tr("Scanning website…").into();
    }
    if summary.enabled {
        let next = summary
            .next_at
            .map(download_engine::queue::schedule_label)
            .unwrap_or_default();
        let repeat = summary
            .every_hours
            .map(|h| tf!("Every {h} hours", h = h))
            .unwrap_or_else(|| tr("One time").into());
        tf!(
            "{repeat} · Next: {next} · Local time",
            repeat = repeat,
            next = next
        )
    } else {
        tr("No upcoming scans").into()
    }
}

impl State {
    pub(super) fn load_schedule(&mut self, store: &Store) -> anyhow::Result<()> {
        if let Some(project) = &self.project
            && let Some(summary) = store
                .grabber_schedules()?
                .iter()
                .find(|s| s.project == project.id)
        {
            if let Some(at) = summary
                .next_at
                .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
            {
                let at = at.with_timezone(&chrono::Local);
                self.schedule.date = at.format("%Y-%m-%d").to_string();
                self.schedule.time = at.format("%H:%M").to_string();
            }
            self.schedule.repeat = summary.every_hours.is_some();
            self.schedule.hours = summary.every_hours.unwrap_or(24);
        }
        Ok(())
    }

    pub(super) fn load_pending(&mut self, store: &Store) -> anyhow::Result<()> {
        let project = self
            .project
            .as_ref()
            .context(tr("Choose a saved project first"))?;
        let files = store.grabber_discoveries(&project.id)?;
        self.receive(Report {
            files: files.iter().map(|d| d.file.clone()).collect(),
            ..Default::default()
        });
        self.pending = Some(files);
        self.schedule.visible = false;
        self.error = None;
        Ok(())
    }

    pub(super) fn selected_pending(&self) -> Vec<String> {
        self.pending
            .as_ref()
            .map(|files| {
                files
                    .iter()
                    .zip(&self.selected)
                    .filter(|(_, selected)| **selected)
                    .map(|(d, _)| d.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn schedule_controls(
        &mut self,
        ui: &mut egui::Ui,
        summary: Option<&Summary>,
        p: Palette,
    ) -> Option<ProjectAction> {
        self.project.as_ref()?;
        let mut action = None;
        ui.add_enabled_ui(self.task.is_none(), |ui| {
            ui.horizontal_wrapped(|ui| {
                if !self.schedule.visible && ui.button(tr("Schedule scans")).clicked() {
                    self.schedule.visible = true;
                }
                let count = summary.map(|s| s.pending).unwrap_or(0);
                if ui
                    .button(if self.pending.is_some() {
                        tr("Refresh preview").into()
                    } else {
                        tf!("Review new files ({count})", count = count)
                    })
                    .clicked()
                {
                    action = Some(ProjectAction::ReviewPending);
                }
                if let Some(summary) = summary {
                    ui.label(RichText::new(caption(summary)).small().color(p.accent));
                }
            });
            if self.schedule.visible {
                egui::Frame::new()
                    .fill(p.background)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 8.0;
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.schedule.repeat, false, tr("One time"));
                            ui.selectable_value(&mut self.schedule.repeat, true, tr("Repeating"));
                            if self.schedule.repeat {
                                ui.label(tr("Every"));
                                ui.add(
                                    egui::DragValue::new(&mut self.schedule.hours)
                                        .range(1..=MAX_INTERVAL_HOURS),
                                );
                                ui.label(tr("hours"));
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(tr("First scan date (YYYY-MM-DD)"));
                                ui.add(
                                    view::line_edit(&mut self.schedule.date)
                                        .desired_width(190.0)
                                        .char_limit(10),
                                );
                            });
                            ui.vertical(|ui| {
                                ui.label(tr("Local time (HH:MM)"));
                                ui.add(
                                    view::line_edit(&mut self.schedule.time)
                                        .desired_width(140.0)
                                        .char_limit(5),
                                );
                            });
                        });
                        ui.label(
                            RichText::new(
                                tr("Runs while Download Engine is open and keeps newly discovered links for your review."),
                            )
                            .small()
                            .color(p.muted),
                        );
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(tr("Save scan schedule")).clicked() {
                                action = Some(ProjectAction::SetSchedule);
                            }
                            if ui
                                .add_enabled(
                                    summary.is_some_and(|s| s.enabled || s.running),
                                    egui::Button::new(tr("Clear scan schedule")),
                                )
                                .clicked()
                            {
                                action = Some(ProjectAction::ClearSchedule);
                            }
                            if ui.button(tr("Reset discovery history")).clicked() {
                                self.schedule.confirm_reset = true;
                            }
                        });
                        if self.schedule.confirm_reset {
                            ui.label(
                                tr("Previously reviewed links will appear as new on a future scan."),
                            );
                            ui.horizontal(|ui| {
                                if ui.button(tr("Forget reviewed links")).clicked() {
                                    action = Some(ProjectAction::ResetHistory);
                                }
                                if ui.button(tr("Keep history")).clicked() {
                                    self.schedule.confirm_reset = false;
                                }
                            });
                        }
                        if let Some(summary) = summary
                            && !summary.message.is_empty()
                        {
                            ui.label(RichText::new(&summary.message).small().color(p.muted));
                        }
                        if let Some(notice) = &self.project_notice {
                            ui.label(RichText::new(notice).small().color(p.success));
                        }
                    });
            }
        });
        action
    }
}

impl Desktop {
    pub(super) fn open_scheduled_grabber(&mut self, id: &str, review: bool) {
        if self.dialog.is_some() {
            return;
        }
        self.open_grabber();
        if let Some(draft) = &mut self.grabber {
            let result = draft
                .project_action(&self.store, ProjectAction::Open(id.into()), &self.queue_id)
                .and_then(|()| {
                    if review {
                        draft.load_pending(&self.store)
                    } else {
                        draft.schedule.visible = true;
                        Ok(())
                    }
                });
            if let Err(error) = result {
                draft.error = Some(format!("{error:#}"));
            }
        }
    }

    pub(in crate::desktop) fn grabber_banner(&mut self, ui: &mut egui::Ui, p: Palette) {
        let count: usize = self.grabber_summaries.iter().map(|s| s.pending).sum();
        if count == 0 {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(tf!(
                    "{count} new website files awaiting review",
                    count = count
                ))
                .color(p.accent),
            );
            if ui.button(tr("Review website files")).clicked() {
                self.view = View::Scheduler;
            }
        });
        ui.add_space(6.0);
    }

    pub(in crate::desktop) fn grabber_schedule_cards(&mut self, ui: &mut egui::Ui, p: Palette) {
        ui.heading(tr("Website scans"));
        ui.label(RichText::new(tr("Scheduled projects collect new links for review. Download queues start only after you add files.")).small().color(p.muted));
        if self.grabber_summaries.is_empty() && ui.button(tr("Create grabber project")).clicked() {
            self.open_grabber();
        }
        for summary in self.grabber_summaries.clone() {
            ui.push_id((&summary.project, "scheduled-grabber"), |ui| {
                egui::Frame::group(ui.style())
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.strong(&summary.name);
                        ui.label(RichText::new(caption(&summary)).color(p.accent));
                        if let Some(at) = summary.last_at {
                            ui.label(
                                RichText::new(tf!(
                                    "Last scan: {} · {}",
                                    download_engine::queue::schedule_label(at),
                                    summary.message
                                ))
                                .small()
                                .color(p.muted),
                            );
                        }
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .button(tf!("Review new files ({})", summary.pending))
                                .clicked()
                            {
                                self.open_scheduled_grabber(&summary.project, true);
                            }
                            if ui.button(tr("Manage scan schedule")).clicked() {
                                self.open_scheduled_grabber(&summary.project, false);
                            }
                        });
                    });
            });
            ui.add_space(10.0);
        }
        ui.separator();
        ui.heading(tr("Download queues"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};
    use rusqlite::params;
    use sha2::{Digest, Sha256};

    fn project(app: &mut Desktop) -> Project {
        let now = chrono::Utc::now().timestamp();
        app.store
            .schedule_queue(MAIN_QUEUE, now + 3600, now)
            .unwrap();
        let project = app
            .store
            .create_grabber_project(
                "Scheduled manuals",
                &Settings {
                    url: "https://example.test/".into(),
                    extensions: "pdf".into(),
                    depth: 1,
                    page_limit: 20,
                    queue: MAIN_QUEUE.into(),
                    destination: Destination::Documents,
                },
            )
            .unwrap();
        app.refresh_and_wait();
        project
    }

    fn seed(app: &Desktop, project: &Project) {
        let conn =
            crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3"))
                .unwrap();
        for name in ["a.pdf", "b.pdf"] {
            let url = format!("https://example.test/{name}");
            let hash = Sha256::digest(url.as_bytes());
            let file = website_grabber::Candidate {
                url,
                name: name.into(),
                filename: None,
                page: project.settings.url.clone(),
            };
            conn.execute(
                "INSERT INTO grabber_pending VALUES(?1,?2,?3,?4,?5)",
                params![
                    Uuid::new_v4().to_string(),
                    project.id,
                    &hash[..],
                    serde_json::to_string(&file).unwrap(),
                    100
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO grabber_seen VALUES(?1,?2)",
                params![project.id, &hash[..]],
            )
            .unwrap();
        }
    }

    #[test]
    fn schedule_controls_validate_save_reopen_and_clear_without_creating_jobs() {
        let (_temp, mut app, ctx) = fixture();
        let project = project(&mut app);
        app.view = View::Scheduler;
        click(&mut app, &ctx, "Manage scan schedule");
        let draft = app.grabber.as_mut().unwrap();
        assert_eq!(draft.project.as_ref().unwrap().id, project.id);
        draft.schedule.date = "invalid".into();
        click(&mut app, &ctx, "Save scan schedule");
        assert!(app.grabber.as_ref().unwrap().error.is_some());
        let next = chrono::Local::now() + chrono::Duration::days(1);
        let draft = app.grabber.as_mut().unwrap();
        draft.schedule.date = next.format("%Y-%m-%d").to_string();
        draft.schedule.time = "14:30".into();
        draft.schedule.hours = 12;
        draft.schedule.repeat = true;
        click(&mut app, &ctx, "Save scan schedule");
        assert!(app.grabber.as_ref().unwrap().error.is_none());
        assert_eq!(
            app.store.grabber_schedules().unwrap()[0].every_hours,
            Some(12)
        );
        click(&mut app, &ctx, "×");
        click(&mut app, &ctx, "Manage scan schedule");
        assert_eq!(app.grabber.as_ref().unwrap().schedule.time, "14:30");
        assert_eq!(app.grabber.as_ref().unwrap().schedule.hours, 12);
        click(&mut app, &ctx, "Clear scan schedule");
        assert!(!app.store.grabber_schedules().unwrap()[0].enabled);
        assert!(app.store.list().unwrap().is_empty());
    }

    #[test]
    fn pending_preview_has_no_preselection_close_keeps_files_and_review_queues_only_selection() {
        let (_temp, mut app, ctx) = fixture();
        let project = project(&mut app);
        seed(&app, &project);
        app.refresh_and_wait();
        click(&mut app, &ctx, "Review website files");
        assert_eq!(app.view, View::Scheduler);
        click(&mut app, &ctx, "Review new files (2)");
        assert_eq!(app.grabber.as_ref().unwrap().selected, [false, false]);
        assert!(app.store.list().unwrap().is_empty());
        app.grabber.as_mut().unwrap().selected[0] = true;
        click(&mut app, &ctx, "×");
        click(&mut app, &ctx, "Review new files (2)");
        assert_eq!(app.grabber.as_ref().unwrap().selected, [false, false]);
        app.grabber.as_mut().unwrap().selected[0] = true;
        click(&mut app, &ctx, "Add selected to queue");
        assert!(app.dialog.is_none());
        assert_eq!(app.store.list().unwrap().len(), 1);
        assert_eq!(app.store.list().unwrap()[0].name, "a.pdf");
        assert_eq!(app.store.grabber_discoveries(&project.id).unwrap().len(), 1);
        app.view = View::Scheduler;
        click(&mut app, &ctx, "Review new files (1)");
        click(&mut app, &ctx, "Select visible");
        click(&mut app, &ctx, "Ignore selected");
        assert!(
            app.store
                .grabber_discoveries(&project.id)
                .unwrap()
                .is_empty()
        );
        assert!(app.grabber.as_ref().unwrap().selected.is_empty());
        assert_eq!(app.store.list().unwrap().len(), 1);
    }

    #[test]
    fn schedule_and_persistent_preview_actions_fit_minimum_window_in_both_themes() {
        let (_temp, mut app, ctx) = fixture();
        let project = project(&mut app);
        seed(&app, &project);
        app.refresh_and_wait();
        app.open_scheduled_grabber(&project.id, false);
        for dark in [false, true] {
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            for preview in [false, true] {
                if preview {
                    app.grabber
                        .as_mut()
                        .unwrap()
                        .load_pending(&app.store)
                        .unwrap();
                } else {
                    let draft = app.grabber.as_mut().unwrap();
                    draft.report = None;
                    draft.pending = None;
                    draft.schedule.visible = true;
                }
                let size = egui::vec2(860.0, 600.0);
                for _ in 0..3 {
                    frame(&mut app, &ctx, size, vec![]);
                }
                let output = frame(&mut app, &ctx, size, vec![]);
                let labels: &[&str] = if preview {
                    &[
                        "Add selected to queue",
                        "Ignore selected",
                        "Project settings",
                        "Documents",
                        "×",
                    ]
                } else {
                    &[
                        "Save scan schedule",
                        "Clear scan schedule",
                        "Back to project",
                        "×",
                    ]
                };
                for label in labels {
                    let rect = text_rect(&output, label);
                    assert!(
                        egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                        "{label}: {rect:?}"
                    );
                    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,egui::epaint::Shape::Text(text)
                        if text.galley.job.text.trim()==*label && shape.clip_rect.contains_rect(egui::Rect::from_min_size(text.pos,text.galley.size())))),"{label} clipped");
                }
            }
        }
    }
}
