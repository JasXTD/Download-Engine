use super::*;
use anyhow::Context;
use download_engine::{
    browser_batch::{Destination, Link, Options},
    grabber_projects::{Project, Settings},
    website_grabber::{self, Config, Progress, Report},
};
use std::sync::mpsc;
use tokio_util::sync::CancellationToken;
mod offline;
mod schedule;

// Both modes are tabs of one dialog. A different area ID restarts egui's
// invisible sizing pass and fades the full-window backdrop on every switch.
const DIALOG_ID: &str = "website-grabber";

enum ProjectAction {
    Open(String),
    New,
    Save { copy: bool },
    Delete,
    SetSchedule,
    ClearSchedule,
    ResetHistory,
    ReviewPending,
    IgnoreSelected,
}

struct Task {
    cancel: CancellationToken,
    updates: mpsc::Receiver<Progress>,
    result: mpsc::Receiver<anyhow::Result<Report>>,
}

impl Drop for Task {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub(super) struct State {
    offline_mode: bool,
    offline: offline::State,
    url: String,
    extensions: String,
    depth: u8,
    page_limit: usize,
    queue: String,
    destination: usize,
    options: Options,
    task: Option<Task>,
    progress: Progress,
    report: Option<Report>,
    selected: Vec<bool>,
    search: String,
    error: Option<String>,
    receipt: String,
    projects: Vec<Project>,
    project: Option<Project>,
    project_name: String,
    project_notice: Option<String>,
    confirm_delete: bool,
    schedule: schedule::Form,
    pending: Option<Vec<download_engine::grabber_schedule::Discovery>>,
}
impl State {
    fn new(queue: String, options: Options) -> Self {
        Self {
            offline_mode: false,
            offline: offline::State::default(),
            url: String::new(),
            extensions: "zip, 7z, rar, pdf".into(),
            depth: 1,
            page_limit: 50,
            queue,
            destination: 0,
            options,
            task: None,
            progress: Progress::default(),
            report: None,
            selected: Vec::new(),
            search: String::new(),
            error: None,
            receipt: format!("batch:{}", Uuid::new_v4()),
            projects: Vec::new(),
            project: None,
            project_name: String::new(),
            project_notice: None,
            confirm_delete: false,
            schedule: schedule::Form::default(),
            pending: None,
        }
    }

    fn settings(&self) -> anyhow::Result<Settings> {
        Ok(Settings {
            url: self.url.clone(),
            extensions: self.extensions.clone(),
            depth: self.depth,
            page_limit: self.page_limit,
            queue: self.queue.clone(),
            destination: self
                .options
                .destinations
                .get(self.destination)
                .ok_or_else(|| anyhow::anyhow!(tr("Choose a destination folder")))?
                .id,
        })
    }

    fn open_project(&mut self, project: Project, options: Options) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.task.is_none(),
            tr("Stop the scan before opening a project")
        );
        let destination = options
            .destinations
            .iter()
            .position(|d| d.id == project.settings.destination)
            .ok_or_else(|| anyhow::anyhow!(tr("This project's destination is unavailable")))?;
        self.url = project.settings.url.clone();
        self.extensions = project.settings.extensions.clone();
        self.depth = project.settings.depth;
        self.page_limit = project.settings.page_limit;
        self.queue = project.settings.queue.clone();
        self.destination = destination;
        self.options = options;
        self.project_name = project.name.clone();
        self.project = Some(project);
        self.schedule = schedule::Form::default();
        self.pending = None;
        self.confirm_delete = false;
        self.project_notice = None;
        self.report = None;
        self.selected.clear();
        self.search.clear();
        self.progress = Progress::default();
        self.error = None;
        self.receipt = format!("batch:{}", Uuid::new_v4());
        Ok(())
    }

    fn project_action(
        &mut self,
        store: &Store,
        action: ProjectAction,
        default_queue: &str,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.task.is_none(),
            tr("Stop the scan before changing projects")
        );
        match action {
            ProjectAction::Open(id) => {
                let projects = store.grabber_projects()?;
                let project = projects.iter().find(|p| p.id == id).cloned()
                    .ok_or_else(|| anyhow::anyhow!(tr("Project no longer exists. Close and reopen the grabber to refresh the list.")))?;
                self.open_project(project, store.browser_batch_options()?)?;
                self.load_schedule(store)?;
                self.projects = projects;
            }
            ProjectAction::New => {
                let projects = store.grabber_projects()?;
                *self = Self::new(default_queue.into(), store.browser_batch_options()?);
                self.projects = projects;
            }
            ProjectAction::Save { copy } => {
                let settings = self.settings()?;
                let saved = match &self.project {
                    Some(project) if !copy => {
                        store.update_grabber_project(project, &self.project_name, &settings)?
                    }
                    _ => store.create_grabber_project(&self.project_name, &settings)?,
                };
                // Keep a successful write's identity even if a later list refresh fails.
                // Retrying must update this project rather than create a second one.
                self.project_name = saved.name.clone();
                self.url = saved.settings.url.clone();
                self.extensions = saved.settings.extensions.clone();
                self.project_notice = Some(tf!(
                    "Saved \"{}\". Reruns always open a fresh preview.",
                    saved.name
                ));
                self.project = Some(saved);
                self.confirm_delete = false;
                self.error = None;
                self.projects = store.grabber_projects()?;
            }
            ProjectAction::Delete => {
                if let Some(project) = &self.project {
                    store.delete_grabber_project(project)?;
                    self.project_notice = Some(tf!(
                        "Deleted \"{}\", its schedule and pending discoveries. Downloads are unchanged.",
                        project.name
                    ));
                    self.project = None;
                    self.pending = None;
                    self.schedule = schedule::Form::default();
                    self.project_name.clear();
                    self.confirm_delete = false;
                    self.error = None;
                    self.projects = store.grabber_projects()?;
                }
            }
            ProjectAction::SetSchedule => {
                let project = self.project.as_ref().context(tr("Save a project first"))?;
                anyhow::ensure!(
                    self.settings()? == project.settings && self.project_name == project.name,
                    tr("Save your project changes before scheduling scans")
                );
                let now = chrono::Utc::now().timestamp();
                let at = local_schedule(&self.schedule.date, &self.schedule.time, now)?;
                store.set_grabber_schedule(
                    project,
                    at,
                    self.schedule.repeat.then_some(self.schedule.hours),
                    now,
                )?;
                self.project_notice =
                    Some(tr("Scan schedule saved. New files will wait for your review.").into());
                self.error = None;
            }
            ProjectAction::ClearSchedule => {
                store.clear_grabber_schedule(
                    self.project.as_ref().context(tr("Save a project first"))?,
                )?;
                self.project_notice =
                    Some(tr("Schedule cleared. Pending files are kept for review.").into());
                self.error = None;
            }
            ProjectAction::ResetHistory => {
                store.reset_grabber_history(
                    self.project.as_ref().context(tr("Save a project first"))?,
                )?;
                self.schedule.confirm_reset = false;
                self.project_notice = Some(
                    tr("Reviewed links can appear again in future scans. Pending files are kept.")
                        .into(),
                );
                self.error = None;
            }
            ProjectAction::ReviewPending => self.load_pending(store)?,
            ProjectAction::IgnoreSelected => {
                let project = self
                    .project
                    .as_ref()
                    .context(tr("Project no longer exists"))?;
                store.review_grabber_files(&project.id, &self.selected_pending(), None)?;
                self.load_pending(store)?;
            }
        }
        Ok(())
    }

    fn project_controls(&mut self, ui: &mut egui::Ui, p: Palette) -> Option<ProjectAction> {
        let mut action = None;
        let preview = self.report.is_some();
        egui::CollapsingHeader::new(tr("Saved projects"))
            .id_salt(("grabber-projects", preview))
            .default_open(!preview)
            .show(ui, |ui| {
                egui::Frame::new()
                    .fill(p.background)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.add_enabled_ui(self.task.is_none(), |ui| {
                            ui.horizontal_wrapped(|ui| {
                                let selected = self
                                    .project
                                    .as_ref()
                                    .map(|p| p.name.as_str())
                                    .unwrap_or(tr("Unsaved scan"));
                                egui::ComboBox::from_id_salt("grabber-project-list")
                                    .selected_text(short_queue_name(selected))
                                    .width(170.0)
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(self.project.is_none(), tr("New scan"))
                                            .clicked()
                                        {
                                            action = Some(ProjectAction::New);
                                        }
                                        for project in &self.projects {
                                            if ui
                                                .selectable_label(
                                                    self.project
                                                        .as_ref()
                                                        .is_some_and(|p| p.id == project.id),
                                                    &project.name,
                                                )
                                                .clicked()
                                            {
                                                action =
                                                    Some(ProjectAction::Open(project.id.clone()));
                                            }
                                        }
                                    })
                                    .response
                                    .on_hover_text(tr("Open saved project settings"));
                                ui.label(tr("Project name"));
                                ui.add(
                                    view::line_edit(&mut self.project_name)
                                        .desired_width(180.0)
                                        .hint_text(tr("e.g. Product manuals"))
                                        .char_limit(download_engine::grabber_projects::MAX_NAME_CHARS),
                                );
                                let label = if self.project.is_some() {
                                    tr("Save changes")
                                } else {
                                    tr("Save project")
                                };
                                if ui.button(label).clicked() {
                                    action = Some(ProjectAction::Save { copy: false });
                                }
                                if self.project.is_some() {
                                    if ui
                                        .button(tr("Save as new"))
                                        .on_hover_text(
                                            tr("Enter a different project name to keep a separate copy"),
                                        )
                                        .clicked()
                                    {
                                        action = Some(ProjectAction::Save { copy: true });
                                    }
                                    if ui.button(tr("Delete project")).clicked() {
                                        self.confirm_delete = true;
                                    }
                                }
                            });
                            if self.confirm_delete {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        tr("Delete this project, its schedule and pending discoveries?"),
                                    );
                                    if ui.button(tr("Delete saved project")).clicked() {
                                        action = Some(ProjectAction::Delete);
                                    }
                                    if ui.button(tr("Keep project")).clicked() {
                                        self.confirm_delete = false;
                                    }
                                });
                            }
                            if let Some(notice) = &self.project_notice {
                                ui.label(RichText::new(notice).small().color(p.success));
                            }
                        });
                    });
            });
        action
    }

    fn destination_controls(&mut self, ui: &mut egui::Ui, queues: &[DownloadQueue], p: Palette) {
        ui.add_enabled_ui(self.task.is_none(), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("Queue"));
                let queue_name = queues
                    .iter()
                    .find(|q| q.id == self.queue)
                    .map(|q| display_queue_name(&q.id, &q.name))
                    .unwrap_or(tr("Queue removed — choose another"));
                egui::ComboBox::from_id_salt("grabber-queue")
                    .selected_text(short_queue_name(queue_name))
                    .width(185.0)
                    .show_ui(ui, |ui| {
                        for queue in queues {
                            ui.selectable_value(
                                &mut self.queue,
                                queue.id.clone(),
                                display_queue_name(&queue.id, &queue.name),
                            );
                        }
                    })
                    .response
                    .on_hover_text(queue_name);
                ui.add_space(8.0);
                ui.label(tr("Save to"));
                let folder_name = self
                    .options
                    .destinations
                    .get(self.destination)
                    .map(|f| f.name.as_str())
                    .unwrap_or(tr("Choose a folder"));
                egui::ComboBox::from_id_salt("grabber-folder")
                    .selected_text(folder_name)
                    .width(185.0)
                    .show_ui(ui, |ui| {
                        for (i, folder) in self.options.destinations.iter().enumerate() {
                            ui.selectable_value(&mut self.destination, i, &folder.name)
                                .on_hover_text(
                                    folder.path.as_deref().unwrap_or(tr(
                                        "Uses your saved folder rules for each file",
                                    )),
                                );
                        }
                    });
            });
        });
        if !queues.iter().any(|q| q.id == self.queue) {
            ui.colored_label(
                p.warning,
                tr("This project's queue was removed. Choose another before saving or adding files."),
            );
        }
    }
    fn receive(&mut self, report: Report) {
        self.pending = None;
        self.selected = vec![false; report.files.len()];
        self.report = Some(report);
        self.task = None;
        self.search.clear();
    }
    fn poll(&mut self) {
        let Some(task) = &self.task else { return };
        for update in task.updates.try_iter() {
            self.progress = update;
        }
        match task.result.try_recv() {
            Ok(Ok(report)) => self.receive(report),
            Ok(Err(error)) => {
                self.error = Some(format!("{error:#}"));
                self.task = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.error =
                    Some(tr("The scan worker stopped unexpectedly. Try scanning again.").into());
                self.task = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
    fn start(&mut self, ctx: &egui::Context, store: &Store) -> anyhow::Result<()> {
        anyhow::ensure!(self.task.is_none(), tr("A scan is already running"));
        let config = Config::new(&self.url, &self.extensions, self.depth, self.page_limit)?
            .with_proxy(store)?;
        let (sender, result) = mpsc::channel();
        let (updates_sender, updates) = mpsc::sync_channel(1);
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let context = ctx.clone();
        std::thread::Builder::new()
            .name("website-grabber".into())
            .spawn(move || {
                let outcome = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(anyhow::Error::from)
                    .and_then(|runtime| {
                        runtime.block_on(website_grabber::scan(config, token, |progress| {
                            let _ = updates_sender.try_send(progress);
                            context.request_repaint();
                        }))
                    });
                let _ = sender.send(outcome);
                context.request_repaint();
            })?;
        self.task = Some(Task {
            cancel,
            updates,
            result,
        });
        self.report = None;
        self.pending = None;
        self.selected.clear();
        self.progress = Progress::default();
        self.error = None;
        self.receipt = format!("batch:{}", Uuid::new_v4());
        Ok(())
    }
    fn selected_links(&self) -> Vec<Link> {
        self.report
            .as_ref()
            .map(|report| {
                report
                    .files
                    .iter()
                    .zip(&self.selected)
                    .filter(|(_, selected)| **selected)
                    .map(|(file, _)| Link {
                        url: file.url.clone(),
                        filename: file.filename.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    fn visible(&self) -> Vec<usize> {
        let search = self.search.trim().to_lowercase();
        self.report
            .as_ref()
            .map(|r| {
                r.files
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| {
                        search.is_empty()
                            || f.name.to_lowercase().contains(&search)
                            || f.url.to_lowercase().contains(&search)
                    })
                    .map(|(i, _)| i)
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Desktop {
    pub(super) fn open_grabber(&mut self) {
        if self.dialog.is_some() {
            return;
        }
        match self
            .store
            .browser_batch_options()
            .and_then(|options| Ok((options, self.store.grabber_projects()?)))
        {
            Ok((options, projects)) => {
                let mut draft = State::new(self.queue_id.clone(), options);
                draft.projects = projects;
                self.grabber = Some(draft);
                self.dialog = Some(Dialog::Grabber);
            }
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot open website grabber: {error:#}",
                    error = error
                )))
            }
        }
    }

    fn add_grabbed_files(&mut self) {
        let Some(draft) = &mut self.grabber else {
            return;
        };
        if draft.task.is_some() {
            return;
        }
        let links = draft.selected_links();
        let destination = draft
            .options
            .destinations
            .get(draft.destination)
            .map(|d| d.id)
            .unwrap_or(Destination::Automatic);
        let result = if draft.pending.is_some() {
            draft
                .project
                .as_ref()
                .context(tr("Project no longer exists"))
                .and_then(|project| {
                    self.store.review_grabber_files(
                        &project.id,
                        &draft.selected_pending(),
                        Some((&draft.queue, destination)),
                    )
                })
        } else {
            self.store
                .enqueue_browser_batch(&draft.receipt, &links, &draft.queue, destination)
        };
        match result {
            Ok(ids) => {
                self.queue_id = draft.queue.clone();
                self.set_selection(ids.first().cloned());
                self.filter = Filter::All;
                self.search.clear();
                self.category = None;
                self.view = View::Downloads;
                self.notice = Some(Notice::success(tf!(
                    "Added {} selected files from the website grabber.",
                    ids.len()
                )));
                self.grabber = None;
                self.dialog = None;
                self.force_refresh();
            }
            Err(error) => draft.error = Some(tf!("Could not add files: {error:#}", error = error)),
        }
    }

    pub(super) fn grabber_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        if self
            .grabber
            .as_ref()
            .is_some_and(|draft| draft.offline_mode)
        {
            self.offline_website_dialog(ctx, p);
            return;
        }
        let Some(draft) = &mut self.grabber else {
            self.dialog = None;
            return;
        };
        draft.poll();
        let (mut scan, mut add, mut close, mut edit) = (false, false, false, false);
        let mut project_action = None;
        let running = draft.task.is_some();
        let preview = draft.report.is_some();
        let scheduling = draft.schedule.visible;
        let persistent_preview = draft.pending.is_some() && !scheduling;
        let height = (ctx.content_rect().height() - 120.0).max(300.0);
        let response = form_dialog::modal(DIALOG_ID, p).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 120.0).clamp(400.0, 760.0));
            close = form_dialog::title(
                ui,
                if scheduling {
                    tr("Schedule website scan")
                } else {
                    tr("Website grabber")
                },
                form_dialog::Icon::Symbol("\u{e774}"),
                p,
                true,
            );
            ui.label(
                RichText::new(if scheduling {
                    tr("Run automatic scans and collect new links for review.")
                } else {
                    tr("Scan a page for file links, review matches, and queue selected files.")
                })
                .small()
                .color(p.muted),
            );
            ui.add_space(4.0);
            if !scheduling && draft.pending.is_none()
                && offline::mode_switch(ui, &mut draft.offline_mode, running) && draft.offline_mode {
                draft.offline.seed_url(&draft.url);
            }
            egui::ScrollArea::vertical()
                .id_salt("grabber-body")
                .max_height(height - 75.0 - if persistent_preview { 65.0 } else { 0.0 })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    if scheduling {
                        if let Some(project) = &draft.project {
                            ui.strong(&project.name);
                        }
                    } else if draft.pending.is_none() {
                        project_action = draft.project_controls(ui, p);
                    }
                    let summary = draft.project.as_ref().and_then(|project| {
                        self.grabber_summaries
                            .iter()
                            .find(|s| s.project == project.id)
                    });
                    if draft.pending.is_some() && !scheduling {
                        ui.horizontal_wrapped(|ui| {
                            if let Some(project) = &draft.project {
                                ui.strong(&project.name);
                            }
                            if ui.button(tr("Refresh preview")).clicked() {
                                project_action = Some(ProjectAction::ReviewPending);
                            }
                        });
                    } else if let Some(action) = draft.schedule_controls(ui, summary, p) {
                        project_action = Some(action);
                    }
                    if scheduling {
                        if let Some(error) = &draft.error {
                            ui.colored_label(p.warning, error);
                        }
                        return;
                    }
                    if !preview {
                        egui::Frame::new()
                            .fill(p.background)
                            .stroke(egui::Stroke::new(1.0, p.border))
                            .corner_radius(8)
                            .inner_margin(egui::Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.spacing_mut().item_spacing.y = 6.0;
                                ui.add_enabled_ui(!running, |ui| {
                                    ui.label(tr("Starting page"));
                                    ui.add(
                                        view::line_edit(&mut draft.url)
                                            .desired_width(f32::INFINITY)
                                            .hint_text("https://example.com/downloads/")
                                            .char_limit(16384),
                                    );
                                    ui.label(tr("File types"));
                                    ui.add(
                                        view::line_edit(&mut draft.extensions)
                                            .desired_width(f32::INFINITY)
                                            .hint_text("pdf, zip, jpg")
                                            .char_limit(512),
                                    );
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(tr("Presets:")).small().color(p.muted));
                                        for (label, types) in [
                                            (
                                                tr("Documents"),
                                                "pdf, doc, docx, xls, xlsx, ppt, pptx, txt, epub",
                                            ),
                                            (tr("Archives"), "zip, 7z, rar, tar, tar.gz, gz"),
                                            (
                                                tr("Images"),
                                                "jpg, jpeg, png, gif, webp, avif, svg",
                                            ),
                                            (tr("Audio"), "mp3, flac, wav, m4a, ogg"),
                                            (tr("Video"), "mp4, webm, mov, mkv"),
                                        ] {
                                            if ui.small_button(label).clicked() {
                                                draft.extensions = types.into();
                                            }
                                        }
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label(tr("Scan depth"));
                                        ui.add(
                                            egui::DragValue::new(&mut draft.depth)
                                                .range(0..=website_grabber::MAX_DEPTH),
                                        );
                                        ui.add_space(8.0);
                                        ui.label(tr("Page limit"));
                                        ui.add(
                                            egui::DragValue::new(&mut draft.page_limit)
                                                .range(1..=website_grabber::MAX_PAGES),
                                        );
                                    });
                                });
                                ui.separator();
                                draft.destination_controls(ui, &self.queues, p);
                                ui.label(
                                    RichText::new(
                                        tr("Depth 0 scans the starting page and its HTML fragments. Fragments count toward the page limit (up to 1,000 matches)."),
                                    )
                                    .small()
                                    .color(p.muted),
                                );
                                if running {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(tf!(
                                            "{} pages read · {} matches",
                                            draft.progress.pages_read, draft.progress.files_found
                                        ));
                                    });
                                    ui.add(egui::Label::new(&draft.progress.current_page).truncate());
                                }
                            });
                    } else {
                        egui::Frame::new()
                            .fill(p.background)
                            .stroke(egui::Stroke::new(1.0, p.border))
                            .corner_radius(8)
                            .inner_margin(egui::Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.spacing_mut().item_spacing.y = 6.0;
                                let report = draft.report.as_ref().expect("preview exists");
                                if draft.pending.is_some() {
                                    ui.strong(tf!(
                                        "New files awaiting review · {} files",
                                        report.files.len()
                                    ));
                                    if let Some(summary) = summary {
                                        ui.label(
                                            RichText::new(&summary.message).small().color(p.muted),
                                        );
                                    }
                                } else {
                                    ui.strong(tf!(
                                        "{} · {} files · {} pages read",
                                        tr(report.stop.label()),
                                        report.files.len(),
                                        report.pages_read
                                    ));
                                }
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(tf!(
                                            "{} · depth {} · {}",
                                            draft.url, draft.depth, draft.extensions
                                        ))
                                        .small()
                                        .color(p.muted),
                                    )
                                    .truncate(),
                                );
                                if !report.issues.is_empty() {
                                    egui::CollapsingHeader::new(tf!(
                                        "Scan notes ({} pages skipped or failed)",
                                        report.pages_failed
                                    ))
                                    .show(ui, |ui| {
                                        for issue in &report.issues {
                                            ui.add(egui::Label::new(&issue.page).truncate())
                                                .on_hover_text(&issue.page);
                                            ui.label(
                                                RichText::new(&issue.message)
                                                    .small()
                                                    .color(p.warning),
                                            );
                                        }
                                        if report.pages_failed > report.issues.len() {
                                            ui.label(tr("Only the first 20 notes are shown."));
                                        }
                                    });
                                }
                                ui.add(
                                    view::line_edit(&mut draft.search)
                                        .desired_width(f32::INFINITY)
                                        .hint_text(tr("Filter preview by filename or URL")),
                                );
                                let visible = draft.visible();
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_enabled(
                                            !visible.is_empty(),
                                            egui::Button::new(tr("Select visible")),
                                        )
                                        .clicked()
                                    {
                                        for &i in &visible {
                                            draft.selected[i] = true;
                                        }
                                    }
                                    if ui.button(tr("Clear selection")).clicked() {
                                        draft.selected.fill(false);
                                    }
                                    ui.label(
                                        RichText::new(tf!(
                                            "{} selected / {} matches",
                                            draft.selected.iter().filter(|v| **v).count(),
                                            draft.selected.len()
                                        ))
                                        .small()
                                        .color(p.muted),
                                    );
                                });
                                egui::ScrollArea::vertical()
                                    .id_salt("grabber-files")
                                    .max_height((height - 340.0).clamp(90.0, 240.0))
                                    .show(ui, |ui| {
                                        for i in visible {
                                            let file = &draft
                                                .report
                                                .as_ref()
                                                .expect("preview exists")
                                                .files[i];
                                            ui.horizontal(|ui| {
                                                ui.square_checkbox(&mut draft.selected[i], "");
                                                ui.vertical(|ui| {
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(&file.name).strong(),
                                                        )
                                                        .truncate(),
                                                    )
                                                    .on_hover_text(tf!("Found on {}", file.page));
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(&file.url)
                                                                .small()
                                                                .color(p.muted),
                                                        )
                                                        .truncate(),
                                                    )
                                                    .on_hover_text(&file.url);
                                                });
                                            });
                                            ui.separator();
                                        }
                                    });
                                if draft.selected.is_empty() {
                                    ui.label(if draft.pending.is_some() {
                                        tr("No new files awaiting review.")
                                    } else {
                                        tr("No matching file links found. Try another page, more depth, or different file types.")
                                    });
                                }
                                if !persistent_preview {
                                    ui.separator();
                                    draft.destination_controls(ui, &self.queues, p);
                                }
                            });
                    }
                    if let Some(error) = &draft.error {
                        ui.colored_label(p.warning, error);
                    }
                });
            if persistent_preview {
                draft.destination_controls(ui, &self.queues, p);
            }
            form_dialog::controls(ui, |ui| {
                if scheduling {
                    if form_dialog::secondary_button(ui, tr("Back to project")).clicked() {
                        draft.schedule.visible = false;
                    }
                } else if running {
                    if form_dialog::secondary_button(ui, tr("Stop scan")).clicked() {
                        draft.task.as_ref().expect("running scan").cancel.cancel();
                    }
                } else if preview {
                    add = form_dialog::primary_enabled(
                        ui,
                        draft.selected.iter().any(|s| *s),
                        tr("Add selected to queue"),
                    )
                    .clicked();
                    if draft.pending.is_some() {
                        if form_dialog::secondary_enabled(
                            ui,
                            draft.selected.iter().any(|s| *s),
                            tr("Ignore selected"),
                        )
                        .on_hover_text(
                            tr("Remove selected discoveries from the preview. These URLs stay remembered and will not reappear."),
                        )
                        .clicked()
                        {
                            project_action = Some(ProjectAction::IgnoreSelected);
                        }
                        edit = form_dialog::secondary_button(ui, tr("Project settings")).clicked();
                    } else {
                        edit = form_dialog::secondary_button(ui, tr("Change scan")).clicked();
                        scan = form_dialog::secondary_button(ui, tr("Scan again")).clicked();
                    }
                } else {
                    scan = form_dialog::primary_button(ui, tr("Scan website")).clicked();
                }
            });
        });
        if close || response.should_close() {
            // Dropping a task cancels its requests. Its private result channel
            // cannot replace a later dialog's results.
            self.grabber = None;
            self.dialog = None;
        } else if let Some(action) = project_action {
            if let Err(error) = draft.project_action(&self.store, action, &self.queue_id) {
                draft.project_notice = None;
                draft.error = Some(format!("{error:#}"));
            }
            self.force_refresh();
        } else if scan {
            if let Err(error) = draft.start(ctx, &self.store) {
                draft.error = Some(error.to_string());
            }
        } else if edit {
            draft.report = None;
            draft.pending = None;
            draft.selected.clear();
            draft.error = None;
        } else if add {
            self.add_grabbed_files();
        }
    }

    #[cfg(feature = "screenshot")]
    pub(super) fn grabber_preview(&mut self) {
        self.grabber_project_preview();
        if let Some(draft) = &mut self.grabber {
            draft.url = "https://example.com/downloads/".into();
            draft.receive(Report {
                files: [
                    "Product-guide.pdf",
                    "Design-assets.zip",
                    "Reference-manual.pdf",
                ]
                .iter()
                .map(|name| website_grabber::Candidate {
                    url: format!("https://example.com/downloads/{name}"),
                    name: (*name).into(),
                    filename: None,
                    page: draft.url.clone(),
                })
                .collect(),
                pages_read: 4,
                ..Default::default()
            });
        }
    }

    #[cfg(feature = "screenshot")]
    pub(super) fn grabber_project_preview(&mut self) {
        self.open_grabber();
        if let Some(draft) = &mut self.grabber {
            draft.url = "https://example.com/downloads/".into();
            draft.extensions = "pdf, zip".into();
            draft.project_name = "Product manuals".into();
            if let Err(error) = draft.project_action(
                &self.store,
                ProjectAction::Save { copy: false },
                &self.queue_id,
            ) {
                draft.error = Some(error.to_string());
            }
        }
    }

    #[cfg(feature = "screenshot")]
    pub(super) fn grabber_schedule_preview(&mut self) {
        self.grabber_project_preview();
        if let Some(draft) = &mut self.grabber {
            draft.schedule.visible = true;
        }
    }

    #[cfg(feature = "screenshot")]
    pub(super) fn grabber_discoveries_preview(&mut self, overview: bool) {
        self.grabber_preview();
        let result = (|| -> anyhow::Result<()> {
            let draft = self.grabber.as_mut().context("Missing preview fixture")?;
            let project = draft.project.as_ref().context("Missing project fixture")?;
            let conn = rusqlite::Connection::open(self.store.paths.data.join("queue.sqlite3"))?;
            let at = chrono::Utc::now().timestamp();
            for file in &draft.report.as_ref().context("Missing file fixture")?.files {
                let fingerprint = <sha2::Sha256 as sha2::Digest>::digest(file.url.as_bytes());
                conn.execute("INSERT INTO grabber_pending(id,project_id,fingerprint,data,found_at) VALUES(?1,?2,?3,?4,?5)",
                    rusqlite::params![Uuid::new_v4().to_string(),project.id,&fingerprint[..],serde_json::to_string(file)?,at])?;
            }
            self.store
                .set_grabber_schedule(project, at + 86400, Some(24), at)?;
            conn.execute("UPDATE grabber_schedules SET last_at=?1,message='Scan finished · 3 new files · 4 pages read · 0 failed' WHERE project_id=?2",rusqlite::params![at,project.id])?;
            draft.load_pending(&self.store)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.notice = Some(Notice::error(format!("Preview fixture: {error:#}")));
        }
        if overview {
            self.grabber = None;
            self.dialog = None;
            self.view = View::Scheduler;
        }
        self.force_refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};
    use download_engine::website_grabber::Candidate;

    fn report() -> Report {
        Report {
            files: ["guide.pdf", "assets.zip", "manual.pdf"]
                .iter()
                .map(|name| Candidate {
                    url: format!("https://example.test/{name}"),
                    name: (*name).into(),
                    filename: None,
                    page: "https://example.test/".into(),
                })
                .collect(),
            pages_read: 2,
            ..Default::default()
        }
    }

    #[test]
    fn saved_grabber_projects_restore_update_copy_and_confirm_deletion_through_ui() {
        let (_temp, mut app, ctx) = fixture();
        let queue = app.store.create_queue("Manuals queue").unwrap();
        app.force_refresh();
        app.open_grabber();
        {
            let draft = app.grabber.as_mut().unwrap();
            draft.project_name = "Product manuals".into();
            draft.url = "https://example.test/manuals/?v=2".into();
            draft.extensions = "pdf, epub".into();
            draft.depth = 4;
            draft.page_limit = 123;
            draft.queue = queue.id.clone();
            draft.destination = draft
                .options
                .destinations
                .iter()
                .position(|d| d.id == Destination::Documents)
                .unwrap();
        }
        click(&mut app, &ctx, "Save project");
        let saved = app.store.grabber_projects().unwrap().remove(0);
        assert_eq!(
            saved.settings,
            app.grabber.as_ref().unwrap().settings().unwrap()
        );
        click(&mut app, &ctx, "×");
        app.store = Store::open(app.store.paths.clone()).unwrap();
        app.open_grabber();
        click(&mut app, &ctx, "Unsaved scan");
        click(&mut app, &ctx, "Product manuals");
        let draft = app.grabber.as_ref().unwrap();
        assert_eq!(draft.settings().unwrap(), saved.settings);
        assert!(draft.report.is_none() && draft.selected.is_empty() && draft.task.is_none());
        assert!(app.store.list().unwrap().is_empty());
        app.grabber.as_mut().unwrap().depth = 2;
        app.grabber.as_mut().unwrap().project_name = "Updated manuals".into();
        click(&mut app, &ctx, "Save changes");
        let updated = app.store.grabber_projects().unwrap().remove(0);
        assert_eq!(updated.id, saved.id);
        assert_eq!(updated.settings.depth, 2);
        assert_eq!(updated.name, "Updated manuals");
        app.grabber.as_mut().unwrap().project_name = "Separate copy".into();
        click(&mut app, &ctx, "Save as new");
        assert_eq!(app.store.grabber_projects().unwrap().len(), 2);
        click(&mut app, &ctx, "Delete project");
        click(&mut app, &ctx, "Keep project");
        assert_eq!(app.store.grabber_projects().unwrap().len(), 2);
        click(&mut app, &ctx, "Delete project");
        click(&mut app, &ctx, "Delete saved project");
        assert_eq!(app.store.grabber_projects().unwrap(), [updated]);
        assert!(app.grabber.as_ref().unwrap().project.is_none());
        assert_eq!(app.grabber.as_ref().unwrap().depth, 2);
        assert!(app.store.list().unwrap().is_empty());
    }

    #[test]
    fn project_switch_clears_preview_and_running_scan_disables_project_changes() {
        let (_temp, mut app, ctx) = fixture();
        app.open_grabber();
        let draft = app.grabber.as_mut().unwrap();
        draft.url = "https://example.test/".into();
        draft.project_name = "Original project".into();
        draft
            .project_action(&app.store, ProjectAction::Save { copy: false }, MAIN_QUEUE)
            .unwrap();
        let project = draft.project.clone().unwrap();
        draft.receive(report());
        draft.selected.fill(true);
        draft.search = "guide".into();
        let receipt = draft.receipt.clone();
        draft
            .project_action(
                &app.store,
                ProjectAction::Open(project.id.clone()),
                MAIN_QUEUE,
            )
            .unwrap();
        assert!(draft.report.is_none() && draft.selected.is_empty() && draft.search.is_empty());
        assert_ne!(draft.receipt, receipt);
        let cancel = CancellationToken::new();
        let (_update, updates) = mpsc::channel();
        let (_sender, result) = mpsc::channel();
        draft.task = Some(Task {
            cancel: cancel.clone(),
            updates,
            result,
        });
        for action in [
            ProjectAction::Open(project.id),
            ProjectAction::New,
            ProjectAction::Save { copy: false },
            ProjectAction::Delete,
        ] {
            assert!(
                draft
                    .project_action(&app.store, action, MAIN_QUEUE)
                    .is_err()
            );
        }
        click(&mut app, &ctx, "Save changes"); // Disabled during the scan.
        assert_eq!(app.store.grabber_projects().unwrap()[0].revision, 1);
        click(&mut app, &ctx, "×");
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn project_save_errors_keep_edits_and_removed_queue_can_be_replaced() {
        let (_temp, mut app, ctx) = fixture();
        let queue = app.store.create_queue("Temporary").unwrap();
        app.queue_id = queue.id.clone();
        app.force_refresh();
        app.open_grabber();
        click(&mut app, &ctx, "Save project");
        assert!(app.grabber.as_ref().unwrap().error.is_some());
        assert!(app.store.grabber_projects().unwrap().is_empty());
        {
            let draft = app.grabber.as_mut().unwrap();
            draft.url = "https://example.test/".into();
            draft.project_name = "Manuals".into();
        }
        click(&mut app, &ctx, "Save project");
        let original = app.grabber.as_ref().unwrap().project.clone().unwrap();
        let current = app
            .store
            .update_grabber_project(&original, "Manuals", &original.settings)
            .unwrap();
        app.grabber.as_mut().unwrap().depth = 5;
        click(&mut app, &ctx, "Save changes");
        assert!(
            app.grabber
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("changed or was deleted")
        );
        assert_eq!(app.grabber.as_ref().unwrap().depth, 5);
        app.store.delete_queue(&queue.id).unwrap();
        app.force_refresh();
        let draft = app.grabber.as_mut().unwrap();
        draft
            .project_action(&app.store, ProjectAction::Open(current.id), MAIN_QUEUE)
            .unwrap();
        assert_eq!(draft.queue, queue.id); // Never silently falls back to Main queue.
        click(&mut app, &ctx, "Save changes");
        assert!(
            app.grabber
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("Queue no longer exists")
        );
        app.grabber.as_mut().unwrap().queue = MAIN_QUEUE.into();
        click(&mut app, &ctx, "Save changes");
        assert!(app.grabber.as_ref().unwrap().error.is_none());
        assert_eq!(
            app.store.grabber_projects().unwrap()[0].settings.queue,
            MAIN_QUEUE
        );
    }

    #[test]
    fn saved_project_reruns_fetch_fresh_previews_and_require_new_selection_before_queueing() {
        use std::io::{Read, Write};
        let (_temp, mut app, ctx) = fixture();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for name in ["first.pdf", "fresh.pdf"] {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "No scan request arrived");
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                // Windows can inherit the listener's nonblocking mode. The
                // request reader below uses blocking I/O with a bounded timeout.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                assert!(request.starts_with(b"GET / HTTP/1.1"));
                let html = format!("<a href='{name}'>Document</a>");
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", html.len(), html).unwrap();
            }
        });
        let queue = app.store.create_queue("Review first").unwrap();
        app.queue_id = queue.id.clone();
        app.force_refresh();
        app.open_grabber();
        app.grabber.as_mut().unwrap().url = url;
        app.grabber.as_mut().unwrap().project_name = "Local manuals".into();
        click(&mut app, &ctx, "Save project");
        click(&mut app, &ctx, "×");
        app.open_grabber();
        click(&mut app, &ctx, "Unsaved scan");
        click(&mut app, &ctx, "Local manuals");
        for (button, name) in [("Scan website", "first.pdf"), ("Scan again", "fresh.pdf")] {
            click(&mut app, &ctx, button);
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.grabber.as_ref().unwrap().task.is_some() {
                app.grabber.as_mut().unwrap().poll();
                assert!(Instant::now() < deadline, "The scan worker did not finish");
                std::thread::sleep(Duration::from_millis(10));
            }
            let draft = app.grabber.as_ref().unwrap();
            assert!(draft.error.is_none(), "{:?}", draft.error);
            assert_eq!(draft.report.as_ref().unwrap().files[0].name, name);
            assert_eq!(draft.selected, [false]);
            click(&mut app, &ctx, "Add selected to queue");
            assert!(app.store.queue_jobs(&queue.id).unwrap().is_empty());
            click(&mut app, &ctx, "Select visible");
        }
        server.join().unwrap();
        click(&mut app, &ctx, "Add selected to queue");
        let jobs = app.store.queue_jobs(&queue.id).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "fresh.pdf");
        assert_eq!(app.store.grabber_projects().unwrap().len(), 1);
    }

    #[test]
    fn grabber_setup_opens_from_menu_and_rejects_invalid_inputs_without_queueing() {
        let (_temp, mut app, ctx) = fixture();
        click(&mut app, &ctx, "Tools");
        click(&mut app, &ctx, "Website grabber");
        assert_eq!(app.dialog, Some(Dialog::Grabber));
        for dark in [false, true] {
            ctx.set_theme(if dark {
                egui::ThemePreference::Dark
            } else {
                egui::ThemePreference::Light
            });
            let size = egui::vec2(860.0, 600.0);
            frame(&mut app, &ctx, size, vec![]);
            let output = frame(&mut app, &ctx, size, vec![]);
            for label in [
                "Starting page",
                "Scan depth",
                "Page limit",
                "Scan website",
                "×",
            ] {
                let rect = text_rect(&output, label);
                assert!(
                    rect.min.y >= 0.0
                        && rect.max.y <= size.y
                        && rect.min.x >= 0.0
                        && rect.max.x <= size.x,
                    "{label}: {rect:?}"
                );
            }
        }
        app.grabber.as_mut().unwrap().url = "file:///C:/private".into();
        click(&mut app, &ctx, "Scan website");
        let draft = app.grabber.as_ref().unwrap();
        assert!(draft.error.is_some() && draft.task.is_none());
        assert!(app.store.queue_jobs(MAIN_QUEUE).unwrap().is_empty());
        click(&mut app, &ctx, "×");
        assert!(app.grabber.is_none());
    }

    #[test]
    fn grabber_preview_requires_selection_filters_and_adds_only_chosen_files() {
        let (_temp, mut app, ctx) = fixture();
        let queue = app.store.create_queue("Website files").unwrap();
        app.queue_id = queue.id.clone();
        app.force_refresh();
        app.open_grabber();
        app.grabber.as_mut().unwrap().receive(report());
        click(&mut app, &ctx, "Add selected to queue"); // Disabled with no selection.
        assert!(app.store.queue_jobs(&queue.id).unwrap().is_empty());
        app.grabber.as_mut().unwrap().search = "assets".into();
        click(&mut app, &ctx, "Select visible");
        app.grabber.as_mut().unwrap().search = "guide".into();
        click(&mut app, &ctx, "Select visible");
        assert_eq!(app.grabber.as_ref().unwrap().selected, [true, true, false]);
        click(&mut app, &ctx, "Clear selection");
        assert_eq!(
            app.grabber.as_ref().unwrap().selected,
            [false, false, false]
        );
        click(&mut app, &ctx, "Select visible");
        click(&mut app, &ctx, "Add selected to queue");
        assert!(app.grabber.is_none() && app.dialog.is_none());
        let jobs = app.store.queue_jobs(&queue.id).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "guide.pdf");
        assert!(
            !app.store
                .queues()
                .unwrap()
                .iter()
                .find(|q| q.id == queue.id)
                .unwrap()
                .running
        );
        app.add_grabbed_files(); // A repeated action cannot submit a cleared preview.
        assert_eq!(app.store.queue_jobs(&queue.id).unwrap().len(), 1);
    }

    #[test]
    fn grabber_missing_queue_keeps_selection_and_allows_another_destination() {
        let (_temp, mut app, ctx) = fixture();
        let queue = app.store.create_queue("Temporary").unwrap();
        let other = app.store.create_queue("Keep files").unwrap();
        app.queue_id = queue.id.clone();
        app.force_refresh();
        app.open_grabber();
        app.grabber.as_mut().unwrap().receive(report());
        click(&mut app, &ctx, "Select visible");
        app.store.delete_queue(&queue.id).unwrap();
        click(&mut app, &ctx, "Add selected to queue");
        let draft = app.grabber.as_mut().unwrap();
        assert!(
            draft
                .error
                .as_deref()
                .unwrap()
                .contains("Queue no longer exists")
        );
        assert!(draft.selected.iter().all(|v| *v));
        draft.queue = other.id.clone();
        draft.destination = 1; // Default downloads, ignoring category folders.
        click(&mut app, &ctx, "Add selected to queue");
        let jobs = app.store.queue_jobs(&other.id).unwrap();
        assert_eq!(jobs.len(), 3);
        assert!(
            jobs.iter()
                .all(|j| j.directory == app.store.paths.downloads)
        );
    }

    #[test]
    fn grabber_closing_cancels_worker_and_late_results_cannot_touch_a_new_preview() {
        let (_temp, mut app, ctx) = fixture();
        app.open_grabber();
        let cancel = CancellationToken::new();
        let (_update, updates) = mpsc::channel();
        let (sender, result) = mpsc::channel();
        app.grabber.as_mut().unwrap().task = Some(Task {
            cancel: cancel.clone(),
            updates,
            result,
        });
        click(&mut app, &ctx, "×");
        assert!(cancel.is_cancelled());
        app.open_grabber();
        assert!(sender.send(Ok(report())).is_err());
        app.grabber.as_mut().unwrap().poll();
        assert!(app.grabber.as_ref().unwrap().report.is_none());
        assert!(app.store.queue_jobs(MAIN_QUEUE).unwrap().is_empty());
    }

    #[test]
    fn grabber_preview_fits_both_themes_and_change_scan_clears_old_selection() {
        let (_temp, mut app, ctx) = fixture();
        app.open_grabber();
        app.grabber.as_mut().unwrap().receive(report());
        for dark in [false, true] {
            ctx.set_theme(if dark {
                egui::ThemePreference::Dark
            } else {
                egui::ThemePreference::Light
            });
            let size = egui::vec2(860.0, 600.0);
            frame(&mut app, &ctx, size, vec![]);
            let output = frame(&mut app, &ctx, size, vec![]);
            for label in [
                "Select visible",
                "Queue",
                "Save to",
                "Add selected to queue",
                "Change scan",
                "Scan again",
                "×",
            ] {
                let rect = text_rect(&output, label);
                assert!(
                    rect.min.y >= 0.0
                        && rect.max.y <= size.y
                        && rect.min.x >= 0.0
                        && rect.max.x <= size.x,
                    "{label}: {rect:?}"
                );
                assert!(output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::epaint::Shape::Text(text)
                        if text.galley.job.text.trim() == label
                            && shape.clip_rect.contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size())))
                }), "{label} is clipped at the minimum window size");
            }
        }
        click(&mut app, &ctx, "Select visible");
        click(&mut app, &ctx, "Change scan");
        let draft = app.grabber.as_ref().unwrap();
        assert!(draft.report.is_none() && draft.selected.is_empty());
        assert!(app.store.queue_jobs(MAIN_QUEUE).unwrap().is_empty());
    }

    #[test]
    fn grabber_background_scan_reaches_preview_without_creating_downloads() {
        use std::io::{Read, Write};
        let (_temp, mut app, ctx) = fixture();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let html = "<a href='actual.pdf'>Actual document</a>";
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", html.len(), html).unwrap();
        });
        app.open_grabber();
        app.grabber.as_mut().unwrap().url = url;
        click(&mut app, &ctx, "Scan website");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.grabber.as_mut().unwrap().poll();
            if app.grabber.as_ref().unwrap().task.is_none() {
                break;
            }
            assert!(Instant::now() < deadline, "The worker did not finish");
            std::thread::sleep(Duration::from_millis(10));
        }
        server.join().unwrap();
        let draft = app.grabber.as_ref().unwrap();
        assert!(draft.error.is_none(), "{:?}", draft.error);
        assert_eq!(draft.report.as_ref().unwrap().files[0].name, "actual.pdf");
        assert_eq!(draft.selected, [false]);
        assert!(app.store.queue_jobs(MAIN_QUEUE).unwrap().is_empty());
    }
}
