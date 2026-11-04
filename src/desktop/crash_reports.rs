//! Failure reports use the shared modal and existing background-I/O lifecycle.
use super::*;
use download_engine::crash_reports::{self as service, Kind, SavedReport, Snapshot};
use std::path::Path;

enum Completed {
    Loaded(Snapshot),
    Exported(PathBuf),
    Acknowledged,
}

#[derive(Default)]
pub(super) struct State {
    task: Option<background_io::Task<Completed>>,
    reports: Vec<SavedReport>,
    selected: usize,
    skipped: usize,
    automatic: bool,
    prompt: bool,
    error: Option<String>,
    saved: Option<PathBuf>,
}

#[cfg(feature = "screenshot")]
impl State {
    pub(super) fn capture_ready(&self) -> bool {
        self.task.is_none()
    }
}

impl Desktop {
    pub fn check_crash_reports_on_launch(&mut self, ctx: &egui::Context) {
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some() {
            return;
        }
        self.load_crash_reports(ctx, true);
    }

    fn load_crash_reports(&mut self, ctx: &egui::Context, automatic: bool) {
        if self.crash_reports.task.is_some() {
            return;
        }
        self.crash_reports.automatic = automatic;
        self.crash_reports.error = None;
        self.crash_reports.saved = None;
        let paths = self.store.paths.clone();
        match background_io::Task::spawn(Some(ctx.clone()), move || {
            Ok(Completed::Loaded(service::load(&paths)?))
        }) {
            Ok(task) => self.crash_reports.task = Some(task),
            Err(error) => self.crash_reports.error = Some(error.to_string()),
        }
    }

    pub(super) fn open_crash_reports(&mut self, ctx: &egui::Context) {
        self.crash_reports.automatic = false;
        self.crash_reports.prompt = false;
        self.dialog = Some(Dialog::CrashReports);
        self.load_crash_reports(ctx, false);
    }

    pub(super) fn poll_crash_reports(&mut self, ctx: &egui::Context) {
        if let Some(result) = self
            .crash_reports
            .task
            .as_mut()
            .and_then(background_io::Task::poll)
        {
            self.crash_reports.task = None;
            match result {
                Ok(Completed::Loaded(snapshot)) => {
                    self.crash_reports.selected = snapshot
                        .reports
                        .iter()
                        .position(|r| r.unreviewed)
                        .unwrap_or(0);
                    self.crash_reports.prompt = self.crash_reports.automatic
                        && snapshot.reports.iter().any(|r| r.unreviewed);
                    self.crash_reports.reports = snapshot.reports;
                    self.crash_reports.skipped = snapshot.skipped;
                }
                Ok(Completed::Exported(path)) => self.crash_reports.saved = Some(path),
                Ok(Completed::Acknowledged) => {}
                Err(error) => {
                    self.crash_reports.error = Some(error.to_string());
                    if self.dialog != Some(Dialog::CrashReports) {
                        self.notice = Some(Notice::error(tr("Crash reports could not be read or updated. Open Help → Crash reports to retry.").into()));
                    }
                }
            }
        }
        if self.crash_reports.task.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.crash_reports.prompt
            && !self.exit_requested
            && self.dialog.is_none()
            && self.preferences_window.is_none()
            && self.extras.exit_deadline.is_none()
            && self.extras.shutdown_deadline.is_none()
            && !egui::Popup::is_any_open(ctx)
            && ctx.input(|i| {
                i.viewport().focused != Some(false) && i.viewport().minimized != Some(true)
            })
        {
            self.crash_reports.prompt = false;
            self.dialog = Some(Dialog::CrashReports);
        }
    }

    fn export_crash_report(&mut self, ctx: &egui::Context) {
        if self.crash_reports.task.is_some() {
            return;
        }
        let Some(saved) = self.crash_reports.reports.get(self.crash_reports.selected) else {
            return;
        };
        let report = saved.report.clone();
        let result = self
            .folder_picker
            .choose_titled(
                "",
                &self.store.paths.downloads,
                tr("Choose where to save the crash report"),
            )
            .and_then(|folder| {
                folder
                    .map(|folder| {
                        background_io::Task::spawn(Some(ctx.clone()), move || {
                            Ok(Completed::Exported(service::export(
                                &report,
                                Path::new(&folder),
                            )?))
                        })
                    })
                    .transpose()
            });
        match result {
            Ok(Some(task)) => {
                self.crash_reports.task = Some(task);
                self.crash_reports.error = None;
                self.crash_reports.saved = None;
            }
            Ok(None) => {}
            Err(error) => self.crash_reports.error = Some(error.to_string()),
        }
    }

    fn close_crash_reports(&mut self, ctx: &egui::Context) {
        // Wait for an explicit export/acknowledgment before allowing dismissal.
        // State retains the worker until it finishes, including during shutdown.
        if self.crash_reports.task.is_some() {
            return;
        }
        let ids: Vec<_> = self
            .crash_reports
            .reports
            .iter()
            .filter(|r| r.unreviewed)
            .map(|r| r.report.id)
            .collect();
        if !ids.is_empty() {
            let paths = self.store.paths.clone();
            match background_io::Task::spawn(Some(ctx.clone()), move || {
                service::acknowledge(&paths, &ids)?;
                Ok(Completed::Acknowledged)
            }) {
                Ok(task) => self.crash_reports.task = Some(task),
                Err(error) => {
                    self.crash_reports.error = Some(error.to_string());
                    return;
                }
            }
        }
        self.crash_reports.automatic = false;
        self.crash_reports.prompt = false;
        self.dialog = None;
    }

    pub(super) fn crash_reports_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let state = &mut self.crash_reports;
        let busy = state.task.is_some();
        let (mut close, mut export, mut refresh) = (false, false, false);
        let response = form_dialog::modal("crash-reports", p).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 96.0).clamp(320.0, 640.0));
            close = form_dialog::title(ui, tr("Crash reports"), form_dialog::Icon::Symbol("\u{e783}"), p, !busy);
            ui.label(RichText::new(tr("Saved on this device. You choose what to share.")).color(p.muted));
            form_dialog::body(ui, "crash-report-body", |ui| {
                if !service::available() {
                    ui.colored_label(p.warning, tr("Crash reporting is unavailable for this session. Check the profile folder permissions."));
                }
                if busy { ui.horizontal(|ui| { ui.spinner(); ui.label(tr("Working…")); }); }
                if let Some(error) = &state.error { ui.colored_label(p.warning, error); }
                if let Some(path) = &state.saved {
                    ui.colored_label(p.success, tr("Crash report exported"));
                    ui.add(egui::Label::new(path.display().to_string()).wrap());
                }
                if state.skipped > 0 {
                    ui.colored_label(p.warning, tr("Some incomplete or invalid reports were skipped."));
                }
                if state.reports.is_empty() && !busy && state.error.is_none() {
                    ui.add_space(18.0);
                    ui.strong(tr("No crash reports saved"));
                    ui.label(tr("If a Rust panic or an unclean desktop shutdown is detected, a local report will appear here."));
                }
                if let Some(selected) = state.reports.get(state.selected) {
                    let label = |saved: &SavedReport| format!("{} UTC · v{}", saved.report.recorded_utc.replace('T', " ").trim_end_matches('Z'), saved.report.version);
                    egui::ComboBox::from_id_salt("crash-report-selection").selected_text(label(selected)).width(ui.available_width() - 12.0).show_ui(ui, |ui| {
                        for (index, report) in state.reports.iter().enumerate() {
                            ui.selectable_value(&mut state.selected, index, label(report));
                        }
                    });
                }
                if let Some(saved) = state.reports.get(state.selected) {
                    let report = &saved.report;
                    ui.add_space(8.0);
                    ui.strong(tr(match report.kind { Kind::RustPanic => "A Rust panic was captured", Kind::UnexpectedExit => "A previous session did not close normally" }));
                    ui.label(tr(match report.kind {
                        Kind::RustPanic => "A component failed unexpectedly. A worker panic does not always close the whole app.",
                        Kind::UnexpectedExit => "A crash, power loss or forced termination may cause this. The exact cause is unknown.",
                    }));
                    ui.add_space(8.0);
                    egui::Grid::new("crash-summary").num_columns(2).spacing([18.0, 6.0]).show(ui, |ui| {
                        ui.label(tr("Version")); ui.label(&report.version); ui.end_row();
                        ui.label(tr("Platform")); ui.label(format!("{} / {}", report.os, report.architecture)); ui.end_row();
                        ui.label(tr("Component")); ui.label(tr(match report.component { download_engine::diagnostics::Component::Desktop => "Desktop", download_engine::diagnostics::Component::NativeHost => "Browser connector" })); ui.end_row();
                        if let Some(location) = &report.location {
                            ui.label(tr("Code location")); ui.add(egui::Label::new(format!("{}:{}:{}", location.file, location.line, location.column)).wrap()); ui.end_row();
                        }
                    });
                    ui.add_space(10.0);
                    ui.label(RichText::new(tr("No passwords, download URLs, personal file paths or memory dumps are included. Nothing is uploaded automatically.")).small().color(p.muted));
                    match service::preview(report) {
                        Ok(mut text) => {
                            ui.collapsing(tr("Review report contents"), |ui| {
                                ui.add(egui::TextEdit::multiline(&mut text.as_str()).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(14));
                            });
                            if ui.button(tr("Copy report")).clicked() { ctx.copy_text(std::mem::take(&mut text)); }
                        }
                        Err(error) => { ui.colored_label(p.warning, error.to_string()); },
                    }
                }
            });
            form_dialog::controls(ui, |ui| {
                export = form_dialog::primary_enabled(ui, !busy && !state.reports.is_empty(), tr("Export report…")).clicked();
                refresh = form_dialog::secondary_enabled(ui, !busy, tr("Refresh")).clicked();
                if form_dialog::secondary_enabled(ui, !busy, tr("Report an issue")).clicked() {
                    // Open the issue form without embedding the report or any user data.
                    ctx.open_url(egui::OpenUrl::new_tab("https://github.com/JasXTD/Download-Engine/issues/new"));
                }
            });
        });
        if (close || response.should_close()) && !busy {
            self.close_crash_reports(ctx);
        } else if export {
            self.export_crash_report(ctx);
        } else if refresh {
            self.load_crash_reports(ctx, false);
        }
    }
}

#[cfg(test)]
mod tests;
