use super::*;
use download_engine::{
    completion::{self, Action, PowerStates, Progress, SoundBatch, Watch},
    integrity,
};
use std::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(super) struct ChecksumTask {
    cancel: CancellationToken,
    result: mpsc::Receiver<anyhow::Result<integrity::State>>,
}
impl Drop for ChecksumTask {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(super) struct ChecksumDialog {
    job: Job,
    expected: String,
    result: integrity::State,
    task: Option<ChecksumTask>,
    error: Option<String>,
}
pub(super) struct Extras {
    pub(super) update_check: Option<updates::State>,
    checksum: Option<ChecksumDialog>,
    pub(super) watches: HashMap<String, Watch>,
    pub(super) exit_deadline: Option<Instant>,
    /// The shut down, sleep or hibernate countdown. Other code cancels it by
    /// assigning `None`, so it keeps the name it had before sleep and hibernate.
    pub(super) shutdown_deadline: Option<PowerCountdown>,
    /// A sleep or hibernation request; Windows answers after the computer wakes.
    power_request: Option<mpsc::Receiver<anyhow::Result<()>>>,
    /// What Windows lets the app request. Tests replace the query.
    pub(super) power_states: fn() -> PowerStates,
    pub(super) sound: SoundBatch,
    completion_check: Instant,
    columns: Option<InterfacePreferences>,
    pending_queue: Option<String>,
    #[cfg(windows)]
    drop_target: Option<crate::url_drop::Registration>,
}
impl Default for Extras {
    fn default() -> Self {
        Self {
            update_check: None,
            checksum: None,
            watches: HashMap::new(),
            exit_deadline: None,
            shutdown_deadline: None,
            power_request: None,
            power_states: power::power_states,
            sound: SoundBatch::default(),
            completion_check: Instant::now(),
            columns: None,
            pending_queue: None,
            #[cfg(windows)]
            drop_target: None,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct PowerCountdown {
    pub(super) action: Action,
    pub(super) deadline: Instant,
}
impl PowerCountdown {
    pub(super) fn start(action: Action) -> Self {
        Self {
            action,
            deadline: Instant::now() + Duration::from_secs(60),
        }
    }
}
fn power_cancelled(action: Action, error: impl std::fmt::Display) -> String {
    match action {
        Action::Sleep => tf!("Sleep cancelled: {error}", error = error),
        Action::Hibernate => tf!("Hibernation cancelled: {error}", error = error),
        _ => tf!("Shutdown cancelled: {error}", error = error),
    }
}
impl Desktop {
    #[cfg(feature = "screenshot")]
    pub(super) fn feature_preview(&mut self, mode: &str) {
        // Shows the explanation for an unavailable choice on any computer.
        if mode.ends_with("-no-hibernate") {
            self.extras.power_states = || PowerStates {
                sleep: true,
                hibernate: false,
            };
        }
        match mode {
            mode if mode.starts_with("status-") => self.status_sample(mode),
            "proxy" | "proxy-enterprise" => self.proxy_sample(mode == "proxy-enterprise"),
            "connections" => self.open_connections(),
            "rename-queue" => {
                self.queue_name = self.current_queue_name().to_owned();
                self.open_queue_dialog(Dialog::RenameQueue);
            }
            "schedule-queue" => {
                self.open_queue_dialog(Dialog::Schedule);
                self.schedule_mode = ScheduleMode::SelectedDays;
            }
            "update-link" => {
                if let Some(job) = self
                    .jobs
                    .iter()
                    .find(|job| job.status != Status::Complete)
                    .cloned()
                {
                    self.open_link(&job);
                }
            }
            "zip-preview" => self.zip_sample(),
            "updates-available" | "updates-current" | "updates-error" => self.updates_sample(mode),
            "ftp-add" | "ftps-add" => {
                self.url = if mode == "ftp-add" {
                    "ftp://downloads.example.com/releases/archive.zip"
                } else {
                    "ftps://downloads.example.com:21/releases/archive.zip"
                }
                .into();
                self.add_authentication = mode == "ftps-add";
                self.open_add();
            }
            "ftps-login" => {
                if let Ok(mut job) = Job::new(
                    "ftps://downloads.example.com:21/archive.zip",
                    None,
                    self.store.paths.downloads.clone(),
                    "screenshot-only".into(),
                ) {
                    job.status = Status::Paused;
                    self.open_authentication(&job);
                }
            }
            "hls" => self.hls_sample(true),
            "youtube-qualities" | "youtube-preview" | "youtube-error" => self.youtube_sample(mode),
            "browser-videos" => self.browser_video_sample(false),
            "browser-videos-empty" => self.browser_video_sample(true),
            "hls-qualities" => self.hls_sample(false),
            "hls-audio" => self.hls_audio_sample(true),
            "hls-audio-qualities" => self.hls_audio_sample(false),
            "dash" => self.dash_sample(true),
            "dash-qualities" => self.dash_sample(false),
            "quotas" => self.open_data_quotas(),
            "grabber" => self.grabber_preview(),
            "link-fallback" | "link-fallback-empty" | "link-fallback-long" => self.link_fallback_sample(mode),
            "grabber-setup" => self.open_grabber(),
            "offline-website" => self.offline_website_sample(false),
            "offline-website-report" => self.offline_website_sample(true),
            "grabber-project" => self.grabber_project_preview(),
            "grabber-schedule" => self.grabber_schedule_preview(),
            "grabber-discoveries" => self.grabber_discoveries_preview(false),
            "grabber-scheduler" => self.grabber_discoveries_preview(true),
            "scan-settings"
            | "scan-settings-kaspersky"
            | "scan-settings-bitdefender"
            | "scan-settings-malwarebytes"
            | "scan-settings-custom" => self.antivirus_settings_preview(mode),
            "scan-single" => self.open_antivirus_scan(antivirus::ScanMode::Single),
            "scan-batch" => self.open_antivirus_scan(antivirus::ScanMode::Batch),
            "scan-batch-results"
            | "scan-batch-review"
            | "scan-batch-review-last"
            | "scan-batch-review-kaspersky"
            | "scan-batch-review-bitdefender" => self.antivirus_batch_preview(mode),
            "scan-result-kaspersky" => self.antivirus_result_preview(),
            "scan-result" => {
                if let Some(job) = self
                    .jobs
                    .iter()
                    .find(|j| j.status == Status::Complete)
                    .cloned()
                {
                    self.review_scan(&job, false);
                }
            }
            "columns" => self.open_columns(),
            "checksum" => {
                if let Some(job) = self
                    .jobs
                    .iter()
                    .find(|j| j.status == Status::Complete)
                    .cloned()
                {
                    let mut job = job;
                    if let Ok(state) = integrity::verify_completed(
                        &self.store,
                        &job,
                        "",
                        &CancellationToken::new(),
                    ) {
                        job.integrity = state;
                    }
                    self.open_checksum(&job);
                }
            }
            "drop" => {
                if let Err(error) = self.review_dropped_urls(
                    "https://example.com/guide.pdf\nhttps://example.com/archive.zip",
                ) {
                    self.notice = Some(Notice::error(error.to_string()));
                }
            }
            "drop-target-one" => self.open_dropped_links(
                "https://downloads.example.com/releases/Studio-installer-2.4.exe".into(),
            ),
            "drop-target-many" => self.open_dropped_links(
                "https://example.com/guide.pdf\nhttps://example.com/archive.zip\nhttps://example.com/video.mp4"
                    .into(),
            ),
            "add" => self.open_add(),
            "clipboard-offer" => self.clipboard_sample(),
            "list-export" | "list-import" | "list-import-details" => {
                self.list_transfer_sample(mode)
            }
            "batch" | "batch-error" => {
                self.batch =
                    "https://example.invalid/guide.pdf\nhttps://example.invalid/archive.zip".into();
                self.open_queue_dialog(Dialog::Batch);
                if mode == "batch-error" {
                    self.batch.push_str("\nnot-a-link");
                    self.submit_queue_dialog(Dialog::Batch);
                }
            }
            "batch-pattern" | "batch-pattern-error" => {
                self.batch_pattern.active = true;
                self.batch_pattern.pattern = "https://example.invalid/photos/img*.jpg".into();
                self.batch_pattern.to = "100".into();
                self.open_queue_dialog(Dialog::Batch);
                if mode == "batch-pattern-error" {
                    self.batch_pattern.to = "5000".into();
                    self.submit_queue_dialog(Dialog::Batch);
                }
            }
            "add-login" | "add-token" | "add-negotiate" | "add-login-error"
            | "add-login-saving" => {
                self.url = "https://cdn.example.invalid/releases/Studio-installer-2.4.exe".into();
                self.add_authentication = true;
                self.add_login.username = Zeroizing::new("demo-user".into());
                self.add_login.secret = Zeroizing::new("preview-only".into());
                if mode == "add-token" {
                    self.add_login.method = AuthKind::Bearer;
                }
                if mode == "add-negotiate" {
                    self.add_login.method = AuthKind::Negotiate;
                    *self.add_login.username = "EXAMPLE\\download-user".into();
                }
                self.open_add();
                if matches!(mode, "add-login-error" | "add-login-saving") {
                    if let Ok(mut job) = self.prepare_add_download() {
                        job.status = Status::Paused;
                        self.add_login.job = Some(job);
                    }
                    if mode == "add-login-saving" {
                        self.add_login.request = Some(Uuid::new_v4());
                    } else {
                        self.form_error = Some(tr("Windows could not save this login. The download remains paused. Try again.").into());
                    }
                }
                self.add_login.reveal = true;
            }
            "add-advanced" => {
                self.url = "https://example.invalid/archive.zip".into();
                self.expected_sha256 = "ab".repeat(32);
                self.add_authentication = true;
                self.open_add();
            }
            "folders" => self.open_download_folders(),
            "speed-limits" => self.open_speed_limits(),
            "remove-confirmation" => {
                self.request_destructive(DestructiveAction::Remove, self.selected_jobs())
            }
            "delete-file-confirmation" | "delete-files-confirmation" => {
                let count = if mode == "delete-files-confirmation" {
                    usize::MAX
                } else {
                    1
                };
                let jobs = self
                    .jobs
                    .iter()
                    .filter(|job| job.status == Status::Complete)
                    .take(count)
                    .cloned()
                    .collect();
                self.request_destructive(DestructiveAction::DeleteFile, jobs);
            }
            "multi-selection" => {
                self.selection
                    .additional
                    .extend(self.jobs.iter().skip(1).take(2).map(|j| j.id.clone()));
                self.notice = Some(Notice::success(
                    tr("Speed limits saved and applied to active downloads.").into(),
                ));
            }
            "add-error" => {
                self.url = "https://example.invalid/releases/archive.zip".into();
                self.open_add();
                self.form_error = Some(
                    tr("Cannot create download folder: access is denied. Choose another folder and try again.")
                        .into(),
                );
            }
            "filtered-empty" => {
                self.search = tr("no matching download").into();
                self.reconcile_selection();
            }
            "completion" => {
                if let Some(queue) = self.queues.iter().find(|q| q.id == self.queue_id)
                    && let Ok(watch) = Watch::new(Action::Notify, queue.name.clone(), &self.jobs)
                {
                    self.extras.watches.insert(queue.id.clone(), watch);
                }
            }
            // The fixture has unfinished files, which would cancel the countdown
            // at the next completion check; hold that check off for the capture.
            "completion-sleep" | "completion-hibernate" => {
                let action = if mode == "completion-sleep" {
                    Action::Sleep
                } else {
                    Action::Hibernate
                };
                self.extras.shutdown_deadline = Some(PowerCountdown::start(action));
                self.extras.completion_check = Instant::now() + Duration::from_secs(3600);
            }
            _ => {}
        }
    }
    pub fn attach_url_drop(&mut self, cc: &eframe::CreationContext<'_>) {
        #[cfg(windows)]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let result = (|| -> anyhow::Result<_> {
                let RawWindowHandle::Win32(handle) = cc.window_handle()?.as_raw() else {
                    anyhow::bail!(tr("Native window handle unavailable"));
                };
                crate::url_drop::Registration::new(handle.hwnd.get(), cc.egui_ctx.clone())
            })();
            match result {
                Ok(target) => self.extras.drop_target = Some(target),
                Err(error) => {
                    self.notice = Some(Notice::error(tf!(
                        "URL drag-and-drop unavailable: {error}",
                        error = error
                    )))
                }
            }
        }
        #[cfg(not(windows))]
        let _ = cc;
    }
    pub(super) fn review_dropped_urls(&mut self, text: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.dialog.is_none(),
            tr("Finish the current dialog before reviewing dropped links")
        );
        let urls = crate::url_drop::urls(text)?;
        // Use a batch draft even for one link, leaving an unfinished Add draft intact.
        self.batch = if self.batch.trim().is_empty() {
            urls
        } else {
            let combined = format!("{}\n{urls}", self.batch);
            anyhow::ensure!(
                combined.len() <= 1_000_000,
                tr("The batch draft is full. Review it before dropping more links.")
            );
            // Preserve even an invalid, unfinished draft for correction in the review form.
            combined
        };
        self.view = View::Downloads;
        self.batch_pattern.active = false;
        self.open_queue_dialog(Dialog::Batch);
        self.notice = Some(Notice::info(
            tr("Dropped links are ready for review. Choose Add files to start them.").into(),
        ));
        self.extras.exit_deadline = None;
        self.extras.shutdown_deadline = None;
        Ok(())
    }
    pub(super) fn review_queue(&mut self, id: String, ctx: &egui::Context) {
        self.extras.pending_queue = Some(id);
        self.show_window(ctx);
    }
    pub(super) fn extras_background(&mut self, ctx: &egui::Context) {
        if self.dialog.is_none() {
            if let Some(id) = self.extras.pending_queue.take() {
                if self.queues.iter().any(|q| q.id == id) {
                    self.queue_id = id;
                    self.view = View::Downloads;
                    self.filter = Filter::All;
                    self.search.clear();
                    self.category = None;
                    self.set_selection(None);
                    self.force_refresh();
                } else {
                    self.notice = Some(Notice::info(tr("This queue has been removed.").into()));
                }
            }
            #[cfg(windows)]
            if let Some(text) = self
                .extras
                .drop_target
                .as_ref()
                .and_then(|t| t.events.try_recv().ok())
                && let Err(error) = self.review_dropped_urls(&text)
            {
                self.notice = Some(Notice::error(error.to_string()));
            }
        }
        if let Some(draft) = &mut self.extras.checksum
            && let Some(task) = &draft.task
        {
            let result = match task.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!(tr(
                    "Checksum worker stopped unexpectedly"
                )))),
            };
            if let Some(result) = result {
                draft.task = None;
                match result {
                    Ok(state) => {
                        draft.result = state;
                        draft.error = None;
                    }
                    Err(error) => draft.error = Some(error.to_string()),
                }
                self.last_refresh = Instant::now() - Duration::from_secs(1);
            }
        }
        if let Some(request) = &self.extras.power_request {
            let result = match request.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!(tr(
                    "Could not request sleep or hibernation"
                )))),
            };
            if let Some(result) = result {
                self.extras.power_request = None;
                if let Err(error) = result {
                    self.notice = Some(Notice::error(format!("{error:#}")));
                }
            }
        }
        if self.extras.completion_check.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.extras.completion_check = Instant::now();
        if !self.extras.sound.is_empty() {
            match completion::downloads_pending(&self.store) {
                Ok(pending) => {
                    if let Some(sound) = self.extras.sound.finish(
                        pending,
                        self.interface.completion_sound,
                        self.preferences.notify_failed,
                    ) {
                        power::play(sound);
                    }
                }
                Err(error) => {
                    // Report once; a later finished file starts a new batch.
                    self.extras.sound = SoundBatch::default();
                    self.notice = Some(Notice::error(error.to_string()));
                }
            }
        }
        let ids: Vec<_> = self.extras.watches.keys().cloned().collect();
        for id in ids {
            let jobs = match self.store.queue_jobs(&id) {
                Ok(jobs) => jobs,
                Err(error) => {
                    self.notice = Some(Notice::error(error.to_string()));
                    continue;
                }
            };
            let watch = &self.extras.watches[&id];
            let progress = watch.progress(&jobs);
            if progress == Progress::Waiting {
                continue;
            }
            let watch = self
                .extras
                .watches
                .remove(&id)
                .expect("Existing queue watch");
            if progress == Progress::Cancelled {
                self.notice = Some(Notice::info(tf!(
                    "Completion action cancelled for {}: a watched file was removed or moved.",
                    watch.name
                )));
                continue;
            }
            self.notice = Some(Notice::success(tf!(
                "{} completed. Completion action is now off.",
                watch.name
            )));
            if let Some(tray) = self.tray.as_ref().filter(|t| t.available()) {
                let notice = download_engine::notifications::Notice {
                    queue: Some(id),
                    job: String::new(),
                    name: watch.name,
                    completed: 1,
                    failed: 0,
                };
                if let Err(error) = tray.notify(notice) {
                    self.notice = Some(Notice::error(error.to_string()));
                }
            }
            if watch.action.is_power() {
                self.extras.exit_deadline = None;
                self.extras.shutdown_deadline = Some(PowerCountdown::start(watch.action));
                self.show_window(ctx);
            } else if watch.action == Action::Exit && self.extras.shutdown_deadline.is_none() {
                self.extras.exit_deadline = Some(Instant::now() + Duration::from_secs(15));
                self.show_window(ctx);
            }
        }
        if let Some(deadline) = self.extras.exit_deadline {
            match self.store.list() {
                Ok(jobs)
                    if completion::can_exit(
                        &jobs,
                        self.dialog.is_some() || self.antivirus.service.busy(),
                    ) =>
                {
                    if Instant::now() >= deadline {
                        self.extras.exit_deadline = None;
                        self.exit(ctx);
                    }
                }
                Ok(_) => {
                    self.extras.exit_deadline = None;
                    self.notice = Some(Notice::info(tr("Automatic exit cancelled: a download or antivirus scan is unfinished, or a dialog is open.").into()));
                }
                Err(error) => {
                    self.extras.exit_deadline = None;
                    self.notice = Some(Notice::info(tf!(
                        "Automatic exit cancelled: {error}",
                        error = error
                    )));
                }
            }
        }
        if let Some(countdown) = self.extras.shutdown_deadline {
            let ready = self.store.list().and_then(|jobs| {
                Ok(completion::can_exit(
                    &jobs,
                    self.dialog.is_some()
                        || self.confirmation.is_some()
                        || self.antivirus.service.busy(),
                ) && completion::background_idle(&self.store)?)
            });
            match ready {
                Ok(true) if Instant::now() >= countdown.deadline => {
                    self.extras.shutdown_deadline = None;
                    self.power_off(countdown.action, ctx);
                }
                Ok(true) => {}
                Ok(false) => {
                    self.extras.shutdown_deadline = None;
                    self.notice = Some(Notice::info(match countdown.action {
                        Action::Sleep => tr("Sleep cancelled: a download, scan, script or command is unfinished, or a dialog is open. Choose the action again when ready.").into(),
                        Action::Hibernate => tr("Hibernation cancelled: a download, scan, script or command is unfinished, or a dialog is open. Choose the action again when ready.").into(),
                        _ => tr("Shutdown cancelled: a download, scan, script or command is unfinished, or a dialog is open. Choose the action again when ready.").into(),
                    }));
                }
                Err(error) => {
                    self.extras.shutdown_deadline = None;
                    self.notice = Some(Notice::error(power_cancelled(countdown.action, error)));
                }
            }
        }
    }
    fn power_off(&mut self, action: Action, ctx: &egui::Context) {
        if action == Action::Shutdown {
            match power::shutdown() {
                Ok(()) => self.exit(ctx),
                Err(error) => self.notice = Some(Notice::error(error.to_string())),
            }
            return;
        }
        // Windows may have turned hibernation off since the action was chosen.
        if let Some(reason) = (self.extras.power_states)().unavailable(action) {
            self.notice = Some(Notice::error(power_cancelled(action, tr(reason))));
            return;
        }
        // A request that has not returned yet already covers this one.
        if self.extras.power_request.is_some() {
            return;
        }
        let (sender, result) = mpsc::channel();
        let context = ctx.clone();
        let hibernate = action == Action::Hibernate;
        match std::thread::Builder::new()
            .name("power-request".into())
            .spawn(move || {
                let _ = sender.send(power::suspend(hibernate));
                context.request_repaint();
            }) {
            Ok(_) => self.extras.power_request = Some(result),
            Err(error) => self.notice = Some(Notice::error(power_cancelled(action, error))),
        }
    }
    /// Arms `action` once for a queue, or turns it off with `None`. A new choice
    /// replaces any visible countdown.
    pub(super) fn arm_completion(
        &mut self,
        queue: &str,
        name: String,
        action: Option<Action>,
    ) -> anyhow::Result<()> {
        let Some(action) = action else {
            self.extras.watches.remove(queue);
            self.extras.exit_deadline = None;
            self.extras.shutdown_deadline = None;
            return Ok(());
        };
        if let Some(reason) = (self.extras.power_states)().unavailable(action) {
            anyhow::bail!(tr(reason));
        }
        let watch = Watch::new(action, name, &self.store.queue_jobs(queue)?)?;
        self.extras.exit_deadline = None;
        self.extras.shutdown_deadline = None;
        self.extras.watches.insert(queue.to_owned(), watch);
        Ok(())
    }
    pub(super) fn completion_menu(&mut self, ui: &mut egui::Ui, queue: &DownloadQueue) {
        ui.menu_button(tr("When queue finishes"), |ui| {
            ui.label(tr("Once, for this app session"));
            let current = self.extras.watches.get(&queue.id).map(|w| w.action);
            let power = (self.extras.power_states)();
            let mut choice = None;
            if ui.selectable_label(current.is_none(), tr("Off")).clicked() {
                choice = Some(None);
            }
            for action in Action::ALL {
                let reason = power.unavailable(action);
                let item = ui.add_enabled(
                    reason.is_none(),
                    egui::Button::selectable(current == Some(action), tr(action.label())),
                );
                if item.clicked() {
                    choice = Some(Some(action));
                }
                if let Some(reason) = reason {
                    item.on_disabled_hover_text(tr(reason));
                }
            }
            for reason in [Action::Sleep, Action::Hibernate]
                .into_iter()
                .filter_map(|action| power.unavailable(action))
            {
                // Menus justify their contents; keep this explanation left-aligned.
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_max_width(280.0);
                    let color = ui.visuals().weak_text_color();
                    ui.add(egui::Label::new(RichText::new(tr(reason)).small().color(color)).wrap());
                });
            }
            if let Some(choice) = choice {
                if let Err(error) = self.arm_completion(&queue.id, queue.name.clone(), choice) {
                    self.notice = Some(Notice::error(error.to_string()));
                }
                ui.close();
            }
            #[cfg(feature = "screenshot")]
            shell::capture_menu_contents(ui, tr("When queue finishes"));
        });
    }
    pub(super) fn completion_banner(&mut self, ui: &mut egui::Ui) {
        if let Some(countdown) = self.extras.shutdown_deadline {
            let seconds = countdown
                .deadline
                .saturating_duration_since(Instant::now())
                .as_secs()
                + 1;
            ui.horizontal_wrapped(|ui| {
                let (message, cancel) = match countdown.action {
                    Action::Sleep => (
                        tf!(
                            "Downloads complete. Putting this computer to sleep in {} seconds.",
                            seconds
                        ),
                        tr("Cancel sleep"),
                    ),
                    Action::Hibernate => (
                        tf!(
                            "Downloads complete. Hibernating this computer in {} seconds.",
                            seconds
                        ),
                        tr("Cancel hibernation"),
                    ),
                    _ => (
                        tf!(
                            "Downloads complete. Shutting down this computer in {} seconds.",
                            seconds
                        ),
                        tr("Cancel shutdown"),
                    ),
                };
                ui.strong(message);
                if ui.button(cancel).clicked() {
                    self.extras.shutdown_deadline = None;
                }
            });
            ui.label(if countdown.action == Action::Shutdown {
                tr("Save any open work. Windows may ask you to close other applications.")
            } else {
                tr("Save any open work. Download Engine stays open and is ready when the computer wakes.")
            });
        }
        if let Some(deadline) = self.extras.exit_deadline {
            ui.horizontal_wrapped(|ui| {
                ui.strong(tf!(
                    "Downloads complete. Exiting Download Engine in {} seconds.",
                    deadline.saturating_duration_since(Instant::now()).as_secs() + 1
                ));
                if ui.button(tr("Cancel exit")).clicked() {
                    self.extras.exit_deadline = None;
                }
            });
        }
    }
    pub(super) fn completion_label(&self, id: &str) -> Option<String> {
        self.extras.watches.get(id).map(|w| {
            tf!(
                "When finished: {} · once this session",
                tr(w.action.label())
            )
        })
    }
    pub(super) fn open_checksum(&mut self, job: &Job) {
        if self.dialog.is_some() {
            return;
        }
        self.extras.checksum = Some(ChecksumDialog {
            job: job.clone(),
            expected: job.integrity.expected.clone().unwrap_or_default(),
            result: job.integrity.clone(),
            task: None,
            error: None,
        });
        self.dialog = Some(Dialog::Checksum);
    }
    pub(super) fn checksum_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(draft) = &mut self.extras.checksum else {
            self.dialog = None;
            return;
        };
        let mut run = false;
        let mut close = false;
        let response = form_dialog::modal("checksum", p).show(ctx, |ui| {
            ui.set_width(460.0); close = form_dialog::title(ui, tr("SHA-256 checksum"), form_dialog::Icon::Symbol("\u{e73e}"), p, draft.task.is_none()); ui.add(egui::Label::new(&draft.job.name).truncate());
            ui.label(tr("Expected SHA-256 (optional, from the publisher)"));
            ui.add_enabled(draft.task.is_none(), view::line_edit(&mut draft.expected).desired_width(f32::INFINITY).char_limit(128));
            ui.label(RichText::new(tr(draft.result.label())).color(if draft.result.matches() == Some(false) { p.warning } else { p.text }));
            if let Some(actual) = &draft.result.actual { ui.add(egui::Label::new(RichText::new(actual).monospace()).wrap()); if ui.button(tr("Copy SHA-256")).clicked() { ctx.copy_text(actual.clone()); } }
            ui.label(RichText::new(tr("The result describes the file when last checked. A match confirms integrity, not whether a file is safe to run.")).small().color(p.muted));
            if let Some(error) = &draft.error { ui.colored_label(p.warning, error); }
            form_dialog::controls(ui, |ui| {
                if let Some(task) = &draft.task {
                    if form_dialog::secondary_button(ui, tr("Cancel calculation")).clicked() { task.cancel.cancel(); }
                    ui.spinner(); ui.label(tr("Reading saved file…"));
                } else { run = form_dialog::primary_button(ui, tr("Calculate / verify")).clicked(); }
            });
        });
        if run {
            match integrity::parse_expected(&draft.expected) {
                Err(error) => draft.error = Some(error.to_string()),
                Ok(_) => {
                    let (sender, result) = mpsc::channel();
                    let cancel = CancellationToken::new();
                    let token = cancel.clone();
                    let store = self.store.clone();
                    let job = draft.job.clone();
                    let expected = draft.expected.clone();
                    let context = ctx.clone();
                    match std::thread::Builder::new()
                        .name("checksum".into())
                        .spawn(move || {
                            let _ = sender
                                .send(integrity::verify_completed(&store, &job, &expected, &token));
                            context.request_repaint();
                        }) {
                        Ok(_) => {
                            draft.task = Some(ChecksumTask { cancel, result });
                            draft.error = None;
                        }
                        Err(error) => draft.error = Some(error.to_string()),
                    }
                }
            }
        }
        if (close || response.should_close()) && draft.task.is_none() {
            self.extras.checksum = None;
            self.dialog = None;
        }
    }
    pub(super) fn open_columns(&mut self) {
        if self.dialog.is_none() {
            self.form_error = None;
            self.extras.columns = Some(
                self.preferences_window
                    .as_ref()
                    .map_or(self.interface, |state| state.interface),
            );
            self.dialog = Some(Dialog::Columns);
        }
    }
    pub(super) fn columns_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(columns) = &mut self.extras.columns else {
            self.dialog = None;
            return;
        };
        let mut save = false;
        let mut cancel = false;
        let response = form_dialog::modal("table-columns", p).show(ctx, |ui| {
            use download_engine::table_layout::{Column, TableLayout};
            ui.set_width(440.0);
            cancel = form_dialog::title(
                ui,
                tr("Table columns"),
                form_dialog::Icon::Symbol("\u{e8fd}"),
                p,
                true,
            );
            form_dialog::body(ui, "columns-body", |ui| {
                // Fill the dialog so the list's scrollbar sits at its edge, not mid-dialog.
                ui.set_min_width(ui.available_width());
                ui.label(tr(
                    "Drag table headings to reorder; drag their edges to resize.",
                ));
                ui.label(tr("File name and Actions cannot be hidden."));
                ui.add_space(6.0);
                egui::Grid::new("column-layout-settings")
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.strong(tr("Column"));
                        ui.strong(tr("Order"));
                        ui.strong(tr("Width"));
                        ui.end_row();
                        for (index, column) in columns.table_layout.order.into_iter().enumerate() {
                            if let Some(i) = column.visibility_index() {
                                ui.square_checkbox(
                                    &mut columns.columns[i],
                                    if column == Column::Number {
                                        tr("Row number")
                                    } else {
                                        tr(column.label())
                                    },
                                );
                            } else {
                                ui.label(tr(column.label()));
                            }
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(index > 0, egui::Button::new("↑"))
                                    .on_hover_text(tr("Move left"))
                                    .clicked()
                                {
                                    columns.table_layout.order.swap(index, index - 1);
                                }
                                if ui
                                    .add_enabled(index < 8, egui::Button::new("↓"))
                                    .on_hover_text(tr("Move right"))
                                    .clicked()
                                {
                                    columns.table_layout.order.swap(index, index + 1);
                                }
                            });
                            ui.horizontal(|ui| {
                                let width = &mut columns.table_layout.widths[column as usize];
                                let mut value = width.unwrap_or(column.default_width());
                                // One width for 2–4 digits keeps the Auto buttons in a column.
                                if ui
                                    .add_sized(
                                        [64.0, ui.spacing().interact_size.y],
                                        egui::DragValue::new(&mut value)
                                            .range(column.minimum()..=TableLayout::MAX_WIDTH)
                                            .suffix(" px"),
                                    )
                                    .changed()
                                {
                                    *width = Some(value);
                                }
                                if ui.selectable_label(width.is_none(), tr("Auto")).clicked() {
                                    *width = None;
                                }
                            });
                            ui.end_row();
                        }
                    });
                ui.add_space(6.0);
                ui.label(tr(
                    "Narrow windows hide extra columns. Wide layouts scroll.",
                ));
                if let Some(error) = &self.form_error {
                    ui.add(egui::Label::new(RichText::new(error).color(p.warning)).truncate())
                        .on_hover_text(error);
                }
            });
            form_dialog::controls(ui, |ui| {
                save = form_dialog::primary_button(ui, tr("Save")).clicked();
                if form_dialog::secondary_button(ui, tr("Reset")).clicked() {
                    columns.columns = InterfacePreferences::default().columns;
                    columns.table_layout = TableLayout::default();
                }
            });
        });
        if save {
            if let Some(state) = &mut self.preferences_window {
                state.interface.columns = columns.columns;
                state.interface.table_layout = columns.table_layout;
                self.dialog = None;
                self.extras.columns = None;
                self.form_error = None;
                return;
            }
            let mut value = self.interface;
            value.columns = columns.columns;
            value.table_layout = columns.table_layout;
            match self.store.save_interface_preferences(value) {
                Ok(()) => {
                    self.interface = value;
                    self.dialog = None;
                    self.extras.columns = None;
                    self.form_error = None;
                }
                Err(error) => {
                    self.form_error = Some(tf!("Cannot save layout: {error}", error = error))
                }
            }
        } else if cancel || response.should_close() {
            self.dialog = None;
            self.extras.columns = None;
            self.form_error = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::{click, fixture, frame, text_rect};
    use super::*;
    fn tick(app: &mut Desktop, ctx: &egui::Context) {
        app.extras.completion_check = Instant::now() - Duration::from_secs(1);
        app.extras_background(ctx);
    }
    #[test]
    fn dropped_links_preserve_drafts_and_never_enqueue_until_reviewed() {
        let (_temp, mut app, _ctx) = fixture();
        app.url = "https://example.com/unfinished-draft".into();
        app.expected_sha256 = "a".repeat(64);
        app.dialog = Some(Dialog::Add);
        assert!(
            app.review_dropped_urls("https://example.com/file.zip")
                .is_err()
        );
        assert_eq!(app.dialog, Some(Dialog::Add));
        app.dialog = None;
        app.review_dropped_urls(
            "https://example.com/file.zip#one\nhttps://example.com/file.zip#two",
        )
        .unwrap();
        assert_eq!(app.batch, "https://example.com/file.zip");
        assert_eq!(app.dialog, Some(Dialog::Batch));
        assert!(app.store.list().unwrap().is_empty());
        assert!(app.url.ends_with("unfinished-draft"));
        assert_eq!(app.expected_sha256, "a".repeat(64));
        app.dialog = None;
        app.review_dropped_urls("https://example.com/two.zip")
            .unwrap();
        assert_eq!(app.batch.lines().count(), 2);
        app.dialog = None;
        app.batch = "unfinished draft".into();
        app.review_dropped_urls("https://example.com/new.zip")
            .unwrap();
        assert_eq!(app.batch, "unfinished draft\nhttps://example.com/new.zip");
        assert!(app.store.list().unwrap().is_empty());
    }
    #[test]
    fn column_choices_cancel_or_save_and_keep_required_cells() {
        let (_temp, mut app, ctx) = fixture();
        app.view = View::Queues;
        app.open_columns();
        click(&mut app, &ctx, "Size");
        click(&mut app, &ctx, "×");
        assert!(app.interface.columns[3]);
        app.open_columns();
        click(&mut app, &ctx, "Size");
        click(&mut app, &ctx, "Save");
        assert!(!app.interface.columns[3]);
        assert_eq!(
            app.store.interface_preferences().unwrap().columns,
            app.interface.columns
        );
        app.view = View::Downloads;
        let output = frame(&mut app, &ctx, egui::vec2(1280.0, 860.0), vec![]);
        text_rect(&output, "File name");
        text_rect(&output, "Actions");
    }
    #[test]
    fn add_checksum_validation_keeps_draft_and_never_starts_invalid_hash() {
        let (_temp, mut app, _ctx) = fixture();
        app.open_add();
        app.url = "https://example.com/file".into();
        app.expected_sha256 = "invalid".into();
        app.add();
        assert!(app.store.list().unwrap().is_empty());
        assert!(app.form_error.as_ref().unwrap().contains("64"));
        assert_eq!(app.dialog, Some(Dialog::Add));
        let now = chrono::Utc::now().timestamp();
        app.store
            .schedule_queue(MAIN_QUEUE, now + 3600, now)
            .unwrap();
        app.expected_sha256 = "Ab".repeat(32);
        app.add();
        assert_eq!(
            app.store.list().unwrap()[0].integrity.expected,
            Some("ab".repeat(32))
        );
        assert!(app.expected_sha256.is_empty());
    }
    #[test]
    fn queue_actions_fire_once_and_exit_is_cancellable_and_guards_other_work() {
        let (_temp, mut app, ctx) = fixture();
        let mut job = app.jobs[0].clone();
        app.store.enqueue(&job).unwrap();
        app.extras.watches.insert(
            MAIN_QUEUE.into(),
            Watch::new(Action::Notify, "Main".into(), &[job.clone()]).unwrap(),
        );
        tick(&mut app, &ctx);
        assert_eq!(app.extras.watches.len(), 1);
        job.status = Status::Complete;
        app.store.save(&job).unwrap();
        tick(&mut app, &ctx);
        assert!(app.extras.watches.is_empty());
        app.notice = None;
        tick(&mut app, &ctx);
        assert!(app.notice.is_none());
        job.status = Status::Paused;
        app.store.save(&job).unwrap();
        app.extras.watches.insert(
            MAIN_QUEUE.into(),
            Watch::new(Action::Exit, "Main".into(), &[job.clone()]).unwrap(),
        );
        job.status = Status::Complete;
        app.store.save(&job).unwrap();
        tick(&mut app, &ctx);
        assert!(app.extras.watches.is_empty());
        assert!(app.extras.exit_deadline.is_some());
        assert!(!app.exit_requested);
        click(&mut app, &ctx, "Cancel exit");
        assert!(app.extras.exit_deadline.is_none());
        job.status = Status::Paused;
        app.store.save(&job).unwrap();
        app.extras.exit_deadline = Some(Instant::now() - Duration::from_secs(1));
        tick(&mut app, &ctx);
        assert!(!app.exit_requested);
        assert!(app.extras.exit_deadline.is_none());
        job.status = Status::Complete;
        app.store.save(&job).unwrap();
        app.antivirus.service.enqueue(
            job.clone(),
            download_engine::antivirus::Settings::default(),
            false,
        );
        app.extras.exit_deadline = Some(Instant::now() - Duration::from_secs(1));
        tick(&mut app, &ctx);
        assert!(!app.exit_requested);
        assert!(app.extras.exit_deadline.is_none());
        app.antivirus.service.cancel_all();
        app.dialog = Some(Dialog::Add);
        app.extras.exit_deadline = Some(Instant::now() - Duration::from_secs(1));
        tick(&mut app, &ctx);
        assert!(!app.exit_requested);
        app.dialog = None;
        app.extras.exit_deadline = Some(Instant::now() - Duration::from_secs(1));
        tick(&mut app, &ctx);
        assert!(app.exit_requested);
    }
    fn all_power() -> PowerStates {
        PowerStates {
            sleep: true,
            hibernate: true,
        }
    }
    fn elapsed(action: Action) -> Option<PowerCountdown> {
        Some(PowerCountdown {
            action,
            deadline: Instant::now() - Duration::from_secs(1),
        })
    }
    fn message(app: &Desktop) -> &str {
        &app.notice.as_ref().expect("A notice").message
    }
    #[test]
    fn sleep_and_hibernate_share_the_shutdown_countdown_cancel_and_safety_rules() {
        let (_temp, mut app, ctx) = fixture();
        app.extras.power_states = all_power;
        let mut job = app.jobs[0].clone();
        app.store.enqueue(&job).unwrap();
        app.arm_completion(MAIN_QUEUE, "Main".into(), Some(Action::Sleep))
            .unwrap();
        job.status = Status::Complete;
        app.store.save(&job).unwrap();
        tick(&mut app, &ctx);
        assert!(app.extras.watches.is_empty());
        let countdown = app.extras.shutdown_deadline.unwrap();
        assert_eq!(countdown.action, Action::Sleep);
        assert!(countdown.deadline > Instant::now() + Duration::from_secs(55));
        assert!(app.extras.exit_deadline.is_none());
        let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
        text_rect(
            &output,
            "Save any open work. Download Engine stays open and is ready when the computer wakes.",
        );
        click(&mut app, &ctx, "Cancel sleep");
        assert!(app.extras.shutdown_deadline.is_none());
        assert!(app.extras.power_request.is_none());

        // Unfinished work or an open dialog cancels the countdown before it acts.
        let mut other = app.jobs[2].clone();
        app.store.enqueue(&other).unwrap();
        app.extras.shutdown_deadline = elapsed(Action::Hibernate);
        tick(&mut app, &ctx);
        assert!(app.extras.shutdown_deadline.is_none() && app.extras.power_request.is_none());
        assert!(message(&app).starts_with("Hibernation cancelled: a download"));
        other.status = Status::Complete;
        app.store.save(&other).unwrap();
        app.dialog = Some(Dialog::Add);
        app.extras.shutdown_deadline = elapsed(Action::Sleep);
        tick(&mut app, &ctx);
        assert!(app.extras.shutdown_deadline.is_none() && app.extras.power_request.is_none());
        assert!(message(&app).starts_with("Sleep cancelled: a download"));
        app.dialog = None;

        // When everything is finished, the request leaves the UI thread. Test
        // builds refuse it, and sleeping never exits Download Engine.
        app.extras.shutdown_deadline = elapsed(Action::Sleep);
        tick(&mut app, &ctx);
        assert!(app.extras.shutdown_deadline.is_none());
        assert!(app.extras.power_request.is_some());
        let end = Instant::now() + Duration::from_secs(5);
        while app.extras.power_request.is_some() {
            assert!(Instant::now() < end, "The power request never returned");
            std::thread::sleep(Duration::from_millis(10));
            app.extras_background(&ctx);
        }
        assert!(message(&app).contains("unavailable in test builds"));
        assert!(!app.exit_requested);

        // Windows can turn hibernation off after the choice was made.
        app.extras.power_states = || PowerStates {
            sleep: true,
            hibernate: false,
        };
        app.extras.shutdown_deadline = elapsed(Action::Hibernate);
        tick(&mut app, &ctx);
        assert!(app.extras.shutdown_deadline.is_none() && app.extras.power_request.is_none());
        assert!(
            message(&app).starts_with("Hibernation cancelled: Hibernate is off"),
            "{}",
            message(&app)
        );
    }
    #[test]
    fn unavailable_power_choices_are_explained_disabled_and_never_armed() {
        let (_temp, mut app, ctx) = fixture();
        app.extras.power_states = || PowerStates {
            sleep: true,
            hibernate: false,
        };
        app.store.enqueue(&app.jobs[0]).unwrap();
        let error = app
            .arm_completion(MAIN_QUEUE, "Main".into(), Some(Action::Hibernate))
            .unwrap_err();
        assert!(error.to_string().contains("powercfg /hibernate on"));
        assert!(app.extras.watches.is_empty());
        app.view = View::Queues;
        click(&mut app, &ctx, "⋯");
        click(&mut app, &ctx, "When queue finishes");
        let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
        text_rect(&output, "Shut down computer");
        text_rect(
            &output,
            "Hibernate is off or not supported on this computer. An administrator can turn it on with powercfg /hibernate on.",
        );
        click(&mut app, &ctx, "Hibernate computer");
        assert!(app.extras.watches.is_empty());
        click(&mut app, &ctx, "Sleep computer");
        assert_eq!(
            app.extras.watches.get(MAIN_QUEUE).map(|w| w.action),
            Some(Action::Sleep)
        );
        app.arm_completion(MAIN_QUEUE, String::new(), None).unwrap();
        assert!(app.extras.watches.is_empty());
    }
    #[test]
    fn one_completion_sound_per_batch_follows_settings_and_waits_for_pending_work() {
        use completion::Sound;
        let (_temp, mut app, ctx) = fixture();
        let played = || power::PLAYED.with(|played| std::mem::take(&mut *played.borrow_mut()));
        // A file waiting for its automatic retry keeps the batch open.
        let mut waiting = app.jobs[0].clone();
        waiting.status = Status::Queued;
        waiting.retry.next_at = Some(chrono::Utc::now().timestamp_millis() + 3_600_000);
        let mut waiting = app.store.enqueue(&waiting).unwrap();
        app.interface.completion_sound = true;
        app.extras.sound.record(false);
        app.extras.sound.record(false);
        tick(&mut app, &ctx);
        assert!(played().is_empty(), "A download still waits to start");
        waiting.status = Status::Paused;
        waiting.retry = Default::default();
        app.store.save(&waiting).unwrap();
        tick(&mut app, &ctx);
        assert_eq!(played(), [Sound::Complete]);
        tick(&mut app, &ctx);
        assert!(played().is_empty(), "One sound per batch");
        app.extras.sound.record(false);
        app.extras.sound.record(true);
        tick(&mut app, &ctx);
        assert_eq!(played(), [Sound::Attention]);
        app.preferences.notify_failed = false;
        app.extras.sound.record(true);
        tick(&mut app, &ctx);
        assert!(played().is_empty(), "Failure alerts are off");
        app.interface.completion_sound = false;
        app.extras.sound.record(false);
        tick(&mut app, &ctx);
        assert!(played().is_empty() && app.extras.sound.is_empty());
    }
    #[test]
    fn completion_sound_setting_saves_with_notifications_and_cancel_keeps_it() {
        let (_temp, mut app, ctx) = fixture();
        app.open_preferences_window();
        app.preferences_window.as_mut().unwrap().category = preferences_window::Category::Startup;
        click(&mut app, &ctx, "Play a sound when downloads finish");
        click(&mut app, &ctx, "×");
        assert!(app.dialog.is_none() && !app.interface.completion_sound);
        app.open_preferences_window();
        app.preferences_window.as_mut().unwrap().category = preferences_window::Category::Startup;
        click(&mut app, &ctx, "Play a sound when downloads finish");
        click(&mut app, &ctx, "Save");
        assert!(app.dialog.is_none());
        assert!(app.interface.completion_sound);
        assert!(app.store.interface_preferences().unwrap().completion_sound);
        assert_eq!(app.store.preferences().unwrap(), app.preferences);
        app.open_preferences_window();
        app.preferences_window.as_mut().unwrap().category = preferences_window::Category::Startup;
        assert!(
            app.preferences_window
                .as_ref()
                .unwrap()
                .interface
                .completion_sound
        );
    }
    #[test]
    fn checksum_dialog_rechecks_saved_bytes_and_displays_the_result() {
        let (_temp, mut app, ctx) = fixture();
        let mut job = app.jobs[1].clone();
        std::fs::create_dir_all(&job.directory).unwrap();
        let path = job.directory.join("guide.pdf");
        std::fs::write(&path, b"abc").unwrap();
        job.saved_path = Some(path);
        app.store.enqueue(&job).unwrap();
        app.open_checksum(&job);
        app.extras.checksum.as_mut().unwrap().expected =
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into();
        click(&mut app, &ctx, "Calculate / verify");
        let end = Instant::now() + Duration::from_secs(5);
        while app.extras.checksum.as_ref().unwrap().task.is_some() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(10));
            app.extras_background(&ctx);
        }
        assert_eq!(
            app.extras.checksum.as_ref().unwrap().result.matches(),
            Some(true)
        );
        assert_eq!(
            app.store.get(&job.id).unwrap().unwrap().integrity.matches(),
            Some(true)
        );
        let output = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
        text_rect(&output, "SHA-256 matched");
        click(&mut app, &ctx, "Copy SHA-256");
        click(&mut app, &ctx, "×");
        assert!(app.dialog.is_none());
    }

    #[test]
    fn column_layout_reset_cancels_or_saves_and_failed_saves_keep_the_draft() {
        use download_engine::table_layout::{Column, TableLayout};
        let (_temp, mut app, ctx) = fixture();
        app.view = View::Queues;
        app.interface
            .table_layout
            .move_column(Column::Speed, Column::Name, true);
        app.interface.table_layout.widths[Column::Name as usize] = Some(420);
        app.store.save_interface_preferences(app.interface).unwrap();
        let custom = app.interface.table_layout;
        app.open_columns();
        click(&mut app, &ctx, "Reset");
        click(&mut app, &ctx, "×");
        assert_eq!(app.interface.table_layout, custom);
        app.open_columns();
        click(&mut app, &ctx, "Reset");
        let conn = rusqlite::Connection::open(app.store.paths.data.join("queue.sqlite3")).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_layout BEFORE UPDATE ON interface_preferences BEGIN SELECT RAISE(ABORT,'fixture save failure'); END;").unwrap();
        click(&mut app, &ctx, "Save");
        assert_eq!(app.dialog, Some(Dialog::Columns));
        assert_eq!(app.interface.table_layout, custom);
        assert_eq!(
            app.extras.columns.unwrap().table_layout,
            TableLayout::default()
        );
        assert!(
            app.form_error
                .as_ref()
                .unwrap()
                .contains("fixture save failure")
        );
        conn.execute_batch("DROP TRIGGER reject_layout;").unwrap();
        click(&mut app, &ctx, "Save");
        assert_eq!(app.dialog, None);
        assert_eq!(app.interface.table_layout, TableLayout::default());
        assert_eq!(
            app.store.interface_preferences().unwrap().table_layout,
            TableLayout::default()
        );
    }
}
