use super::*;
use download_engine::extensions::discovery::{self, Report, Status};

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct State {
    pub(super) directory: Option<PathBuf>,
    pub(super) report: Option<Report>,
    task: Option<background_io::Task<Report>>,
    error: Option<String>,
    shown: bool,
}

impl Desktop {
    #[cfg(feature = "screenshot")]
    pub(in super::super) fn plugin_discovery_capture_ready(&self) -> bool {
        self.extensions.discovery.task.is_none()
    }

    pub(in super::super) fn start_plugin_discovery(&mut self) {
        let state = &mut self.extensions.discovery;
        if state.task.is_some() {
            return;
        }
        let result = (|| -> anyhow::Result<_> {
            let directory = match &state.directory {
                Some(directory) => directory.clone(),
                None => discovery::directory()?,
            };
            state.directory = Some(directory.clone());
            background_io::Task::spawn(self.background_io.context.clone(), move || {
                discovery::scan(&directory)
            })
        })();
        match result {
            Ok(task) => state.task = Some(task),
            Err(error) => state.error = Some(format!("{error:#}")),
        }
    }

    pub(in super::super) fn poll_plugin_discovery(&mut self, ctx: &egui::Context) {
        let state = &mut self.extensions.discovery;
        if self.view != View::Automation {
            state.shown = false;
        }
        if let Some(result) = state.task.as_mut().and_then(background_io::Task::poll) {
            state.task = None;
            match result {
                Ok(report) => {
                    state.report = Some(report);
                    state.error = None;
                }
                Err(error) => {
                    state.report = None;
                    state.error = Some(format!("{error:#}"));
                }
            }
        }
        if state.task.is_some() {
            // The startup scan is launched before we have an egui context.
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    pub(super) fn plugin_discovery_panel(&mut self, ui: &mut egui::Ui, p: Palette) {
        if !self.extensions.discovery.shown {
            self.extensions.discovery.shown = true;
            self.start_plugin_discovery();
        }
        let state = &self.extensions.discovery;
        let mut refresh = false;
        let mut open = false;
        let mut review = None;
        group(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("From plugins folder")).strong());
                open = ui.add_enabled(state.directory.is_some(), egui::Button::new(tr("Open folder"))).clicked();
                refresh = ui.add_enabled(state.task.is_none(), egui::Button::new(tr("Refresh"))).clicked();
                if state.task.is_some() {
                    ui.spinner();
                }
            });
            if let Some(directory) = &state.directory {
                ui.add(egui::Label::new(RichText::new(directory.display().to_string()).small().color(p.muted)).truncate())
                    .on_hover_text(directory.display().to_string());
            }
            if let Some(error) = &state.error {
                ui.add(egui::Label::new(RichText::new(error).color(p.warning)).truncate()).on_hover_text(error);
            }
            if let Some(report) = &state.report {
                let mut available = false;
                egui::ScrollArea::vertical().id_salt("plugin-discovery")
                    .max_height((ui.available_height() * 0.35).clamp(64.0, 160.0))
                    .auto_shrink([false, true]).show(ui, |ui| {
                    for (index, candidate) in report.candidates.iter().enumerate() {
                        let installed = self.extensions.entries.iter().find(|e| e.id() == candidate.package.id);
                        let status = candidate.status(installed.map(|e| &e.installed.package));
                        if matches!(status, Ok(Status::Current)) {
                            continue;
                        }
                        available = true;
                        ui.push_id(index, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                let action = match status {
                                    Ok(Status::New) => Some(tr("Review install")),
                                    Ok(Status::Update) => Some(tr("Review update")),
                                    _ => None,
                                };
                                if let Some(action) = action
                                    && ui.add(primary(action, p)).clicked() {
                                        review = Some(index);
                                    }
                                ui.label(RichText::new(format!("{} · {}", candidate.package.name, candidate.package.version)).strong())
                                    .on_hover_text(candidate.path.display().to_string());
                                match status {
                                    Ok(Status::Older) => { ui.label(tr("Installed version is newer")); }
                                    Ok(Status::Changed) => { ui.colored_label(p.warning, tr("Changed package: its author must increase the version")); }
                                    Err(error) => { ui.colored_label(p.warning, error.to_string()); }
                                    _ => {}
                                }
                            });
                        });
                    }
                    for issue in &report.issues {
                        available = true;
                        let text = format!("{}: {}", issue.path.file_name().unwrap_or_default().to_string_lossy(), issue.message);
                        ui.add(egui::Label::new(RichText::new(&text).color(p.warning)).truncate()).on_hover_text(&text);
                    }
                });
                if !available {
                    ui.label(RichText::new(tr("No new packages. Add a .de-extension.json file here, then Refresh.")).small().color(p.muted));
                } else {
                    ui.label(RichText::new(tr("Review permissions before installing or updating. New extensions start disabled.")).small().color(p.muted));
                }
            }
        });
        ui.add_space(8.0);
        if refresh {
            self.start_plugin_discovery();
        }
        if open && let Some(directory) = self.extensions.discovery.directory.clone() {
            self.open_folder_async(directory);
        }
        if let Some(index) = review {
            self.review_discovered_extension(index);
        }
    }

    pub(super) fn review_discovered_extension(&mut self, index: usize) {
        if let Some(candidate) = self
            .extensions
            .discovery
            .report
            .as_ref()
            .and_then(|r| r.candidates.get(index))
        {
            // Recheck against the current database, but retain the scanned bytes.
            // Installing uses this same snapshot even if a subsequent scan finishes.
            match self.store.review_extension(candidate.bytes()) {
                Ok(review) => self.review_extension(candidate.bytes().to_vec(), review),
                Err(error) => self.extensions.error = Some(format!("{error:#}")),
            }
        }
    }
}
