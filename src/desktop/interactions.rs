//! Shared feedback, list selection, and destructive-action confirmation.
use super::*;
use anyhow::Context;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NoticeKind {
    Info,
    Success,
    Error,
}

#[derive(Clone, Debug)]
pub(super) struct Notice {
    pub message: String,
    pub kind: NoticeKind,
}
impl Notice {
    pub fn info(message: String) -> Self {
        Self {
            message,
            kind: NoticeKind::Info,
        }
    }
    pub fn success(message: String) -> Self {
        Self {
            message,
            kind: NoticeKind::Success,
        }
    }
    pub fn error(message: String) -> Self {
        download_engine::diagnostics::message(
            download_engine::diagnostics::Event::UiError,
            &message,
        );
        Self {
            message,
            kind: NoticeKind::Error,
        }
    }
    pub fn outcome(warning: Option<String>, success: String) -> Self {
        warning
            .map(Self::error)
            .unwrap_or_else(|| Self::success(success))
    }
    pub fn color(&self, p: Palette) -> Color32 {
        match self.kind {
            NoticeKind::Info => p.blue,
            NoticeKind::Success => p.success,
            NoticeKind::Error => p.warning,
        }
    }
}
impl std::ops::Deref for Notice {
    type Target = str;
    fn deref(&self) -> &str {
        &self.message
    }
}

#[derive(Default)]
pub(super) struct Selection {
    pub additional: HashSet<String>,
    pub anchor: Option<String>,
    pub empty_by_choice: bool,
    pub scroll_to_focus: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DestructiveAction {
    Remove,
    ClearHistory,
    Restart,
    DeleteFile,
}
impl DestructiveAction {
    fn label(self) -> &'static str {
        match self {
            Self::Remove => tr("Remove"),
            Self::ClearHistory => tr("Clear history"),
            Self::Restart => tr("Restart"),
            Self::DeleteFile => tr("Delete"),
        }
    }
    fn command(self, job: Job, history_queue: Option<&str>) -> anyhow::Result<Command> {
        Ok(match self {
            Self::Remove => Command::Remove(job.id),
            Self::ClearHistory => Command::ClearHistory {
                id: job.id,
                queue: history_queue.context("Missing history queue")?.into(),
            },
            Self::Restart => Command::Restart(job.id),
            Self::DeleteFile => Command::DeleteFile {
                id: job.id,
                path: job
                    .saved_path
                    .context(tr("The saved file location is unavailable"))?,
            },
        })
    }
}
pub(super) struct Confirmation {
    action: DestructiveAction,
    jobs: Vec<Job>,
    history_queue: Option<String>,
}

impl Desktop {
    pub(super) fn set_selection(&mut self, id: Option<String>) {
        self.selection = Selection {
            anchor: id.clone(),
            ..Default::default()
        };
        self.selected = id;
    }
    pub(super) fn is_selected(&self, id: &str) -> bool {
        self.selected.as_deref() == Some(id) || self.selection.additional.contains(id)
    }
    pub(super) fn selected_jobs(&self) -> Vec<Job> {
        self.selected_job_refs().cloned().collect()
    }
    pub(super) fn selected_job_refs(&self) -> impl Iterator<Item = &Job> {
        let search = self.search.trim().to_lowercase();
        self.jobs
            .iter()
            .filter(move |j| self.is_selected(&j.id) && self.accepts_with_search(j, &search))
    }
    pub(super) fn visible_job_indices(&self) -> Vec<usize> {
        let search = self.search.trim().to_lowercase();
        self.jobs
            .iter()
            .enumerate()
            .filter(|(_, job)| self.accepts_with_search(job, &search))
            .map(|(index, _)| index)
            .collect()
    }
    pub(super) fn visible_job_ids(&self) -> Vec<String> {
        self.visible_job_indices()
            .into_iter()
            .map(|index| self.jobs[index].id.clone())
            .collect()
    }
    pub(super) fn choose_download(&mut self, id: &str, modifiers: egui::Modifiers) {
        let visible = self.visible_job_ids();
        let Some(index) = visible.iter().position(|i| i == id) else {
            return;
        };
        if modifiers.shift {
            let anchor = self.selection.anchor.as_ref().or(self.selected.as_ref());
            let start = anchor
                .and_then(|a| visible.iter().position(|i| i == a))
                .unwrap_or(index);
            let anchor = visible[start].clone();
            if !modifiers.command && !modifiers.ctrl {
                self.selection.additional.clear();
            } else if let Some(selected) = &self.selected {
                self.selection.additional.insert(selected.clone());
            }
            self.selection
                .additional
                .extend(visible[start.min(index)..=start.max(index)].iter().cloned());
            self.selection.additional.remove(id);
            self.selected = Some(id.into());
            self.selection.anchor = Some(anchor);
        } else if modifiers.command || modifiers.ctrl {
            if self.selected.as_deref() == Some(id) {
                self.selected = visible
                    .iter()
                    .find(|i| self.selection.additional.contains(*i))
                    .cloned();
                if let Some(selected) = &self.selected {
                    self.selection.additional.remove(selected);
                }
            } else if !self.selection.additional.remove(id)
                && let Some(previous) = self.selected.replace(id.into())
            {
                self.selection.additional.insert(previous);
            }
            self.selection.anchor = Some(id.into());
        } else {
            self.set_selection(Some(id.into()));
        }
        self.selection.empty_by_choice = self.selected.is_none();
    }
    pub(super) fn list_keyboard(&mut self, ctx: &egui::Context) {
        let list = egui::Id::new("download-list");
        if self.view != View::Downloads || !ctx.memory(|m| m.has_focus(list)) {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
            let ids = self.visible_job_ids();
            self.set_selection(ids.first().cloned());
            self.selection.additional.extend(ids.into_iter().skip(1));
        }
        let modifiers = ctx.input(|i| i.modifiers);
        for key in [
            egui::Key::ArrowUp,
            egui::Key::ArrowDown,
            egui::Key::Home,
            egui::Key::End,
        ] {
            if ctx.input_mut(|i| i.consume_key(modifiers, key)) {
                let ids = self.visible_job_ids();
                if ids.is_empty() {
                    return;
                }
                let current = self
                    .selected
                    .as_ref()
                    .and_then(|s| ids.iter().position(|i| i == s));
                let index = match key {
                    egui::Key::Home => 0,
                    egui::Key::End => ids.len() - 1,
                    egui::Key::ArrowUp => current.unwrap_or(1).saturating_sub(1),
                    _ => current.map_or(0, |i| (i + 1).min(ids.len() - 1)),
                };
                self.choose_download(
                    &ids[index],
                    egui::Modifiers {
                        shift: modifiers.shift,
                        ..Default::default()
                    },
                );
                self.selection.scroll_to_focus = true;
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
            self.request_destructive(DestructiveAction::Remove, self.selected_jobs());
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
            // Space controls the existing selection; Enter opens its status.
            let pause = self
                .selected_jobs()
                .iter()
                .any(|j| matches!(j.status, Status::Downloading | Status::Queued));
            self.selection_command(pause);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            && let Some(job) = self.selected_jobs().into_iter().next()
        {
            self.open_download_status(&job);
        }
    }
    pub(super) fn selection_command(&mut self, pause: bool) {
        let jobs = self.selected_jobs();
        let single = jobs.len() == 1;
        for job in jobs {
            if pause && matches!(job.status, Status::Downloading | Status::Queued) {
                self.command(Command::Pause(job.id));
            } else if !pause && matches!(job.status, Status::Paused | Status::Failed) {
                if single {
                    self.expect_download_status(&job.id);
                }
                self.command(Command::Resume(job.id));
            }
        }
    }
    fn has_partial_data(&self, job: &Job) -> bool {
        if job.status == Status::Complete {
            return false;
        }
        if job.downloaded > 0 {
            return true;
        }
        let Ok(parts) = self.store.paths.parts(&job.id) else {
            return true;
        };
        let has_parts = match std::fs::read_dir(parts) {
            Ok(mut entries) => entries.any(|entry| {
                entry
                    .and_then(|e| e.metadata())
                    .map_or(true, |m| m.len() > 0)
            }),
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        };
        let staging = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
        has_parts
            || match std::fs::symlink_metadata(staging) {
                Ok(metadata) => metadata.len() > 0,
                Err(error) => error.kind() != std::io::ErrorKind::NotFound,
            }
    }
    fn current_action_jobs(
        &self,
        action: DestructiveAction,
        jobs: &[Job],
    ) -> anyhow::Result<Vec<Job>> {
        let mut current = Vec::new();
        for snapshot in jobs {
            if let Some(job) = self.store.get(&snapshot.id)? {
                if action == DestructiveAction::ClearHistory && job.status != Status::Complete {
                    continue;
                }
                if action == DestructiveAction::DeleteFile {
                    anyhow::ensure!(
                        job.status == Status::Complete,
                        tr("Only completed files can be deleted.")
                    );
                    anyhow::ensure!(
                        job.saved_path.is_some(),
                        tr(
                            "The saved file location is unavailable. Use Clear from history to remove this entry."
                        )
                    );
                    anyhow::ensure!(
                        job.saved_path == snapshot.saved_path,
                        tr(
                            "The saved file location changed. Close this dialog and choose Delete file again."
                        )
                    );
                }
                anyhow::ensure!(
                    job.status != Status::Downloading,
                    "Pause the selected downloads before {}.",
                    if action == DestructiveAction::Remove {
                        tr("removing them")
                    } else {
                        tr("restarting them")
                    }
                );
                anyhow::ensure!(
                    action != DestructiveAction::Restart || job.status != Status::Complete,
                    tr("This download has finished. Use Redownload to keep the saved file.")
                );
                current.push(job);
            } else if action == DestructiveAction::DeleteFile {
                anyhow::bail!(tr(
                    "This download was removed. Close this dialog and refresh the list."
                ));
            }
        }
        Ok(current)
    }
    pub(super) fn request_destructive(&mut self, action: DestructiveAction, jobs: Vec<Job>) {
        if self.dialog.is_some() {
            return;
        }
        match self.current_action_jobs(action, &jobs) {
            Ok(jobs)
                if !jobs.is_empty()
                    && (action == DestructiveAction::DeleteFile
                        || jobs.iter().any(|j| self.has_partial_data(j))) =>
            {
                self.confirmation = Some(Confirmation {
                    action,
                    jobs,
                    history_queue: None,
                });
                self.form_error = None;
                self.dialog = Some(Dialog::Confirm);
            }
            Ok(jobs) => self.dispatch_destructive(action, jobs, None),
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
    }
    pub(super) fn request_clear_history(&mut self, queue: &str) {
        if self.dialog.is_some() {
            return;
        }
        match self.store.queue_jobs(queue) {
            Ok(jobs) => {
                let jobs: Vec<_> = jobs
                    .into_iter()
                    .filter(|job| job.status == Status::Complete)
                    .collect();
                if !jobs.is_empty() {
                    self.confirmation = Some(Confirmation {
                        action: DestructiveAction::ClearHistory,
                        jobs,
                        history_queue: Some(queue.into()),
                    });
                    self.form_error = None;
                    self.dialog = Some(Dialog::Confirm);
                }
            }
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
    }
    fn dispatch_destructive(
        &mut self,
        action: DestructiveAction,
        jobs: Vec<Job>,
        history_queue: Option<&str>,
    ) {
        for job in jobs {
            if let Err(error) = action.command(job, history_queue).and_then(|command| {
                self.cancel_status_request(&command);
                self.scheduler.send(command)
            }) {
                self.notice = Some(Notice::error(error.to_string()));
                break;
            }
        }
    }
    pub(super) fn confirmation_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(form) = &self.confirmation else {
            self.dialog = None;
            return;
        };
        let action = form.action;
        let clearing = action == DestructiveAction::ClearHistory;
        let mut confirm = false;
        let mut cancel = false;
        let response = form_dialog::modal("confirm-download-action", p).show(ctx, |ui| {
            ui.set_width(430.0);
            let deleting = action == DestructiveAction::DeleteFile;
            let title = if clearing {
                tr("Clear completed history?").into()
            } else if deleting {
                if form.jobs.len() == 1 {
                    tr("Delete file?").into()
                } else {
                    tr("Delete files?").into()
                }
            } else {
                tf!("{} downloads?", tr(action.label()))
            };
            cancel = form_dialog::title(ui, &title, form_dialog::Icon::Symbol("\u{e7ba}"), p, true);
            ui.add_space(8.0);
            if clearing {
                let queue_id = form.history_queue.as_deref().unwrap_or("");
                let name = self
                    .queues
                    .iter()
                    .find(|queue| queue.id == queue_id)
                    .map_or(queue_id, |queue| queue.name.as_str());
                ui.label(
                    RichText::new(tr("Remove these entries from the download list?"))
                        .color(p.muted),
                );
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(p.background)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.columns(2, |columns| {
                            columns[0].label(RichText::new(tr("Queue")).size(11.0).color(p.muted));
                            columns[0].add_space(4.0);
                            columns[0]
                                .add(egui::Label::new(RichText::new(name).size(20.0)).truncate())
                                .on_hover_text(name);
                            columns[1].with_layout(
                                egui::Layout::top_down(egui::Align::Max),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(tr("Completed downloads"))
                                                .size(11.0)
                                                .color(p.muted),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(tr("Completed downloads"));
                                    ui.add_space(4.0);
                                    ui.label(
                                        RichText::new(form.jobs.len().to_string())
                                            .size(20.0)
                                            .strong(),
                                    );
                                },
                            );
                        });
                    });
            } else if form.jobs.len() == 1 {
                ui.add(egui::Label::new(&form.jobs[0].name).truncate())
                    .on_hover_text(&form.jobs[0].name);
            } else {
                ui.label(tf!("{} selected downloads", form.jobs.len()));
            }
            if !clearing && deleting {
                egui::ScrollArea::vertical()
                    .max_height(120.0)
                    .show(ui, |ui| {
                        for job in &form.jobs {
                            if let Some(path) = &job.saved_path {
                                ui.add(egui::Label::new(path.to_string_lossy()).wrap());
                            }
                        }
                    });
                ui.add_space(8.0);
                ui.label(tr(
                    "Permanently delete the saved files and remove them from history?",
                ));
                ui.label(tr(
                    "Files will not go to the Recycle Bin. This cannot be undone.",
                ));
            } else if !clearing {
                ui.label(tr(
                    "Saved progress will be discarded. This cannot be undone.",
                ));
                ui.label(if action == DestructiveAction::Remove {
                    tr("Completed files stay in their folders.")
                } else {
                    tr("The selected downloads will start again from the beginning.")
                });
            }
            if let Some(error) = &self.form_error {
                ui.label(RichText::new(error).color(p.warning));
            }
            form_dialog::controls(ui, |ui| {
                let label = if clearing {
                    tr("Clear history").into()
                } else if deleting {
                    if form.jobs.len() == 1 {
                        tr("Delete file").into()
                    } else {
                        tr("Delete files").into()
                    }
                } else {
                    tf!("{} and discard progress", tr(action.label()))
                };
                confirm = form_dialog::danger_button(ui, &label, p).clicked();
                if clearing {
                    cancel |= form_dialog::secondary_button(ui, tr("Cancel")).clicked();
                }
            });
        });
        if cancel || response.should_close() {
            self.confirmation = None;
            self.dialog = None;
            self.form_error = None;
        } else if confirm {
            // Re-read before dispatch: refreshes and worker events can arrive while the modal is open.
            match self.current_action_jobs(action, &form.jobs) {
                Ok(jobs) => {
                    let history_queue = form.history_queue.clone();
                    self.confirmation = None;
                    self.dialog = None;
                    self.form_error = None;
                    self.dispatch_destructive(action, jobs, history_queue.as_deref());
                }
                Err(error) => self.form_error = Some(error.to_string()),
            }
        }
    }
}
