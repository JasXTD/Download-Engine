//! Blocking storage and shell work must not delay egui's pointer event processing.
use super::*;
use std::{sync::mpsc, thread};

pub(super) struct Task<T> {
    result: mpsc::Receiver<anyhow::Result<T>>,
    worker: Option<thread::JoinHandle<()>>,
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::fixture;
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn queue_switch_discards_an_older_inflight_snapshot() {
        let (_temp, mut app, ctx) = fixture();
        let main_job = app.jobs[0].clone();
        app.store.enqueue(&main_job).unwrap();
        let queue = app.store.create_queue("Other queue").unwrap();
        let mut other = main_job.clone();
        other.id = Uuid::new_v4().to_string();
        other.request_id = Uuid::new_v4().to_string();
        other.name = "other.zip".into();
        app.store
            .enqueue_batch(std::slice::from_ref(&other), &queue.id)
            .unwrap();
        app.refresh_and_wait();
        let store = app.store.clone();
        let generation = app.background_io.generation;
        let (release, gate) = mpsc::channel();
        app.background_io.task = Some(
            Task::spawn(None, move || {
                gate.recv_timeout(Duration::from_secs(5))?;
                Ok(Snapshot::load(
                    &store,
                    MAIN_QUEUE.into(),
                    generation,
                    true,
                    false,
                    true,
                ))
            })
            .unwrap(),
        );
        app.queue_id = queue.id.clone();
        app.force_refresh();
        assert!(
            app.jobs.is_empty(),
            "Old queue rows must not appear under the new queue"
        );
        let output = super::super::interaction_tests::frame(
            &mut app,
            &ctx,
            egui::vec2(1080.0, 800.0),
            vec![],
        );
        super::super::interaction_tests::text_rect(&output, "Loading…");
        release.send(()).unwrap();
        app.wait_for_io();
        assert_eq!(app.queue_id, queue.id);
        assert_eq!(app.jobs.len(), 1);
        assert_eq!(app.jobs[0].id, other.id);
    }

    #[test]
    fn folder_launch_is_nonblocking_deduplicated_and_reports_failure() {
        let (_temp, mut app, _ctx) = fixture();
        let directory = app.store.paths.downloads.clone();
        let (release, gate) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let started = Instant::now();
        app.launch_folder_async(directory.clone(), move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            gate.recv_timeout(Duration::from_secs(5))?;
            anyhow::bail!("Isolated shell launch failure")
        });
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_millis(500));
        app.launch_folder_async(directory, |_| panic!("Duplicate shell launch"));
        assert_eq!(app.background_io.folders.len(), 1);
        release.send(()).unwrap();
        app.wait_for_io();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .contains("Isolated shell launch failure")
        );
        eprintln!("folder launch dispatch: {elapsed:?}");
    }
}

impl<T: Send + 'static> Task<T> {
    pub fn spawn(
        context: Option<egui::Context>,
        work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<Self> {
        let language = download_engine::i18n::current();
        let (sender, result) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("desktop-io".into())
            .spawn(move || {
                let _language = download_engine::i18n::scope(language);
                let _ = sender.send(work());
                if let Some(context) = context {
                    context.request_repaint();
                }
            })?;
        Ok(Self {
            result,
            worker: Some(worker),
        })
    }

    pub fn poll(&mut self) -> Option<anyhow::Result<T>> {
        // Never join a running thread in an input frame, even after its send.
        if !self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            return None;
        }
        let joined = self.worker.take().unwrap().join();
        Some(match joined {
            Ok(()) => self.result.try_recv().unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "Background operation ended without a result"
                ))
            }),
            Err(_) => Err(anyhow::anyhow!("Background operation failed unexpectedly")),
        })
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        // A durable submission must finish before Store and the profile lock go
        // away. Tasks are retained until completion, or joined during shutdown.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    pub context: Option<egui::Context>,
    task: Option<Task<Snapshot>>,
    generation: u64,
    pub starts: HashMap<String, bool>,
    pub need_summaries: bool,
    pub need_settings: bool,
    folders: HashMap<PathBuf, Task<()>>,
    sampled: Option<Instant>,
    displayed_queue: Option<String>,
}

pub(super) struct Snapshot {
    apply_jobs: bool,
    generation: u64,
    queue: String,
    sampled: Instant,
    schedules: anyhow::Result<Vec<download_engine::grabber_schedule::Summary>>,
    limits: anyhow::Result<bandwidth::Limits>,
    queues: anyhow::Result<Vec<DownloadQueue>>,
    immediate: anyhow::Result<Vec<String>>,
    jobs: anyhow::Result<Vec<Job>>,
    starts: anyhow::Result<HashMap<String, bool>>,
    folders: anyhow::Result<DownloadFolders>,
    summaries: Option<HashMap<String, Result<queue_overview::Summary, String>>>,
    settings: Option<settings_page::Summary>,
}

#[cfg(test)]
impl State {
    pub(super) fn pending(&self) -> bool {
        self.task.is_some() || !self.folders.is_empty()
    }
}

impl Snapshot {
    fn load(
        store: &Store,
        queue: String,
        generation: u64,
        summaries: bool,
        settings: bool,
        apply_jobs: bool,
    ) -> Self {
        let schedules = store.grabber_schedules();
        let limits = store.bandwidth_limits();
        let queues = store.queues();
        let immediate = store.immediate_downloads();
        let jobs = store.queue_jobs(&queue);
        let starts = queues
            .as_ref()
            .map_err(|e| anyhow::anyhow!(e.to_string()))
            .and_then(|queues| {
                queues
                    .iter()
                    .map(|q| {
                        // Only Main can start automatically while it is idle.
                        let starts = if q.id == MAIN_QUEUE {
                            store.new_download_starts(&q.id)?
                        } else {
                            q.running
                        };
                        Ok((q.id.clone(), starts))
                    })
                    .collect()
            });
        let summaries = summaries.then(|| {
            queues
                .as_ref()
                .map(|queues| {
                    queues
                        .iter()
                        .map(|q| {
                            let summary = if q.id == queue {
                                jobs.as_ref()
                                    .map(|jobs| queue_overview::Summary::from_jobs(jobs))
                                    .map_err(ToString::to_string)
                            } else {
                                store
                                    .queue_jobs(&q.id)
                                    .map(|jobs| queue_overview::Summary::from_jobs(&jobs))
                                    .map_err(|e| e.to_string())
                            };
                            (q.id.clone(), summary)
                        })
                        .collect()
                })
                .unwrap_or_default()
        });
        Self {
            apply_jobs,
            generation,
            queue,
            sampled: Instant::now(),
            schedules,
            limits,
            queues,
            immediate,
            jobs,
            starts,
            summaries,
            folders: store.download_folders(),
            settings: settings.then(|| settings_page::Summary::load(store)),
        }
    }
}

impl Desktop {
    pub(super) fn queue_loading(&self) -> bool {
        self.background_io.task.is_some()
            && self
                .background_io
                .displayed_queue
                .as_ref()
                .is_some_and(|queue| queue != &self.queue_id)
    }

    /// Initial data is read before the event loop starts. Later refreshes never
    /// wait for SQLite, decoding large media plans, or filesystem metadata.
    pub(super) fn initialize_snapshot(&mut self) {
        let data = Snapshot::load(&self.store, self.queue_id.clone(), 0, true, false, true);
        self.apply_snapshot(data);
    }

    pub(super) fn refresh_snapshot(&mut self) {
        self.request_snapshot(true);
    }

    pub(super) fn request_snapshot(&mut self, apply_jobs: bool) {
        if apply_jobs
            && self
                .background_io
                .displayed_queue
                .as_ref()
                .is_some_and(|queue| queue != &self.queue_id)
        {
            self.jobs.clear();
            self.rates.clear();
        }
        if let Some(result) = self.background_io.task.as_mut().and_then(Task::poll) {
            self.background_io.task = None;
            match result {
                Ok(data)
                    if data.generation == self.background_io.generation
                        && data.queue == self.queue_id =>
                {
                    self.apply_snapshot(data)
                }
                Ok(_) => self.last_refresh = Instant::now() - Duration::from_secs(1),
                Err(error) => {
                    self.last_refresh = Instant::now();
                    self.notice = Some(Notice::error(error.to_string()));
                }
            }
        }
        if self.background_io.task.is_some()
            || self.last_refresh.elapsed() < Duration::from_millis(500)
        {
            return;
        }
        let store = self.store.clone();
        let queue = self.queue_id.clone();
        let generation = self.background_io.generation;
        let summaries = self.background_io.need_summaries
            || matches!(self.view, View::Queues | View::Scheduler);
        let settings = self.background_io.need_settings || self.preferences_window.is_some();
        self.last_refresh = Instant::now();
        match Task::spawn(self.background_io.context.clone(), move || {
            Ok(Snapshot::load(
                &store, queue, generation, summaries, settings, apply_jobs,
            ))
        }) {
            Ok(task) => self.background_io.task = Some(task),
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
        if apply_jobs {
            self.refresh_download_status();
        }
    }

    fn apply_snapshot(&mut self, data: Snapshot) {
        match data.schedules {
            Ok(summaries) => self.grabber_summaries = summaries,
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot read grabber schedules: {error}",
                    error = error
                )))
            }
        }
        match data.limits {
            Ok(limits) => self.limits = limits,
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
        match data.queues {
            Ok(queues) => self.queues = queues,
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
        match data.immediate {
            Ok(jobs) => self.immediate_downloads = jobs,
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
        if data.apply_jobs {
            match data.jobs {
                Ok(jobs) => {
                    let seconds = self.background_io.sampled.map_or(0.5, |previous| {
                        data.sampled
                            .saturating_duration_since(previous)
                            .as_secs_f64()
                            .max(0.001)
                    });
                    for job in &jobs {
                        let previous = self
                            .rates
                            .get(&job.id)
                            .map_or(job.downloaded, |value| value.0);
                        let rate = if job.status == Status::Downloading {
                            job.downloaded.saturating_sub(previous) as f64 / seconds
                        } else {
                            0.0
                        };
                        self.rates.insert(job.id.clone(), (job.downloaded, rate));
                    }
                    self.rates
                        .retain(|id, _| jobs.iter().any(|job| &job.id == id));
                    self.jobs = jobs;
                    self.reconcile_selection();
                    self.background_io.sampled = Some(data.sampled);
                    self.background_io.displayed_queue = Some(data.queue.clone());
                }
                Err(error) => self.notice = Some(Notice::error(error.to_string())),
            }
        }
        match data.starts {
            Ok(starts) => self.background_io.starts = starts,
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
        match data.folders {
            Ok(folders) => self.download_folders = Some(folders),
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
        if let Some(summaries) = data.summaries {
            self.queue_summaries = summaries;
            self.background_io.need_summaries = false;
        }
        if let Some(settings) = data.settings {
            self.settings_summary = settings;
            self.background_io.need_settings = false;
        }
        self.last_refresh = data.sampled;
        if data.apply_jobs && self.view == View::Automation {
            self.refresh_automation();
        }
        if !self.queues.iter().any(|queue| queue.id == self.queue_id) {
            self.queue_id = MAIN_QUEUE.into();
            self.jobs.clear();
            self.last_refresh = Instant::now() - Duration::from_secs(1);
        }
    }

    pub(super) fn invalidate_snapshot(&mut self) {
        self.background_io.generation = self.background_io.generation.wrapping_add(1);
        self.last_refresh = Instant::now() - Duration::from_secs(1);
    }

    pub(super) fn open_folder_async(&mut self, directory: PathBuf) {
        self.launch_folder_async(directory, |path| {
            std::process::Command::new(download_engine::paths::file_browser()?)
                .arg(path)
                .spawn()?;
            Ok(())
        });
    }

    fn launch_folder_async(
        &mut self,
        directory: PathBuf,
        launch: impl FnOnce(PathBuf) -> anyhow::Result<()> + Send + 'static,
    ) {
        if self.background_io.folders.contains_key(&directory) {
            return;
        }
        let path = directory.clone();
        match Task::spawn(self.background_io.context.clone(), move || launch(path)) {
            Ok(task) => {
                self.background_io.folders.insert(directory, task);
                self.notice = Some(Notice::info(tr("Opening folder…").into()));
            }
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
    }

    pub(super) fn poll_background_io(&mut self, ctx: &egui::Context) {
        self.background_io.context = Some(ctx.clone());
        self.poll_add_save();
        self.poll_hls_save();
        self.poll_browser_video_save();
        self.poll_antivirus_report();
        let mut finished = Vec::new();
        self.background_io
            .folders
            .retain(|path, task| match task.poll() {
                Some(result) => {
                    finished.push((path.clone(), result.map_err(|error| error.to_string())));
                    false
                }
                None => true,
            });
        for (path, result) in finished {
            self.status_folder_result(&path, result.as_ref().err().map(String::as_str));
            if let Err(error) = result {
                self.notice = Some(Notice::error(error));
            } else if self.background_io.folders.is_empty()
                && self
                    .notice
                    .as_ref()
                    .is_some_and(|notice| notice.message == tr("Opening folder…"))
            {
                self.notice = None;
            }
        }
        self.poll_download_status();
        if self.background_io.task.is_some() || !self.background_io.folders.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}
