//! Non-modal per-download windows. Queue selection never owns their snapshots.
use super::*;
use download_engine::{
    completion::{self, PowerStates},
    transfer_activity::{Phase, SegmentSnapshot, SegmentState},
};
use egui::{Align, Color32, Layout, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use std::collections::{BTreeMap, HashSet};

mod media;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Status,
    SpeedLimiter,
    Completion,
}

/// Plain-language connection states for the window; the engine keeps its own names.
pub(super) fn segment_label(state: SegmentState) -> &'static str {
    match state {
        SegmentState::SendingGet => tr("Connecting"),
        SegmentState::ReceivingData => tr("Downloading"),
        other => tr(other.label()),
    }
}

#[derive(Clone, Debug)]
struct ConnectionRow {
    number: usize,
    start: u64,
    end: u64,
    downloaded: u64,
    info: String,
}

#[derive(Default)]
pub(super) struct State {
    windows: BTreeMap<String, Window>,
    pending: HashSet<String>,
    auto_close_on_complete: bool,
}

struct Window {
    job: Option<Job>,
    queue_id: String,
    queue: String,
    queue_running: bool,
    phase: Option<Phase>,
    live_segments: Vec<SegmentSnapshot>,
    live_media: Option<download_engine::transfer_activity::MediaSnapshot>,
    live_media_parts: Vec<download_engine::transfer_activity::MediaSnapshot>,
    disk_segments: Vec<u64>,
    error: Option<String>,
    notice: Option<String>,
    sampled: Instant,
    rate: f64,
    details: bool,
    focus: bool,
    waiting: Option<Status>,
    tab: Tab,
    speed_initialized: bool,
    file_speed: SpeedField,
    global_speed: SpeedField,
    speed_request: Option<Uuid>,
    close_on_complete: bool,
    open_file_on_complete: bool,
    open_folder_on_complete: bool,
}

impl Window {
    fn new(job: Job) -> Self {
        Self {
            job: Some(job),
            queue_id: MAIN_QUEUE.into(),
            queue: String::new(),
            queue_running: false,
            phase: None,
            live_segments: Vec::new(),
            live_media: None,
            live_media_parts: Vec::new(),
            disk_segments: Vec::new(),
            error: None,
            notice: None,
            sampled: Instant::now(),
            rate: 0.0,
            details: true,
            focus: true,
            waiting: None,
            tab: Tab::Status,
            speed_initialized: false,
            file_speed: SpeedField::new(None),
            global_speed: SpeedField::new(None),
            speed_request: None,
            close_on_complete: false,
            open_file_on_complete: false,
            open_folder_on_complete: false,
        }
    }

    fn refresh(&mut self, store: &Store) -> Option<Action> {
        let Some(previous) = &self.job else {
            return None;
        };
        let was_active = matches!(previous.status, Status::Downloading | Status::Queued);
        let result = (|| -> anyhow::Result<_> {
            let job = store.get(&previous.id)?;
            let queue_id = store.job_queue(&previous.id)?;
            let queues = store.queues()?;
            let queue = queues
                .into_iter()
                .find(|q| Some(&q.id) == queue_id.as_ref());
            let running = queue.as_ref().is_some_and(|q| q.running)
                || store.immediate_downloads()?.contains(&previous.id);
            let limits = store.bandwidth_limits()?;
            Ok((
                job,
                queue_id.unwrap_or_else(|| MAIN_QUEUE.into()),
                queue.map(|q| q.name).unwrap_or_default(),
                running,
                limits,
            ))
        })();
        match result {
            Ok((mut job, queue_id, queue, running, limits)) => {
                self.phase = store.activity.get(&previous.id);
                self.live_segments = store.activity.segments(&previous.id);
                self.live_media_parts = store.activity.media_parts(&previous.id);
                self.live_media = self.live_media_parts.first().copied();
                if let Some(ref mut current) = job
                    && current.status == Status::Downloading
                    && let Some(download) = &current.hls
                {
                    let live: u64 = self
                        .live_media_parts
                        .iter()
                        .filter(|part| download.saved_part(part.index).is_none())
                        .map(|part| part.downloaded)
                        .sum();
                    current.downloaded = current.downloaded.max(download.saved_bytes() + live);
                }
                if let Some(ref mut current) = job
                    && current.status == Status::Downloading
                    && !self.live_segments.is_empty()
                {
                    let live_sum: u64 = self.live_segments.iter().map(|s| s.downloaded).sum();
                    current.downloaded = current.downloaded.max(live_sum);
                }
                if let Some(ref current) = job
                    && let Some(ref plan) = current.plan
                    && self.live_segments.is_empty()
                {
                    if (self.disk_segments.len() != plan.segments.len()
                        || current.downloaded != previous.downloaded)
                        && let Ok(root) = store.paths.parts(&current.id)
                    {
                        self.disk_segments = (0..plan.segments.len())
                            .map(|i| {
                                std::fs::metadata(root.join(format!("segment-{i}.part")))
                                    .map(|m| m.len())
                                    .unwrap_or(0)
                            })
                            .collect();
                    }
                } else {
                    self.disk_segments.clear();
                }
                if !self.speed_initialized && self.speed_request.is_none() {
                    self.file_speed = SpeedField::new(limits.downloads.get(&previous.id).copied());
                    self.global_speed = SpeedField::new(limits.global);
                    self.speed_initialized = true;
                }
                let seconds = self.sampled.elapsed().as_secs_f64().max(0.001);
                self.rate = job
                    .as_ref()
                    .filter(|job| {
                        job.status == Status::Downloading
                            && previous.status == Status::Downloading
                            && self.phase.is_none_or(|phase| phase == Phase::Downloading)
                    })
                    .map_or(0.0, |job| {
                        job.downloaded.saturating_sub(previous.downloaded) as f64 / seconds
                    });
                self.sampled = Instant::now();
                if job.as_ref().is_none_or(|job| {
                    self.waiting == Some(job.status)
                        || (self.waiting == Some(Status::Downloading)
                            && job.status == Status::Queued)
                        || matches!(job.status, Status::Complete | Status::Failed)
                }) {
                    self.waiting = None;
                }
                let finished_now = was_active
                    && job
                        .as_ref()
                        .is_some_and(|current| current.status == Status::Complete);
                self.job = job;
                self.queue_id = queue_id;
                self.queue = queue;
                self.queue_running = running;
                self.error = None;
                if finished_now {
                    if self.open_file_on_complete {
                        return Some(Action::OpenFile);
                    }
                    if self.open_folder_on_complete {
                        return Some(Action::OpenFolder);
                    }
                    if self.close_on_complete {
                        return Some(Action::Hide);
                    }
                }
                None
            }
            Err(error) => {
                self.error = Some(tf!(
                    "Cannot refresh download status: {error}",
                    error = error
                ));
                self.rate = 0.0;
                None
            }
        }
    }

    fn connection_rows(&self, job: &Job) -> Vec<ConnectionRow> {
        if !self.live_segments.is_empty() {
            return self
                .live_segments
                .iter()
                .enumerate()
                .map(|(idx, seg)| ConnectionRow {
                    number: idx + 1,
                    start: seg.start,
                    end: seg.end,
                    downloaded: seg.downloaded,
                    info: segment_label(seg.state).into(),
                })
                .collect();
        }
        if let Some(plan) = &job.plan
            && !plan.segments.is_empty()
        {
            let mut indexed: Vec<_> = plan.segments.iter().enumerate().collect();
            indexed.sort_unstable_by_key(|(_, seg)| seg.start);
            let disk_sum: u64 = self.disk_segments.iter().sum();
            let mut remaining_fallback = job.downloaded;
            return indexed
                .into_iter()
                .enumerate()
                .map(|(pos, (orig_idx, seg))| {
                    let cap = seg.end.saturating_sub(seg.start) + 1;
                    let downloaded = if job.status == Status::Complete {
                        cap
                    } else if disk_sum > 0 {
                        self.disk_segments
                            .get(orig_idx)
                            .copied()
                            .unwrap_or(0)
                            .min(cap)
                    } else {
                        let share = remaining_fallback.min(cap);
                        remaining_fallback = remaining_fallback.saturating_sub(share);
                        share
                    };
                    let info = if downloaded >= cap || job.status == Status::Complete {
                        tr("Completed").into()
                    } else if job.status == Status::Downloading {
                        self.phase
                            .map(|phase| {
                                if phase == Phase::Downloading {
                                    tr("Downloading")
                                } else {
                                    tr(phase.label())
                                }
                            })
                            .unwrap_or(tr("Downloading"))
                            .into()
                    } else if job.status == Status::Paused {
                        tr("Paused").into()
                    } else if job.status == Status::Failed {
                        tr("Disconnected").into()
                    } else {
                        tr("Queued").into()
                    };
                    ConnectionRow {
                        number: pos + 1,
                        start: seg.start,
                        end: seg.end,
                        downloaded,
                        info,
                    }
                })
                .collect();
        }
        let total = job.total.unwrap_or_else(|| job.downloaded.max(1));
        let info = match job.status {
            Status::Complete => tr("Completed").into(),
            Status::Downloading => self
                .phase
                .map(|phase| {
                    if phase == Phase::Downloading {
                        tr("Downloading")
                    } else {
                        tr(phase.label())
                    }
                })
                .unwrap_or(tr("Downloading"))
                .into(),
            Status::Paused => tr("Paused").into(),
            Status::Failed => tr("Disconnected").into(),
            Status::Queued => tr("Waiting in queue").into(),
        };
        vec![ConnectionRow {
            number: 1,
            start: 0,
            end: total.saturating_sub(1),
            downloaded: job.downloaded,
            info,
        }]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Hide,
    Pause,
    Start,
    OpenFile,
    OpenWith,
    OpenFolder,
    ApplySpeed,
    SetQueueCompletion(Option<completion::Action>),
}

impl Desktop {
    pub(super) fn open_download_status(&mut self, job: &Job) {
        self.download_status.pending.remove(&job.id);
        let auto_close = self.download_status.auto_close_on_complete;
        let window = self
            .download_status
            .windows
            .entry(job.id.clone())
            .or_insert_with(|| {
                let mut window = Window::new(job.clone());
                window.close_on_complete = auto_close;
                window
            });
        window.focus = true;
        let _ = window.refresh(&self.store);
    }

    /// Called only for an explicitly started single file. Existing jobs, batches,
    /// CLI commands, scripts and queue starts do not open windows implicitly.
    pub(super) fn expect_download_status(&mut self, id: &str) {
        if self.interface.auto_download_status {
            self.download_status.pending.insert(id.into());
        }
    }

    pub(super) fn expect_added_status(&mut self, id: &str) {
        let result = (|| -> anyhow::Result<bool> {
            let queue = self.store.job_queue(id)?;
            Ok(self
                .store
                .queues()?
                .iter()
                .any(|q| Some(&q.id) == queue.as_ref() && q.running)
                || self
                    .store
                    .immediate_downloads()?
                    .iter()
                    .any(|job| job == id))
        })();
        match result {
            Ok(true) => self.expect_download_status(id),
            Ok(false) => {}
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot open download status: {error}",
                    error = error
                )))
            }
        }
    }

    pub(super) fn cancel_status_request(&mut self, command: &Command) {
        match command {
            Command::Pause(id) | Command::Remove(id) => {
                self.download_status.pending.remove(id);
            }
            Command::PauseAll | Command::PauseQueue(_) => self.download_status.pending.clear(),
            _ => {}
        }
    }

    pub(super) fn stop_selected_downloads(&mut self) {
        for job in self.selected_jobs() {
            if !matches!(job.status, Status::Downloading | Status::Queued) {
                continue;
            }
            match self.scheduler.send(Command::Pause(job.id.clone())) {
                Ok(()) => {
                    self.download_status.pending.remove(&job.id);
                    self.download_status.windows.remove(&job.id);
                }
                Err(error) => self.notice = Some(Notice::error(error.to_string())),
            }
        }
    }

    pub(super) fn stop_all_downloads(&mut self) {
        // Individual pauses also stop immediate downloads and clear automatic
        // queue-resume ownership. Command::Stop would shut down the scheduler.
        match self.scheduler.send(Command::PauseAll) {
            Ok(()) => {
                self.download_status.pending.clear();
                self.download_status.windows.retain(|_, window| {
                    window
                        .job
                        .as_ref()
                        .is_some_and(|job| matches!(job.status, Status::Complete | Status::Failed))
                });
            }
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
    }

    pub(super) fn refresh_download_status(&mut self) {
        match self.store.take_download_status_requests() {
            Ok(ids) => {
                for id in ids {
                    self.expect_added_status(&id);
                }
            }
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot read browser status requests: {error}",
                    error = error
                )))
            }
        }
        if !self.interface.auto_download_status {
            self.download_status.pending.clear();
        }
        let pending: Vec<_> = self.download_status.pending.iter().cloned().collect();
        for id in pending {
            match self.store.get(&id) {
                Ok(Some(job))
                    if job.status == Status::Complete
                        && self.download_status.auto_close_on_complete =>
                {
                    self.download_status.pending.remove(&id);
                }
                Ok(Some(job))
                    if matches!(
                        job.status,
                        Status::Downloading | Status::Complete | Status::Failed
                    ) =>
                {
                    self.open_download_status(&job);
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    self.download_status.pending.remove(&id);
                }
                Err(error) => {
                    self.download_status.pending.remove(&id);
                    self.notice = Some(Notice::error(tf!(
                        "Cannot open download status: {error}",
                        error = error
                    )));
                }
            }
        }
        let mut auto_actions = Vec::new();
        for (id, window) in &mut self.download_status.windows {
            if let Some(action) = window.refresh(&self.store) {
                auto_actions.push((id.clone(), action));
            }
        }
        for (id, action) in auto_actions {
            if let Some(mut window) = self.download_status.windows.remove(&id)
                && self.status_action(&id, &mut window, Some(action))
            {
                self.download_status.windows.insert(id, window);
            }
        }
    }

    pub(super) fn status_command_failed(&mut self, error: &str) {
        self.download_status.pending.clear();
        for window in self.download_status.windows.values_mut() {
            if window.waiting.take().is_some() {
                window.notice = Some(error.into());
            }
        }
    }

    pub(super) fn status_bandwidth_event(&mut self, event: &bandwidth::Event) -> bool {
        for window in self.download_status.windows.values_mut() {
            if window.speed_request == Some(event.request) {
                window.speed_request = None;
                match &event.result {
                    Ok(limits) => {
                        self.limits = limits.clone();
                        window.notice = Some("Speed limits saved and applied.".into());
                    }
                    Err(error) => {
                        window.notice = Some(error.clone());
                    }
                }
                return true;
            }
        }
        false
    }

    pub(super) fn download_status_windows(&mut self, ctx: &egui::Context) {
        let language = self.interface.language;
        let _language = download_engine::i18n::scope(language);
        static ICON: std::sync::LazyLock<std::sync::Arc<egui::IconData>> =
            std::sync::LazyLock::new(|| std::sync::Arc::new(crate::tray::icon()));
        #[cfg(feature = "screenshot")]
        if std::env::var("IDM_EX_TEST_FEATURE")
            .is_ok_and(|mode| mode.starts_with("status-") && !mode.starts_with("status-native"))
        {
            ctx.set_embed_viewports(true);
        }
        let mut windows = std::mem::take(&mut self.download_status.windows);
        windows.retain(|id, window| {
            let viewport = egui::ViewportId::from_hash_of(("download-status", id));
            let mut action = None;
            let current_watch = self.extras.watches.get(&window.queue_id).map(|w| w.action);
            let power_states = self.extras.power_states;
            let is_complete = window
                .job
                .as_ref()
                .is_some_and(|j| j.status == Status::Complete)
                && window.phase.is_none();
            let title_text = if is_complete {
                tr("Download complete — Download Engine").into()
            } else {
                window
                    .job
                    .as_ref()
                    .map(|j| {
                        if let Some(pct) = progress(j) {
                            format!("{:.0}% {} — Download Engine", pct * 100.0, j.name)
                        } else {
                            format!("{} — Download Engine", j.name)
                        }
                    })
                    .unwrap_or_else(|| tr("Download status — Removed download").into())
            };
            let initial_size = if is_complete {
                [520.0, 310.0]
            } else {
                [702.0, 495.0]
            };
            let prev_close_on_complete = window.close_on_complete;
            ctx.show_viewport_immediate(
                viewport,
                egui::ViewportBuilder::default()
                    .with_title(title_text)
                    .with_inner_size(initial_size)
                    .with_min_inner_size([400.0, 290.0])
                    .with_decorations(false)
                    .with_resizable(true)
                    .with_icon(ICON.clone()),
                |ui, class| {
                    let _language = download_engine::i18n::scope(language);
                    if std::mem::take(&mut window.focus)
                        && class != egui::ViewportClass::EmbeddedWindow
                    {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    let had_input = ui.ctx().input(|i| !i.events.is_empty());
                    let prev_tab = window.tab;
                    let prev_details = window.details;
                    let p = Palette::new(ui.visuals().dark_mode);
                    if class == egui::ViewportClass::EmbeddedWindow {
                        form_dialog::frame(p).show(ui, |ui| {
                            action = window.body(ui, p, false, current_watch, power_states)
                        });
                    } else {
                        egui::CentralPanel::default()
                            .frame(form_dialog::frame(p))
                            .show(ui, |ui| {
                                action = window.body(ui, p, true, current_watch, power_states)
                            });
                    }
                    if ui.ctx().input(|i| i.viewport().close_requested())
                        || ui
                            .ctx()
                            .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                    {
                        action = Some(Action::Hide);
                    }
                    if had_input
                        || window.tab != prev_tab
                        || window.details != prev_details
                        || action.is_some()
                    {
                        ui.ctx().request_repaint();
                        ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                    } else {
                        ui.ctx().request_repaint_after(Duration::from_millis(250));
                    }
                    #[cfg(feature = "screenshot")]
                    if std::env::var("IDM_EX_TEST_FEATURE")
                        .is_ok_and(|mode| mode.starts_with("status-native"))
                    {
                        self.capture_native_status(ui);
                    }
                },
            );
            if window.close_on_complete != prev_close_on_complete {
                self.download_status.auto_close_on_complete = window.close_on_complete;
            }
            let keep = self.status_action(id, window, action);
            if action == Some(Action::OpenFile) && self.dialog.is_some() {
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::Visible(true),
                );
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::Minimized(false),
                );
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            }
            keep
        });
        self.download_status.windows = windows;
    }

    fn status_action(&mut self, id: &str, window: &mut Window, action: Option<Action>) -> bool {
        if action == Some(Action::Hide) {
            self.download_status.pending.remove(id);
            return false;
        }
        let Some(job) = window.job.clone().filter(|_| window.error.is_none()) else {
            return true;
        };
        if action.is_some() {
            window.notice = None;
        }
        let old_notice = if matches!(
            action,
            Some(Action::OpenFile | Action::OpenWith | Action::OpenFolder)
        ) {
            self.notice.take()
        } else {
            None
        };
        let command = match action {
            Some(Action::Pause) => Some((Command::Pause(id.into()), Status::Paused)),
            Some(Action::Start) => {
                Some((Command::StartImmediately(id.into()), Status::Downloading))
            }
            Some(Action::OpenFile) => {
                self.open_file(&job);
                None
            }
            Some(Action::OpenWith) => {
                self.open_file_with(&job);
                None
            }
            Some(Action::OpenFolder) => {
                self.show_folder(&job);
                None
            }
            Some(Action::ApplySpeed) => {
                let result = (|| -> anyhow::Result<()> {
                    let global = window.global_speed.rate()?;
                    let download = Some((id.to_owned(), window.file_speed.rate()?));
                    let request = Uuid::new_v4();
                    self.scheduler.send(Command::Bandwidth(bandwidth::Update {
                        request,
                        global,
                        download,
                    }))?;
                    window.speed_request = Some(request);
                    Ok(())
                })();
                if let Err(error) = result {
                    window.notice = Some(error.to_string());
                }
                None
            }
            Some(Action::SetQueueCompletion(choice)) => {
                let queue_name = if window.queue.is_empty() {
                    tr("Main queue").into()
                } else {
                    window.queue.clone()
                };
                if let Err(error) = self.arm_completion(&window.queue_id, queue_name, choice) {
                    window.notice = Some(error.to_string());
                }
                None
            }
            None | Some(Action::Hide) => None,
        };
        if let Some((command, status)) = command {
            match self.scheduler.send(command) {
                Ok(()) => {
                    window.waiting = Some(status);
                }
                Err(error) => window.notice = Some(error.to_string()),
            }
        }
        if matches!(
            action,
            Some(Action::OpenFile | Action::OpenWith | Action::OpenFolder)
        ) {
            if let Some(notice) = self.notice.take() {
                window.notice = Some(notice.message);
            }
            self.notice = old_notice;
        }
        true
    }
}

impl Window {
    fn body(
        &mut self,
        ui: &mut egui::Ui,
        p: Palette,
        native: bool,
        current_watch: Option<completion::Action>,
        power_states: fn() -> PowerStates,
    ) -> Option<Action> {
        if let Some(job) = self.job.clone()
            && job.status == Status::Complete
            && self.phase.is_none()
        {
            return self.draw_complete_window(ui, &job, p, native);
        }

        let mut action = None;
        let top = ui.cursor().min;
        // The full name; the shared header ends it with … when the window is narrow.
        // Progress stays in the summary below and in the native window title.
        let title_str = self
            .job
            .as_ref()
            .map(|job| job.name.clone())
            .unwrap_or_else(|| tr("Removed download").into());
        if form_dialog::title(ui, &title_str, form_dialog::Icon::Download, p, true) {
            action = Some(Action::Hide);
        }
        ui.spacing_mut().interact_size.y = 22.0;
        if native {
            let drag = egui::Rect::from_min_max(
                top,
                egui::pos2(ui.max_rect().right() - 40.0, top.y + 40.0),
            );
            if ui
                .interact(drag, ui.id().with("drag-status"), egui::Sense::drag())
                .drag_started()
            {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        }

        // Top tab strip: Status | Speed limit | When done
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (tab, label) in [
                (Tab::Status, tr("Status")),
                (Tab::SpeedLimiter, tr("Speed limit")),
                (Tab::Completion, tr("When done")),
            ] {
                let selected = self.tab == tab;
                let response = ui.add(
                    egui::Button::new(RichText::new(label).size(11.5).color(if selected {
                        p.text
                    } else {
                        p.muted
                    }))
                    .fill(if selected {
                        p.selected
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::new(
                        1.0,
                        if selected {
                            p.border
                        } else {
                            Color32::TRANSPARENT
                        },
                    ))
                    .corner_radius(6),
                );
                let activate = response.clicked()
                    || (response.hovered() && ui.input(|i| i.pointer.primary_pressed()));
                if activate && self.tab != tab {
                    self.tab = tab;
                    ui.ctx().request_repaint();
                    ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                }
                if self.tab == tab {
                    ui.painter().line_segment(
                        [
                            pos2(response.rect.left() + 6.0, response.rect.bottom()),
                            pos2(response.rect.right() - 6.0, response.rect.bottom()),
                        ],
                        Stroke::new(2.0, p.accent),
                    );
                }
            }
        });
        ui.add_space(2.0);

        let footer_reserve = if self.tab == Tab::Status { 4.0 } else { 42.0 };
        let height = (ui.available_height() - footer_reserve).max(110.0);
        egui::ScrollArea::vertical()
            .id_salt(("status-body", self.tab as u8))
            .auto_shrink([false, true])
            .max_height(height)
            .show(ui, |ui| {
                if let Some(error) = &self.error {
                    ui.colored_label(p.warning, error);
                }
                if let Some(notice) = &self.notice {
                    let is_ok = notice.contains("saved and applied");
                    ui.colored_label(if is_ok { p.success } else { p.warning }, tr(notice));
                }
                let Some(job) = self.job.clone() else {
                    ui.label(tr("This download has been removed from history."));
                    return;
                };
                match self.tab {
                    Tab::Status => {
                        if let Some(next) = self.draw_status_tab(ui, &job, p) {
                            action = Some(next);
                        }
                    }
                    Tab::SpeedLimiter => {
                        if self.draw_speed_tab(ui, &job, p) {
                            action = Some(Action::ApplySpeed);
                        }
                    }
                    Tab::Completion => {
                        if let Some(next_watch) =
                            self.draw_completion_tab(ui, p, current_watch, power_states())
                        {
                            action = Some(Action::SetQueueCompletion(next_watch));
                        }
                    }
                }
            });

        if self.tab != Tab::Status
            && let Some(job) = &self.job
        {
            form_dialog::controls(ui, |ui| {
                let enabled = self.error.is_none() && self.waiting.is_none();
                let (label, next) = match job.status {
                    Status::Downloading => (tr("Pause"), Action::Pause),
                    Status::Queued => (tr("Start immediately"), Action::Start),
                    _ => (resume_action(job), Action::Start),
                };
                if form_dialog::secondary_enabled(ui, enabled, label).clicked() {
                    action = Some(next);
                }
            });
        }
        action
    }

    fn draw_complete_window(
        &mut self,
        ui: &mut egui::Ui,
        job: &Job,
        p: Palette,
        native: bool,
    ) -> Option<Action> {
        let mut action = None;
        let top = ui.cursor().min;
        if form_dialog::title(
            ui,
            tr("Download complete"),
            form_dialog::Icon::Download,
            p,
            true,
        ) {
            action = Some(Action::Hide);
        }
        ui.spacing_mut().interact_size.y = 20.0;
        if native {
            let drag = egui::Rect::from_min_max(
                top,
                egui::pos2(ui.max_rect().right() - 40.0, top.y + 40.0),
            );
            if ui
                .interact(
                    drag,
                    ui.id().with("drag-status-complete"),
                    egui::Sense::drag(),
                )
                .drag_started()
            {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(p.warning, error);
        }
        if let Some(notice) = &self.notice {
            ui.colored_label(p.warning, notice);
        }

        let size_bytes = job.total.unwrap_or(job.downloaded).max(job.downloaded);
        egui::Frame::new()
            .fill(p.background)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (badge, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                    ui.painter()
                        .rect_filled(badge, 6, p.accent.gamma_multiply(0.16));
                    ui.painter().rect_stroke(
                        badge,
                        6,
                        Stroke::new(1.0, p.accent.gamma_multiply(0.35)),
                        StrokeKind::Inside,
                    );
                    let c = badge.center();
                    ui.painter().add(egui::Shape::line(
                        vec![
                            c + vec2(-5.5, 0.5),
                            c + vec2(-1.5, 4.5),
                            c + vec2(6.0, -4.0),
                        ],
                        Stroke::new(2.2, p.accent),
                    ));
                    ui.add_space(4.0);
                    // The window title already says it is complete; show the file instead.
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.add(
                            egui::Label::new(RichText::new(&job.name).size(12.5).strong())
                                .truncate(),
                        )
                        .on_hover_text(&job.name);
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                bytes(size_bytes),
                                exact_bytes(size_bytes)
                            ))
                            .size(11.0)
                            .color(p.muted),
                        );
                    });
                });
            });

        ui.add_space(5.0);
        ui.label(RichText::new(tr("Source")).size(11.0).color(p.muted));
        egui::Frame::new()
            .fill(p.background)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(5)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(&job.url).size(11.0)).truncate())
                    .on_hover_text(&job.url);
            });

        ui.add_space(4.0);
        ui.label(RichText::new(tr("Saved to")).size(11.0).color(p.muted));
        let saved_as = job
            .saved_path
            .clone()
            .unwrap_or_else(|| job.directory.join(&job.name))
            .to_string_lossy()
            .into_owned();
        egui::Frame::new()
            .fill(p.background)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(5)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(&saved_as).size(11.0)).truncate())
                    .on_hover_text(&saved_as);
            });

        ui.add_space(6.0);
        let enabled = self.error.is_none() && self.waiting.is_none();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if form_dialog::primary_enabled(ui, enabled, tr("Open")).clicked() {
                action = Some(Action::OpenFile);
            }
            if form_dialog::secondary_enabled(ui, enabled, tr("Open with…")).clicked() {
                action = Some(Action::OpenWith);
            }
            if form_dialog::secondary_enabled(ui, enabled, tr("Open folder")).clicked() {
                action = Some(Action::OpenFolder);
            }
        });

        ui.add_space(3.0);
        ui.square_checkbox(
            &mut self.close_on_complete,
            tr("Don't show this dialog again"),
        );
        action
    }

    fn draw_status_tab(&mut self, ui: &mut egui::Ui, job: &Job, p: Palette) -> Option<Action> {
        let mut action = None;
        let status_text = if self.waiting == Some(Status::Paused) {
            tr("Pausing…").into()
        } else if self.waiting == Some(Status::Downloading) {
            tr("Starting…").into()
        } else if job.status == Status::Downloading {
            self.phase
                .map(|phase| {
                    if phase == Phase::Downloading {
                        tr("Downloading").into()
                    } else {
                        tr(phase.label()).to_owned()
                    }
                })
                .unwrap_or_else(|| {
                    status_label(
                        job,
                        self.queue_running,
                        chrono::Utc::now().timestamp_millis(),
                    )
                })
        } else {
            status_label(
                job,
                self.queue_running,
                chrono::Utc::now().timestamp_millis(),
            )
        };

        let pct_badge = progress(job)
            .map(|v| format!("{:.0}%", v * 100.0))
            .unwrap_or_else(|| tr("Size unknown").into());

        // Main summary card
        egui::Frame::new()
            .fill(p.background)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    let url_width = (ui.available_width() - 120.0).max(80.0);
                    ui.allocate_ui_with_layout(
                        vec2(url_width, 15.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.set_max_width(url_width);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&job.url).size(11.0).color(p.text).strong(),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&job.url);
                        },
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(pct_badge).size(13.5).strong().color(p.accent));
                    });
                });
                ui.add_space(1.0);
                let file_size_value = job
                    .total
                    .map(bytes)
                    .unwrap_or_else(|| tr("Size unknown").into());
                let downloaded_value = job
                    .total
                    .map(|n| {
                        let pct = (job.downloaded as f64 / n.max(1) as f64) * 100.0;
                        tf!("{} ({pct:.2}%)", bytes(job.downloaded), pct = pct)
                    })
                    .unwrap_or_else(|| bytes(job.downloaded));
                let rate_value = if job.status == Status::Downloading && self.rate > 0.0 {
                    tf!("{}/sec", bytes(self.rate as u64))
                } else {
                    "—".into()
                };
                let resume_cap = if job.plan.is_some() || job.hls.is_some() || job.ftp.is_some() {
                    (tr("Supported"), p.accent)
                } else if job.total.is_some() && job.downloaded > 0 {
                    (tr("Starts from the beginning"), p.warning)
                } else {
                    (tr("Checked when downloading"), p.text)
                };

                ui.spacing_mut().interact_size.y = 15.0;
                egui::Grid::new("status-summary-grid")
                    .num_columns(2)
                    .min_row_height(15.0)
                    .spacing([14.0, 1.5])
                    .show(ui, |ui| {
                        ui.label(RichText::new(tr("Status")).size(11.0).color(p.muted));
                        ui.label(
                            RichText::new(&status_text)
                                .size(11.0)
                                .strong()
                                .color(p.status(job.status)),
                        );
                        ui.end_row();

                        ui.label(RichText::new(tr("Size")).size(11.0).color(p.muted));
                        ui.label(RichText::new(&file_size_value).size(11.0));
                        ui.end_row();

                        ui.label(RichText::new(tr("Downloaded")).size(11.0).color(p.muted));
                        ui.label(RichText::new(&downloaded_value).size(11.0));
                        ui.end_row();

                        ui.label(RichText::new(tr("Speed")).size(11.0).color(p.muted));
                        ui.label(RichText::new(&rate_value).size(11.0));
                        ui.end_row();

                        ui.label(RichText::new(tr("Time left")).size(11.0).color(p.muted));
                        ui.label(RichText::new(eta(job, self.rate)).size(11.0));
                        ui.end_row();

                        ui.label(RichText::new(tr("Resume")).size(11.0).color(p.muted));
                        ui.label(RichText::new(resume_cap.0).size(11.0).color(resume_cap.1))
                            .on_hover_text(resume_description(job));
                        ui.end_row();
                    });
            });

        ui.add_space(4.0);
        ui.scope(|ui| {
            ui.visuals_mut().extreme_bg_color = p.track;
            ui.add(
                egui::ProgressBar::new(progress(job).unwrap_or(0.0))
                    .desired_height(8.0)
                    .fill(p.accent),
            );
        });

        // Action bar right below the progress bar:
        // Left: Hide details / Show details with an arrow
        // Right: Pause (or Resume / Start). The header × hides the window.
        ui.add_space(4.0);
        let enabled = self.error.is_none() && self.waiting.is_none();
        ui.horizontal(|ui| {
            ui.spacing_mut().interact_size.y = 28.0;
            if details_toggle(ui, self.details).clicked() {
                self.details = !self.details;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (label, next) = match job.status {
                    Status::Downloading => (tr("Pause"), Action::Pause),
                    Status::Queued => (tr("Start immediately"), Action::Start),
                    _ => (resume_action(job), Action::Start),
                };
                if form_dialog::secondary_enabled(ui, enabled, label).clicked() {
                    action = Some(next);
                }
            });
        });

        if let Some(error) = &job.error {
            ui.add_space(2.0);
            if let Some(retry) = retry_description(job, self.queue_running) {
                ui.colored_label(p.blue, retry);
            }
            ui.colored_label(p.warning, error);
        }

        if self.details {
            ui.add_space(4.0);
            if let Some(download) = &job.hls {
                self.draw_media_details(ui, job, download, p);
                return action;
            }
            let rows = self.connection_rows(job);
            ui.label(
                RichText::new(format!("{} · {}", tr("Connections"), rows.len()))
                    .size(11.0)
                    .color(p.muted),
            );
            ui.add_space(2.0);
            draw_connection_bar(ui, job, &rows, p);
            ui.add_space(4.0);
            draw_connection_table(ui, &rows, p);
        }
        action
    }

    fn draw_speed_tab(&mut self, ui: &mut egui::Ui, job: &Job, p: Palette) -> bool {
        let saving = self.speed_request.is_some();
        let mut apply = false;
        ui.spacing_mut().interact_size.y = 18.0;
        let labels = [tr("Limit speed for this download"), tr("Limit total speed")];
        let label_width = view::checkbox_column(ui, &labels);
        ui.add_enabled_ui(!saving, |ui| {
            egui::Frame::new()
                .fill(p.background)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(&job.name).strong()).truncate())
                        .on_hover_text(&job.name);
                    ui.add_space(3.0);
                    ui.push_id("status-file-speed", |ui| {
                        self.file_speed.draw(ui, labels[0], label_width);
                    });
                    ui.add_space(3.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(tr("Presets:")).small().color(p.muted));
                        for (label, kib) in [
                            ("256 KiB/s", "256"),
                            ("1 MiB/s", "1024"),
                            ("5 MiB/s", "5120"),
                            ("10 MiB/s", "10240"),
                        ] {
                            if ui.small_button(label).clicked() {
                                self.file_speed.enabled = true;
                                self.file_speed.kib = kib.into();
                            }
                        }
                    });
                });
            ui.add_space(5.0);
            egui::Frame::new()
                .fill(p.background)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.strong(tr("All downloads and queues"));
                    ui.add_space(3.0);
                    ui.push_id("status-global-speed", |ui| {
                        self.global_speed.draw(ui, labels[1], label_width);
                    });
                });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if form_dialog::secondary_enabled(ui, !saving, tr("Apply speed limits")).clicked() {
                    apply = true;
                }
                if saving {
                    ui.spinner();
                }
                ui.label(
                    RichText::new(tr("Applies immediately · 1,024 KiB/s = 1 MiB/s"))
                        .small()
                        .color(p.muted),
                );
            });
        });
        apply
    }

    fn draw_completion_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: Palette,
        current_watch: Option<completion::Action>,
        power: PowerStates,
    ) -> Option<Option<completion::Action>> {
        let mut next_watch = None;
        ui.spacing_mut().interact_size.y = 18.0;
        ui.spacing_mut().item_spacing.y = 2.0;
        egui::Frame::new()
            .fill(p.background)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.strong(tr("When this download finishes"));
                ui.add_space(2.0);
                ui.square_checkbox(
                    &mut self.close_on_complete,
                    tr("Close this status window automatically"),
                );
                ui.square_checkbox(
                    &mut self.open_file_on_complete,
                    tr("Open file when download finishes"),
                );
                ui.square_checkbox(
                    &mut self.open_folder_on_complete,
                    tr("Open folder when download finishes"),
                );
            });
        ui.add_space(5.0);
        egui::Frame::new()
            .fill(p.background)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let queue_title = if self.queue.is_empty() {
                    tr("When all downloads in queue finish").to_owned()
                } else {
                    tf!("When all downloads in {} finish", self.queue)
                };
                ui.strong(queue_title);
                ui.label(
                    RichText::new(tr("Applies once for this session when every unfinished file in the queue completes."))
                        .small()
                        .color(p.muted),
                );
                ui.add_space(2.0);
                if ui
                    .radio(current_watch.is_none(), tr("Do nothing"))
                    .clicked()
                    && current_watch.is_some()
                {
                    next_watch = Some(None);
                }
                for choice in completion::Action::ALL {
                    let reason = power.unavailable(choice);
                    let radio = ui.add_enabled(
                        reason.is_none(),
                        egui::RadioButton::new(current_watch == Some(choice), tr(choice.label())),
                    );
                    if radio.clicked() && current_watch != Some(choice) {
                        next_watch = Some(Some(choice));
                    }
                    if let Some(reason) = reason {
                        radio.on_disabled_hover_text(tr(reason));
                    }
                }
                for reason in [completion::Action::Sleep, completion::Action::Hibernate]
                    .into_iter()
                    .filter_map(|choice| power.unavailable(choice))
                {
                    ui.add(
                        egui::Label::new(RichText::new(tr(reason)).small().color(p.muted)).wrap(),
                    );
                }
            });
        next_watch
    }
}

fn exact_bytes(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    tf!("{out} Bytes", out = out)
}

fn draw_connection_bar(ui: &mut egui::Ui, job: &Job, rows: &[ConnectionRow], p: Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3, p.track);
    let inner = rect.shrink(1.0);
    let total = job
        .total
        .or_else(|| rows.last().map(|r| r.end.saturating_add(1)))
        .unwrap_or(0);
    if total > 0 && !rows.is_empty() {
        for (i, row) in rows.iter().enumerate() {
            let x_start = inner.left() + (row.start as f32 / total as f32) * inner.width();
            let x_end = inner.left()
                + ((row.end.saturating_add(1)).min(total) as f32 / total as f32) * inner.width();
            let seg_len = row.end.saturating_sub(row.start) + 1;
            let ratio = (row.downloaded as f64 / seg_len.max(1) as f64).clamp(0.0, 1.0) as f32;
            let x_fill = x_start + (x_end - x_start) * ratio;
            if x_fill > x_start {
                let fill_rect =
                    Rect::from_min_max(pos2(x_start, inner.top()), pos2(x_fill, inner.bottom()));
                painter.rect_filled(fill_rect, 0, p.blue);
            }
            if i > 0 {
                painter.line_segment(
                    [pos2(x_start, inner.top()), pos2(x_start, inner.bottom())],
                    Stroke::new(1.5, p.orange),
                );
            }
        }
    } else if let Some(pct) = progress(job) {
        let x_fill = inner.left() + inner.width() * pct;
        painter.rect_filled(
            Rect::from_min_max(inner.min, pos2(x_fill, inner.bottom())),
            2,
            p.blue,
        );
    }
    painter.rect_stroke(rect, 3, Stroke::new(1.0, p.border), StrokeKind::Inside);
}

/// A secondary button reading "Hide details" or "Show details", with an up or down arrow.
fn details_toggle(ui: &mut egui::Ui, open: bool) -> egui::Response {
    let label = if open {
        tr("Hide details")
    } else {
        tr("Show details")
    };
    let text = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(13.0),
        Color32::PLACEHOLDER,
    );
    let size = vec2((text.size().x + 44.0).max(74.0), 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), label)
    });
    let visuals = ui.style().interact(&response);
    ui.painter().rect(
        rect.expand(visuals.expansion),
        7,
        visuals.weak_bg_fill,
        visuals.bg_stroke,
        StrokeKind::Inside,
    );
    let ink = visuals.text_color();
    ui.painter().galley(
        pos2(rect.left() + 12.0, rect.center().y - text.size().y / 2.0),
        text,
        ink,
    );
    let tip = pos2(rect.right() - 16.0, rect.center().y);
    let (ends, point) = if open { (2.0, -2.0) } else { (-2.0, 2.0) };
    ui.painter().add(egui::Shape::line(
        vec![
            tip + vec2(-4.0, ends),
            tip + vec2(0.0, point),
            tip + vec2(4.0, ends),
        ],
        Stroke::new(1.6, ink),
    ));
    response
}

fn draw_connection_table(ui: &mut egui::Ui, rows: &[ConnectionRow], p: Palette) {
    egui::Frame::new()
        .fill(p.background)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            let table_width = ui.available_width();
            ui.set_min_width(table_width);
            // Measure the first two columns so the status column, and the row stripes,
            // end with the same inset they start with.
            // egui's Grid gives every column at least `interact_size.x`.
            let widest = |texts: &mut dyn Iterator<Item = (String, f32)>| {
                texts
                    .map(|(text, size)| {
                        ui.painter()
                            .layout_no_wrap(text, egui::FontId::proportional(size), p.text)
                            .size()
                            .x
                    })
                    .fold(ui.spacing().interact_size.x, f32::max)
            };
            let number = widest(
                &mut std::iter::once(("#".into(), 10.5))
                    .chain(rows.iter().map(|row| (row.number.to_string(), 11.0))),
            );
            let downloaded = widest(
                &mut std::iter::once((tr("Downloaded").into(), 10.5))
                    .chain(rows.iter().map(|row| (bytes(row.downloaded), 11.0))),
            );
            let info_width = (table_width - number - downloaded - 2.0 * 24.0).max(80.0);
            ui.spacing_mut().interact_size.y = 14.5;
            egui::Grid::new("status-connections-grid")
                .num_columns(3)
                .min_row_height(14.5)
                .spacing([24.0, 1.5])
                .striped(true)
                .show(ui, |ui| {
                    ui.label(RichText::new("#").size(10.5).strong().color(p.muted));
                    ui.label(
                        RichText::new(tr("Downloaded"))
                            .size(10.5)
                            .strong()
                            .color(p.muted),
                    );
                    ui.allocate_ui_with_layout(
                        vec2(info_width, 14.5),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.set_min_width(info_width);
                            ui.label(
                                RichText::new(tr("Status"))
                                    .size(10.5)
                                    .strong()
                                    .color(p.muted),
                            );
                        },
                    );
                    ui.end_row();
                    for row in rows {
                        ui.label(
                            RichText::new(row.number.to_string())
                                .size(11.0)
                                .color(p.muted),
                        );
                        ui.label(RichText::new(bytes(row.downloaded)).size(11.0));
                        ui.allocate_ui_with_layout(
                            vec2(info_width, 14.5),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.set_min_width(info_width);
                                ui.label(RichText::new(&row.info).size(11.0).color(p.text));
                            },
                        );
                        ui.end_row();
                    }
                });
        });
}

#[cfg(feature = "screenshot")]
impl Desktop {
    fn capture_native_status(&mut self, ui: &egui::Ui) {
        // egui 0.36's Glow immediate-viewport renderer doesn't process Screenshot
        // commands. Read this viewport after its last paint command instead.
        let Some(path) = std::env::var_os("IDM_EX_SCREENSHOT_TO") else {
            return;
        };
        if self.screenshot_requested || ui.ctx().cumulative_frame_nr() < 4 {
            return;
        }
        self.screenshot_requested = true;
        let ctx = ui.ctx().clone();
        let size = (ui.ctx().content_rect().size() * ui.ctx().pixels_per_point()).round();
        ui.painter().add(egui::PaintCallback {
            rect: ui.ctx().content_rect(),
            callback: std::sync::Arc::new(eframe::egui_glow::CallbackFn::new(move |_, painter| {
                use std::io::Write;
                let image = painter.read_screen_rgba([size.x as u32, size.y as u32]);
                let mut file = File::create(&path).expect("Create native status capture");
                write!(file, "P6\n{} {}\n255\n", image.width(), image.height()).unwrap();
                let pixels: Vec<u8> = image
                    .pixels
                    .iter()
                    .flat_map(|p| [p.r(), p.g(), p.b()])
                    .collect();
                file.write_all(&pixels).unwrap();
                file.sync_all().unwrap();
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
                ctx.request_repaint_of(egui::ViewportId::ROOT);
            })),
        });
    }

    pub(super) fn status_sample(&mut self, mode: &str) {
        let _ = self.store.set_queue_running(MAIN_QUEUE, false);
        let mut job = match self.jobs.first().cloned() {
            Some(job) => job,
            None => {
                let mut sample = Job::new(
                    "https://cdn.example.com/releases/Studio-installer-2.4.exe",
                    Some("Studio-installer-2.4.exe"),
                    self.store.paths.downloads.clone(),
                    Uuid::new_v4().to_string(),
                )
                .unwrap();
                sample.status = Status::Paused;
                self.store.enqueue(&sample).unwrap()
            }
        };
        job.status = match mode {
            "status-complete" | "status-native-complete" => Status::Complete,
            "status-error" => Status::Failed,
            "status-paused" => Status::Paused,
            _ => Status::Downloading,
        };
        job.error = (mode == "status-error")
            .then(|| tr("The server connection was interrupted. Resume to try again.").into());
        let total = 1024 * 1024 * 1024;
        job.total = Some(total);
        job.downloaded = if job.status == Status::Complete {
            total
        } else {
            536_870_912
        };
        let seg_count = 6u64;
        let chunk = total / seg_count;
        let segments: Vec<download_engine::model::Segment> = (0..seg_count)
            .map(|i| download_engine::model::Segment {
                start: i * chunk,
                end: if i + 1 == seg_count {
                    total - 1
                } else {
                    (i + 1) * chunk - 1
                },
            })
            .collect();
        job.plan = Some(download_engine::model::Plan {
            size: total,
            etag: "\"sample-etag\"".into(),
            final_url: job.url.clone(),
            segments: segments.clone(),
        });
        if mode == "status-unknown" {
            job.total = None;
            job.plan = None;
        }
        if mode == "status-long" {
            job.name = "A very long download name with spaces and Unicode 日本語 ".repeat(8);
        }
        if matches!(
            mode,
            "status-media" | "status-media-unknown" | "status-native-media"
        ) {
            job.name = "Reviewed-video-720p.mp4".into();
            job.url = "https://cdn.example.com/video/master.m3u8".into();
            let part = |index| download_engine::hls::Part {
                url: format!("https://cdn.example.com/video/{index}.ts"),
                range: None,
                initialization: false,
            };
            job.plan = None;
            job.total = None;
            let mut download =
                download_engine::hls::Download::new(download_engine::hls::Playlist {
                    public_only: false,
                    source: job.url.clone(),
                    url: "https://cdn.example.com/video.m3u8".into(),
                    quality: "720p".into(),
                    bandwidth: None,
                    duration_ms: 60_000,
                    sequence: 0,
                    format: download_engine::hls::Format::TransportStream,
                    parts: (0..4).map(part).collect(),
                    audio: Some(download_engine::hls::AudioTrack {
                        name: tr("Audio").into(),
                        language: None,
                        url: "https://cdn.example.com/audio.m3u8".into(),
                        duration_ms: 60_000,
                        sequence: 0,
                        format: download_engine::hls::Format::TransportStream,
                        parts: (4..8).map(part).collect(),
                    }),
                    dash: None,
                });
            download.completed = (0..3)
                .map(|_| download_engine::hls::CompletedPart {
                    size: 1_048_576,
                    sha256: "0".repeat(64),
                })
                .collect();
            download
                .completed_ahead
                .insert(4, download.completed[0].clone());
            job.downloaded = 4 * 1_048_576 + 524_288;
            job.hls = Some(download);
        }
        self.store.save(&job).unwrap();
        if job.status == Status::Downloading {
            let live: Vec<SegmentSnapshot> = if job.plan.is_some() {
                segments
                    .iter()
                    .enumerate()
                    .map(|(i, s)| SegmentSnapshot {
                        index: i,
                        start: s.start,
                        end: s.end,
                        downloaded: ((s.end - s.start + 1) * (35 + i as u64 * 6)) / 100,
                        state: if i == 0 {
                            SegmentState::SendingGet
                        } else {
                            SegmentState::ReceivingData
                        },
                    })
                    .collect()
            } else {
                Vec::new()
            };
            self.store
                .activity
                .seed_preview(&job.id, Phase::Downloading, live);
            if job.hls.is_some() {
                self.store.activity.seed_media_preview(
                    &job.id,
                    [3, 5, 6]
                        .into_iter()
                        .map(|index| download_engine::transfer_activity::MediaSnapshot {
                            index,
                            downloaded: 524_288 / (index as u64 - 2),
                            total: (mode != "status-media-unknown").then_some(1_048_576),
                            state: SegmentState::ReceivingData,
                        })
                        .collect(),
                );
            }
        }
        self.open_download_status(&job);
        self.last_refresh = Instant::now();
        let window = self.download_status.windows.get_mut(&job.id).unwrap();
        window.rate = 2_097_152.0;
        window.phase = (job.status == Status::Downloading).then_some(Phase::Downloading);
        window.details = true;
        if mode == "status-speed" {
            window.tab = Tab::SpeedLimiter;
        } else if mode.starts_with("status-completion") {
            window.tab = Tab::Completion;
        }
    }
}

#[cfg(test)]
mod tests;
