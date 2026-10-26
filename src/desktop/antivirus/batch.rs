//! Single and batch modes share one dialog and at most one queued scan request.
//! Full scan reports remain in SQLite and are released from this UI on close.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::desktop) enum ScanMode {
    Single,
    Batch,
}

enum ResultState {
    Ready,
    Waiting,
    Done(Verdict, String),
    Skipped(String),
}
struct Entry {
    id: String,
    name: String,
    path: Option<PathBuf>,
    hash: Option<String>,
    selected: bool,
    result: ResultState,
}
pub(super) struct Batch {
    pub(super) id: Uuid,
    mode: ScanMode,
    entries: Vec<Entry>,
    settings: Option<Settings>,
    error: Option<String>,
    current: Option<String>,
    external_pending: Option<String>,
    started: bool,
    running: bool,
}
impl Batch {
    fn busy(&self) -> bool {
        self.running || self.current.is_some()
    }

    fn set_mode(&mut self, mode: ScanMode) {
        if self.busy() || self.mode == mode {
            return;
        }
        self.mode = mode;
        if self.started {
            self.reset();
        }
        if mode == ScanMode::Single {
            // Keep at most one selection when leaving the multi-file picker.
            let mut selected = false;
            for entry in &mut self.entries {
                entry.selected &= !selected;
                selected |= entry.selected;
            }
        }
    }

    fn reset(&mut self) {
        self.started = false;
        self.error = None;
        for entry in &mut self.entries {
            entry.result = if entry.path.is_some() {
                ResultState::Ready
            } else {
                ResultState::Skipped(tr("Saved file location unavailable").into())
            };
        }
    }

    pub(super) fn completed(&mut self, id: &str, result: &anyhow::Result<av::Outcome>) {
        if self.current.as_deref() != Some(id) {
            return;
        }
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            let (verdict, message) = match result {
                Ok(outcome) => (outcome.report.verdict, outcome.report.message.clone()),
                Err(error) => (Verdict::Failed, error.to_string()),
            };
            entry.result = ResultState::Done(verdict, message.chars().take(512).collect());
        }
        self.current = None;
        // Explorer only acknowledges dispatch. The antivirus can still reject
        // it as busy, so neither success nor failure authorizes another scan.
        if self.running
            && self
                .settings
                .as_ref()
                .is_some_and(|s| s.provider.external_review())
        {
            self.external_pending = Some(id.to_owned());
        }
    }
    fn counts(&self) -> (usize, usize) {
        let selected = self.entries.iter().filter(|entry| entry.selected);
        let total = selected.clone().count();
        let done = selected
            .filter(|entry| {
                matches!(
                    entry.result,
                    ResultState::Done(..) | ResultState::Skipped(_)
                )
            })
            .count();
        (done, total)
    }
}

impl Desktop {
    #[cfg(feature = "screenshot")]
    pub(in crate::desktop) fn antivirus_batch_preview(&mut self, mode: &str) {
        let review = mode.starts_with("scan-batch-review");
        if review {
            self.store
                .save_antivirus_settings(&Settings {
                    provider: if mode.ends_with("-kaspersky") {
                        Provider::Kaspersky
                    } else if mode.ends_with("-bitdefender") {
                        Provider::Bitdefender
                    } else {
                        Provider::Malwarebytes
                    },
                    ..Settings::default()
                })
                .expect("Cannot save isolated preview settings");
        }
        self.open_antivirus_scan(ScanMode::Batch);
        if let Some(batch) = &mut self.antivirus.batch {
            batch.started = true;
            batch.running = review;
            for (index, entry) in batch.entries.iter_mut().enumerate() {
                entry.selected = true;
                if review {
                    entry.result = if index == 0 || mode.ends_with("-last") {
                        ResultState::Done(
                            Verdict::ScanRequested,
                            tr("Illustrative request for UI preview only").into(),
                        )
                    } else {
                        ResultState::Waiting
                    };
                    if index == 0 {
                        batch.external_pending = Some(entry.id.clone());
                    }
                    continue;
                }
                entry.result = ResultState::Done(
                    if index == 0 {
                        Verdict::NoThreatsFound
                    } else {
                        Verdict::Failed
                    },
                    tr("Illustrative result for UI preview only").into(),
                );
            }
        }
    }
    pub(in crate::desktop) fn open_antivirus_scan(&mut self, mode: ScanMode) {
        if self.dialog.is_some() {
            self.notice = Some(Notice::info(
                tr("Finish the current dialog before scanning this file.").into(),
            ));
            return;
        }
        let jobs = match self.store.list() {
            Ok(jobs) => jobs,
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot load downloads: {error}",
                    error = error
                )));
                return;
            }
        };
        let (settings, error) = match self.store.antivirus_settings() {
            Ok(settings) => (Some(settings), None),
            Err(error) => (None, Some(error.to_string())),
        };
        self.antivirus.batch = Some(Batch {
            id: Uuid::new_v4(),
            mode,
            entries: jobs
                .into_iter()
                .filter(|job| job.status == Status::Complete)
                .map(|job| Entry {
                    selected: mode == ScanMode::Single
                        && job.saved_path.is_some()
                        && self.selected.as_deref() == Some(&job.id),
                    id: job.id,
                    name: job.name,
                    result: if job.saved_path.is_some() {
                        ResultState::Ready
                    } else {
                        ResultState::Skipped(tr("Saved file location unavailable").into())
                    },
                    path: job.saved_path,
                    hash: job.integrity.actual,
                })
                .collect(),
            settings,
            error,
            current: None,
            external_pending: None,
            started: false,
            running: false,
        });
        self.dialog = Some(Dialog::AntivirusScan);
    }

    fn start_antivirus_batch(&mut self) {
        let Some(batch) = &mut self.antivirus.batch else {
            return;
        };
        let (_, total) = batch.counts();
        if batch.started || total == 0 || (batch.mode == ScanMode::Single && total != 1) {
            return;
        }
        let settings = self.store.antivirus_settings().and_then(|settings| {
            settings.validate()?;
            Ok(settings)
        });
        match settings {
            Ok(settings) => batch.settings = Some(settings),
            Err(error) => {
                batch.error = Some(error.to_string());
                return;
            }
        }
        batch.error = None;
        batch.started = true;
        batch.running = true;
        for entry in &mut batch.entries {
            if entry.selected {
                entry.result = ResultState::Waiting;
            }
        }
        self.advance_antivirus_batch();
    }

    fn stop_antivirus_batch(&mut self) {
        let Some(batch) = &mut self.antivirus.batch else {
            return;
        };
        batch.running = false;
        batch.external_pending = None;
        self.antivirus.service.cancel_batch(batch.id);
        let stopped = if batch.mode == ScanMode::Single {
            tr("Cancelled")
        } else {
            tr("Not scanned: batch stopped")
        };
        for entry in &mut batch.entries {
            if matches!(entry.result, ResultState::Waiting)
                && batch.current.as_deref() != Some(&entry.id)
            {
                entry.result = ResultState::Done(Verdict::Cancelled, stopped.into());
            }
        }
        // A queued request disappears immediately; an active scanner reports
        // cancellation only after its process and file handles have closed.
        self.advance_antivirus_batch();
    }

    fn continue_external_batch(&mut self, retry: bool) {
        let Some(batch) = &mut self.antivirus.batch else {
            return;
        };
        if !batch.running || batch.current.is_some() {
            return;
        }
        let Some(id) = batch.external_pending.take() else {
            return;
        };
        if retry && let Some(entry) = batch.entries.iter_mut().find(|entry| entry.id == id) {
            entry.result = ResultState::Waiting;
        }
        self.advance_antivirus_batch();
    }

    pub(super) fn advance_antivirus_batch(&mut self) {
        let Some(batch) = &mut self.antivirus.batch else {
            return;
        };
        if batch.current.is_some() && !self.antivirus.service.batch_pending(batch.id) {
            let id = batch.current.take().unwrap();
            if let Some(entry) = batch.entries.iter_mut().find(|entry| entry.id == id) {
                entry.result = ResultState::Skipped(if batch.running {
                    tr("Scan request replaced or cancelled").into()
                } else if batch.mode == ScanMode::Single {
                    tr("Cancelled").into()
                } else {
                    tr("Not scanned: batch stopped").into()
                });
            }
        }
        if !batch.running || batch.current.is_some() || batch.external_pending.is_some() {
            return;
        }
        let settings = batch.settings.as_ref().expect("started batch settings");
        // Do not keep scanning with stale configuration after an external edit.
        if !self
            .store
            .antivirus_settings()
            .is_ok_and(|current| &current == settings)
        {
            batch.error = Some(
                tr("Scan settings changed. Start a new scan with the current settings.").into(),
            );
            self.stop_antivirus_batch();
            return;
        }
        for entry in &mut batch.entries {
            if !matches!(entry.result, ResultState::Waiting) {
                continue;
            }
            let job = match self.store.get(&entry.id) {
                Ok(Some(job))
                    if job.status == Status::Complete
                        && job.saved_path == entry.path
                        && job.integrity.actual == entry.hash =>
                {
                    job
                }
                Ok(_) => {
                    entry.result =
                        ResultState::Skipped(tr("Download removed, restarted or changed").into());
                    continue;
                }
                Err(error) => {
                    entry.result = ResultState::Skipped(error.to_string());
                    continue;
                }
            };
            if self
                .antivirus
                .service
                .enqueue_batch(job, settings.clone(), batch.id)
            {
                batch.current = Some(entry.id.clone());
                return;
            }
            entry.result = ResultState::Skipped(tr("Already queued for another scan").into());
        }
        batch.running = false;
    }

    pub(in crate::desktop) fn antivirus_scan_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(batch) = &mut self.antivirus.batch else {
            self.dialog = None;
            return;
        };
        let mut close = false;
        let mut start = false;
        let mut stop = false;
        let mut settings = false;
        let mut again = false;
        let mut external_continue = None;
        // Keep one area for both tabs so changing modes cannot fade the backdrop.
        let response = form_dialog::modal("antivirus-scan", p).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 80.0).clamp(280.0, 600.0));
            close = form_dialog::title(ui, tr("Antivirus Scan"), form_dialog::Icon::Symbol("\u{ea18}"), p, true);
            ui.add_enabled_ui(!batch.busy(), |ui| {
                ui.horizontal(|ui| {
                    for (mode, label) in [(ScanMode::Single, tr("Single")), (ScanMode::Batch, tr("Batch"))] {
                        if ui.selectable_label(batch.mode == mode, label).clicked() {
                            batch.set_mode(mode);
                        }
                    }
                });
            });
            let single = batch.mode == ScanMode::Single;
            ui.label(RichText::new(if single { tr("Select one completed download to scan.") } else { tr("Select completed downloads from any queue. Files are scanned one at a time.") }).small().color(p.muted));
            if let Some(settings) = &batch.settings {
                ui.label(tf!("Scanner: {}", tr(settings.provider.label())));
                if settings.provider.external_review() {
                    ui.label(RichText::new(if single { tr("Review the result in the selected antivirus app.") } else { tr("This batch waits for your review after each request. Stop batch cancels remaining requests; scans already handed off continue in the antivirus app.") }).small().color(p.muted));
                }
            }
            let (done, total) = batch.counts();
            if batch.started {
                let label = if batch.external_pending.is_some() {
                    tf!("Waiting for your review · {done} of {total} processed", done = done, total = total)
                } else if batch.running {
                    tf!("Scanning {} of {total}", (done + 1).min(total), total = total)
                } else if batch.current.is_some() {
                    tf!("Stopping… {done} of {total} finished", done = done, total = total)
                } else if single {
                    tr("Scan finished").into()
                } else {
                    tf!("Batch finished · {done} of {total} processed", done = done, total = total)
                };
                ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).text(label));
            } else if !single {
                ui.horizontal(|ui| {
                    if ui.button(tr("Select all")).clicked() {
                        for entry in &mut batch.entries {
                            entry.selected = matches!(entry.result, ResultState::Ready);
                        }
                    }
                    if ui.button(tr("Clear selection")).clicked() {
                        for entry in &mut batch.entries { entry.selected = false; }
                    }
                    ui.label(tf!("{total} selected", total = total));
                });
            }
            if let Some(error) = &batch.error { ui.colored_label(p.warning, error); }
            if let Some(entry) = batch.external_pending.as_ref().and_then(|id| batch.entries.iter().find(|entry| &entry.id == id)) {
                ui.add(egui::Label::new(RichText::new(tf!("Review in antivirus: {}", entry.name)).strong()).truncate()).on_hover_text(&entry.name);
                ui.label(RichText::new(tr("Wait for the scan to finish before continuing. If the antivirus said another scan is in progress, wait for it to finish, then use Retry this file.")).small().color(p.muted));
            }
            let reserve = if batch.external_pending.is_some() { 455.0 } else { 380.0 };
            let height = (ctx.content_rect().height() - reserve).clamp(80.0, 320.0);
            let mut single_selection = None;
            egui::ScrollArea::vertical().id_salt("batch-scan-files").max_height(height).auto_shrink([false, false]).show_rows(ui, 48.0, batch.entries.len(), |ui, rows| {
                for index in rows {
                    let entry = &mut batch.entries[index];
                    ui.push_id(&entry.id, |ui| {
                        ui.horizontal(|ui| {
                            let enabled = !batch.started && matches!(entry.result, ResultState::Ready);
                            if single {
                                let radio = ui.add_enabled(enabled, egui::RadioButton::new(entry.selected, ""));
                                radio.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled, entry.selected, &entry.name));
                                if radio.clicked() { single_selection = Some(index); }
                            } else {
                                let checkbox = ui.add_enabled(enabled, egui::Checkbox::without_text(&mut entry.selected));
                                checkbox.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, entry.selected, &entry.name));
                            }
                            ui.vertical(|ui| {
                                let name = ui.add(egui::Label::new(RichText::new(&entry.name).strong()).truncate().sense(if single && enabled { egui::Sense::click() } else { egui::Sense::hover() })).on_hover_text(&entry.name);
                                if single && enabled && name.clicked() { single_selection = Some(index); }
                                let (label, color, detail) = match &entry.result {
                                    ResultState::Ready => (tr("Ready to scan"), p.muted, ""),
                                    ResultState::Waiting => (self.antivirus.service.status(&entry.id).unwrap_or(tr("Waiting to scan…")), p.accent, ""),
                                    ResultState::Done(verdict, message) => (tr(verdict.label()), if *verdict == Verdict::NoThreatsFound { p.accent } else if *verdict == Verdict::ScanRequested { p.muted } else { p.warning }, message.as_str()),
                                    ResultState::Skipped(reason) => (tr("Skipped"), p.muted, reason.as_str()),
                                };
                                ui.add(egui::Label::new(RichText::new(if detail.is_empty() { label.into() } else { format!("{label} · {detail}") }).small().color(color)).truncate()).on_hover_text(detail);
                            });
                        });
                    });
                }
            });
            if let Some(selected) = single_selection {
                for (index, entry) in batch.entries.iter_mut().enumerate() {
                    entry.selected = index == selected;
                }
            }
            if batch.entries.is_empty() { ui.label(tr("No completed downloads to scan.")); }
            ui.label(RichText::new(tr("Closing cancels pending scans. Saved results remain with each download.")).small().color(p.muted));
            form_dialog::controls(ui, |ui| {
                if batch.external_pending.is_some() {
                    let label = if single { tr("Finish review") } else if batch.entries.iter().any(|entry| matches!(entry.result, ResultState::Waiting)) { tr("Scan next file") } else { tr("Finish batch") };
                    if form_dialog::primary_button(ui, label).clicked() { external_continue = Some(false); }
                    if form_dialog::secondary_button(ui, tr("Retry this file")).clicked() { external_continue = Some(true); }
                }
                if batch.running {
                    stop = form_dialog::danger_button(ui, if single { tr("Cancel scan") } else { tr("Stop batch") }, p).clicked();
                } else if !batch.started {
                    start = form_dialog::primary_enabled(ui, total > 0, tr("Start scan")).clicked();
                } else if !batch.busy() {
                    again = form_dialog::primary_button(ui, tr("New scan")).clicked();
                }
                if !batch.started { settings = form_dialog::secondary_button(ui, tr("Scan settings")).clicked(); }
            });
        });
        if close || response.should_close() || settings {
            self.stop_antivirus_batch();
            self.antivirus.batch = None;
            self.dialog = None;
            if settings {
                self.open_antivirus_settings();
            }
        } else if stop {
            self.stop_antivirus_batch();
        } else if start {
            self.start_antivirus_batch();
        } else if let Some(retry) = external_continue {
            self.continue_external_batch(retry);
        } else if again {
            let mode = batch.mode;
            self.antivirus.batch = None;
            self.dialog = None;
            self.open_antivirus_scan(mode);
        }
    }
}

#[cfg(test)]
mod tests;
