//! The scheduler owns workers here; queue policy and worker settlement stay
//! independent of the event loop so commands and shutdown use the same rules.
use super::*;

pub(super) struct Events {
    pub errors: mpsc::Sender<String>,
    pub links: mpsc::Sender<link_recovery::Event>,
    pub authentication: mpsc::Sender<auth::Event>,
    pub bandwidth: mpsc::Sender<bandwidth::Event>,
    pub quota: mpsc::Sender<quota::Event>,
    pub finished: mpsc::Sender<Finished>,
    pub script_notices: mpsc::Sender<ScriptNotice>,
}

struct Runtime {
    store: Store,
    active: HashMap<String, Active>,
    queue_turn: usize,
    links: link_recovery::Controller,
    grabber: crate::grabber_schedule::Controller,
    scripts: crate::script_automation::Controller,
    tasks: JoinSet<(String, Result<()>)>,
    worker_ids: HashMap<tokio::task::Id, String>,
    limiter: bandwidth::Limiter,
    limits_snapshot: Option<bandwidth::Limits>,
    events: Events,
}

pub(super) async fn run(
    store: Store,
    mut receiver: tokio::sync::mpsc::UnboundedReceiver<Command>,
    limiter: bandwidth::Limiter,
    events: Events,
) {
    let mut state = Runtime {
        links: link_recovery::Controller::new(events.links.clone(), limiter.clone()),
        limits_snapshot: None,
        store,
        active: HashMap::new(),
        queue_turn: 0,
        grabber: crate::grabber_schedule::Controller::default(),
        scripts: crate::script_automation::Controller::default(),
        tasks: JoinSet::new(),
        worker_ids: HashMap::new(),
        limiter,
        events,
    };
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            command = receiver.recv() => {
                let Some(command) = command else { break };
                if matches!(command, Command::Stop) { break; }
                state.command(command).await;
            }
            result = state.links.next(), if state.links.running() => {
                state.links.complete(result);
            }
            result = state.tasks.join_next_with_id(), if !state.tasks.is_empty() => {
                if let Some(result) = result { state.finish(result); }
            }
            _ = interval.tick() => state.tick().await,
        }
    }
    state.shutdown().await;
}

impl Runtime {
    async fn control(
        &mut self,
        request: &str,
        action: crate::control::Action,
    ) -> Result<serde_json::Value> {
        use crate::control::{self, Action};
        // A script's file actions need its permission before anything changes.
        control::check_script_files(&self.store, &action)?;
        let script_retry = match &action {
            Action::Retry {
                id,
                script_run: Some(_),
            }
            | Action::Restart {
                id,
                script_run: Some(_),
            } => Some(id.clone()),
            _ => None,
        };
        // Removed downloads have no summary to return afterwards.
        let mut removed = None;
        let (command, job) = match action {
            Action::Pause { id } => (Command::Pause(id.clone()), Some(id)),
            Action::Resume { id } => (Command::Resume(id.clone()), Some(id)),
            Action::Retry { id, .. } => {
                let job = self.store.get(&id)?.context("Download not found")?;
                anyhow::ensure!(
                    job.status == Status::Failed,
                    "Only downloads that need attention can be retried. Use resume for paused downloads."
                );
                (Command::Resume(id.clone()), Some(id))
            }
            Action::Restart { id, .. } => (Command::Restart(id.clone()), Some(id)),
            Action::Rename { id, name, .. } => (
                Command::RenameFile {
                    id: id.clone(),
                    name,
                },
                Some(id),
            ),
            Action::MoveFile { id, folder, .. } => (
                Command::MoveFile {
                    id: id.clone(),
                    folder,
                },
                Some(id),
            ),
            Action::Remove {
                id, delete_file, ..
            } => {
                let job = self.store.get(&id)?.context("Download not found")?;
                removed = Some(id.clone());
                let command = if delete_file {
                    anyhow::ensure!(
                        job.status == Status::Complete,
                        "Only finished downloads have a file to delete"
                    );
                    Command::DeleteFile {
                        id: id.clone(),
                        path: job.saved_path.context("This download has no saved file")?,
                    }
                } else {
                    Command::Remove(id.clone())
                };
                (command, Some(id))
            }
            Action::Notify {
                title,
                text,
                script_run,
            } => {
                let (script, _) = self.store.script_for_run(&script_run)?;
                let title = control::clip(&title, control::NOTICE_TITLE_CHARS);
                let text = control::clip(&text, control::NOTICE_TEXT_CHARS);
                anyhow::ensure!(!title.is_empty(), "A notification needs a title");
                self.store.count_script_notice(&script_run)?;
                let _ = self.events.script_notices.send(ScriptNotice {
                    run: script_run,
                    script: script.name,
                    title: title.clone(),
                    text: text.clone(),
                });
                return Ok(serde_json::json!({"title":title,"text":text}));
            }
            Action::PauseAll => (Command::PauseAll, None),
            Action::StartQueue { queue } => (
                Command::StartQueue(control::queue_id(&self.store, &queue)?),
                None,
            ),
            Action::PauseQueue { queue } => (
                Command::PauseQueue(control::queue_id(&self.store, &queue)?),
                None,
            ),
            Action::QueueLimit { queue, limit } => (
                Command::SetQueueLimit(control::queue_id(&self.store, &queue)?, limit),
                None,
            ),
            other => return control::execute_stored(&self.store, request, other),
        };
        if let Some(id) = &job {
            anyhow::ensure!(self.store.get(id)?.is_some(), "Download not found");
            anyhow::ensure!(
                !self.links.busy(id),
                "Finish or cancel this download's link check first"
            );
        }
        settle_before_command(
            &self.store,
            &command,
            &mut self.active,
            &mut self.tasks,
            &mut self.worker_ids,
            &self.events.errors,
            &self.events.finished,
        )
        .await;
        if let Some(id) = &script_retry {
            self.store.count_script_retry(id)?;
        }
        // A move to another drive copies the file. Download tasks keep running
        // on other threads meanwhile.
        let result = if matches!(command, Command::MoveFile { .. }) {
            tokio::task::block_in_place(|| apply(&self.store, &mut self.active, command))
        } else {
            apply(&self.store, &mut self.active, command)
        };
        if let Err(error) = result {
            if let Some(id) = &script_retry {
                let _ = self.store.uncount_script_retry(id);
            }
            return Err(error);
        }
        if let Some(id) = removed {
            self.limiter.forget(&id);
            return Ok(serde_json::json!({"removed":id}));
        }
        if let Some(id) = job {
            control::job_summary(
                &self.store,
                &self.store.get(&id)?.context("Download was removed")?,
            )
        } else {
            Ok(serde_json::json!({"accepted":true}))
        }
    }

    fn report(&self, error: impl std::fmt::Display) {
        let message = error.to_string();
        crate::diagnostics::message(crate::diagnostics::Event::SchedulerFailed, &message);
        let _ = self.events.errors.send(message);
    }

    fn finish(&mut self, result: WorkerResult) {
        finish_worker(
            &self.store,
            &mut self.active,
            &mut self.worker_ids,
            result,
            &self.events.errors,
            &self.events.finished,
        );
    }

    async fn command(&mut self, command: Command) {
        settle_before_command(
            &self.store,
            &command,
            &mut self.active,
            &mut self.tasks,
            &mut self.worker_ids,
            &self.events.errors,
            &self.events.finished,
        )
        .await;
        match command {
            Command::Link(command) => {
                self.links
                    .handle(&self.store, command, |id| self.active.contains_key(id));
            }
            Command::Bandwidth(update) => {
                let result = self.store.set_bandwidth_limits(&update).and_then(|limits| {
                    self.limiter.update(&limits)?;
                    Ok(limits)
                });
                let _ = self.events.bandwidth.send(bandwidth::Event {
                    request: update.request,
                    result: result.map_err(|e| format!("{e:#}")),
                });
            }
            Command::Quota(update) => {
                let result = self.limiter.set_quota_limits(update.limits).await;
                let _ = self.events.quota.send(quota::Event {
                    request: update.request,
                    result: result.map_err(|e| format!("{e:#}")),
                });
            }
            Command::Authentication(update) => {
                let request = update.request;
                let result = if self.active.contains_key(&update.job)
                    || self.links.busy(&update.job)
                {
                    Err(anyhow::anyhow!(
                        "Wait for this download to pause and finish or cancel its link check first."
                    ))
                } else {
                    auth::apply(&self.store, update)
                };
                let _ = self.events.authentication.send(auth::Event {
                    request,
                    result: result.map_err(|error| format!("{error:#}")),
                });
            }
            command => {
                let id = match &command {
                    Command::Pause(id)
                    | Command::Resume(id)
                    | Command::StartImmediately(id)
                    | Command::Restart(id)
                    | Command::SelectDiscoveredFile { id, .. }
                    | Command::Remove(id)
                    | Command::DeleteFile { id, .. }
                    | Command::RenameFile { id, .. }
                    | Command::MoveFile { id, .. }
                    | Command::MoveJob(id, _)
                    | Command::ReorderJob(id, _) => Some(id),
                    _ => None,
                };
                if id.is_some_and(|id| self.links.busy(id)) {
                    self.report("Finish or cancel the link check before changing this download.");
                    return;
                }
                let removed = match &command {
                    Command::Remove(id) | Command::DeleteFile { id, .. } => Some(id.clone()),
                    _ => None,
                };
                match apply(&self.store, &mut self.active, command) {
                    Err(error) => self.report(format!("{error:#}")),
                    Ok(()) => {
                        if let Some(id) = removed {
                            self.limiter.forget(&id);
                        }
                    }
                }
            }
        }
    }

    async fn tick(&mut self) {
        if let Err(error) = self
            .scripts
            .tick(&self.store, chrono::Utc::now().timestamp())
        {
            self.report(format!("Script automation: {error:#}"));
        }
        // Process a bounded number per tick so scripts cannot starve workers.
        for _ in 0..8 {
            match self.store.claim_control() {
                Ok(Some((id, action))) => {
                    let result = self.control(&id, action).await;
                    if let Err(error) = self.store.finish_control(&id, result) {
                        self.report(error);
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    self.report(error);
                    break;
                }
            }
        }
        if let Err(error) = self.store.run_queue_automations() {
            self.report(format!("Automation: {error:#}"));
            return;
        }
        if let Err(error) = self
            .grabber
            .tick(&self.store, chrono::Utc::now().timestamp())
            .await
        {
            self.report(format!("Scheduled website scan: {error:#}"));
        }
        if let Err(error) = self.limiter.quota_status().await {
            self.report(format!("Data quota accounting is unavailable: {error:#}"));
            return;
        }
        let jobs = match ready_jobs(&self.store, &self.active, &mut self.queue_turn) {
            Ok(jobs) => jobs,
            Err(error) => {
                self.report(format!("{error:#}"));
                return;
            }
        };
        // Read limits AFTER selecting jobs: a browser can insert a new job and
        // its routing limit concurrently. Reading first could dispatch that job
        // without its limit until the next tick. Unchanged buckets retain credit.
        match self.store.bandwidth_limits() {
            Ok(limits) if Some(&limits) != self.limits_snapshot.as_ref() => {
                if let Err(error) = self.limiter.update(&limits) {
                    self.report(error);
                    return;
                }
                self.limits_snapshot = Some(limits);
            }
            Ok(_) => {}
            Err(error) => {
                self.report(error);
                return;
            }
        }
        for (queue, mut job) in jobs {
            if self.links.busy(&job.id) {
                continue;
            }
            job.status = Status::Downloading;
            job.quota_waiting = false;
            job.error = None;
            job.retry.next_at = None;
            if let Err(error) = self.store.save(&job) {
                self.report(error);
                continue;
            }
            let token = CancellationToken::new();
            self.active.insert(
                job.id.clone(),
                Active {
                    token: token.clone(),
                    queue,
                    user_paused: false,
                },
            );
            let store = self.store.clone();
            let limiter = self.limiter.clone();
            let id = job.id.clone();
            let worker = self.tasks.spawn(async move {
                let id = job.id.clone();
                (id, engine::run(store, job, token, limiter).await)
            });
            self.worker_ids.insert(worker.id(), id);
        }
    }

    async fn shutdown(&mut self) {
        if let Err(error) = self.scripts.shutdown(&self.store) {
            self.report(format!("Script shutdown: {error:#}"));
        }
        for worker in self.active.values() {
            worker.token.cancel();
        }
        if let Err(error) = self.grabber.shutdown(&self.store).await {
            self.report(format!("Scheduled website scan: {error:#}"));
        }
        self.links.shutdown().await;
        // Preserve task IDs and use normal settlement, including errors and
        // panics, until every worker has released its files.
        while let Some(result) = self.tasks.join_next_with_id().await {
            self.finish(result);
        }
    }
}
