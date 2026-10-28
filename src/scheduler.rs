mod commands;
mod file_operations;
mod runtime;
#[cfg(test)]
use commands::ready_jobs_at;
use commands::{apply, ready_jobs};

use crate::{
    auth, bandwidth, engine, link_recovery,
    model::{Job, Status},
    notifications::{Finished, ScriptNotice},
    queue::MAX_ACTIVE,
    queue_schedule::RecurringSchedule,
    quota, retry,
    store::Store,
};
use anyhow::{Context, Result};
use std::{collections::HashMap, sync::mpsc, thread, time::Duration};
use tokio::{
    sync::mpsc::{UnboundedSender, unbounded_channel},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub enum Command {
    Link(link_recovery::Command),
    Authentication(auth::Update),
    Bandwidth(bandwidth::Update),
    Quota(quota::Update),
    Pause(String),
    Resume(String),
    StartImmediately(String),
    Restart(String),
    SelectDiscoveredFile {
        id: String,
        discovery: uuid::Uuid,
        selection: crate::link_fallback::Selection,
    },
    Remove(String),
    DeleteFile {
        id: String,
        path: std::path::PathBuf,
    },
    /// Rename a finished file in its folder; never replaces another file.
    RenameFile {
        id: String,
        name: String,
    },
    /// Move a finished file into another folder; never replaces another file.
    MoveFile {
        id: String,
        folder: std::path::PathBuf,
    },
    PauseAll,
    StartQueue(String),
    PauseQueue(String),
    SetQueueLimit(String, usize),
    ScheduleQueue(String, i64),
    ScheduleRecurringQueue(String, RecurringSchedule),
    EnableSchedule(String),
    ClearSchedule(String),
    MoveJob(String, String),
    ReorderJob(String, bool),
    Stop,
}
struct Active {
    token: CancellationToken,
    queue: String,
    user_paused: bool,
}

pub struct Scheduler {
    sender: UnboundedSender<Command>,
    thread: Option<thread::JoinHandle<()>>,
    pub errors: mpsc::Receiver<String>,
    pub link_events: mpsc::Receiver<link_recovery::Event>,
    pub auth_events: mpsc::Receiver<auth::Event>,
    pub bandwidth_events: mpsc::Receiver<bandwidth::Event>,
    pub quota_events: mpsc::Receiver<quota::Event>,
    quota_status: tokio::sync::watch::Receiver<Result<quota::Status, String>>,
    pub finished: mpsc::Receiver<Finished>,
    /// Notifications scripts asked to show.
    pub script_notices: mpsc::Receiver<ScriptNotice>,
}
impl Scheduler {
    pub fn start(store: Store) -> Result<Self> {
        store.recover()?;
        store.recover_grabber_scans()?;
        store.recover_controls()?;
        store.recover_scripts()?;
        let (sender, receiver) = unbounded_channel();
        let (errors_tx, errors) = mpsc::channel();
        let (link_tx, link_events) = mpsc::channel();
        let (auth_tx, auth_events) = mpsc::channel();
        let (bandwidth_tx, bandwidth_events) = mpsc::channel();
        let (quota_tx, quota_events) = mpsc::channel();
        let (quota_status_tx, quota_status) =
            tokio::sync::watch::channel(Ok(store.quota_status()?));
        let (finished_tx, finished) = mpsc::channel();
        let (script_notices_tx, script_notices) = mpsc::channel();
        let limiter = bandwidth::Limiter::new(&store.bandwidth_limits()?)?.with_store(&store)?;
        if let Err(error) = auth::cleanup_pending(&store) {
            let _ = errors_tx.send(format!("Credential cleanup is pending: {error}"));
        }
        if let Err(error) = crate::proxy::cleanup_pending(&store) {
            let _ = errors_tx.send(format!("Proxy credential cleanup is pending: {error}"));
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()?;
        let events = runtime::Events {
            errors: errors_tx,
            links: link_tx,
            authentication: auth_tx,
            bandwidth: bandwidth_tx,
            quota: quota_tx,
            quota_status: quota_status_tx,
            finished: finished_tx,
            script_notices: script_notices_tx,
        };
        let thread = thread::Builder::new()
            .name("download-scheduler".into())
            .spawn(move || runtime.block_on(runtime::run(store, receiver, limiter, events)))
            .context("Cannot start the download scheduler")?;
        Ok(Self {
            sender,
            thread: Some(thread),
            errors,
            link_events,
            auth_events,
            bandwidth_events,
            quota_events,
            quota_status,
            finished,
            script_notices,
        })
    }
    pub fn send(&self, command: Command) -> Result<()> {
        self.sender
            .send(command)
            .context("Download scheduler is unavailable")
    }
    /// A bounded, latest-value snapshot; UI callers never wait on SQLite or the meter mutex.
    pub fn quota_snapshot(&self) -> Result<quota::Status> {
        self.quota_status
            .borrow()
            .clone()
            .map_err(anyhow::Error::msg)
    }
}
impl Drop for Scheduler {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
type WorkerResult =
    std::result::Result<(tokio::task::Id, (String, Result<()>)), tokio::task::JoinError>;

fn finish_worker(
    store: &Store,
    active: &mut HashMap<String, Active>,
    worker_ids: &mut HashMap<tokio::task::Id, String>,
    result: WorkerResult,
    errors: &mpsc::Sender<String>,
    finished: &mpsc::Sender<Finished>,
) {
    let (id, error) = match result {
        Ok((task_id, (id, result))) => {
            worker_ids.remove(&task_id);
            (Some(id), result.err().map(|error| format!("{error:#}")))
        }
        Err(error) => (
            worker_ids.remove(&error.id()),
            Some(format!("Download worker stopped unexpectedly: {error}")),
        ),
    };
    if let Some(message) = &error {
        crate::diagnostics::message(crate::diagnostics::Event::WorkerFailed, message);
        let _ = errors.send(message.clone());
    }
    if let Some(id) = id {
        let settled = (|| -> Result<()> {
            if let Some(worker) = active.remove(&id) {
                settle_worker(store, &worker, &id)?;
            }
            // Initialization/final-save failures can escape the engine before
            // it publishes a terminal state. Never leave a job owned by no worker
            // as Downloading, or overwrite a durable completion on cleanup failure.
            if let Some(mut job) = store.get(&id)?
                && job.status == Status::Downloading
            {
                job.status = Status::Failed;
                job.quota_waiting = false;
                job.retry.next_at = None;
                job.error = Some(error.unwrap_or_else(|| {
                    "The download worker stopped unexpectedly. Resume to retry.".into()
                }));
                store.save(&job)?;
            }
            if store
                .get(&id)?
                .is_none_or(|job| job.status != Status::Queued)
            {
                store.cancel_immediate(&id)?;
            }
            Ok(())
        })();
        if let Err(error) = settled {
            let _ = errors.send(format!("Cannot settle download worker: {error:#}"));
        }
        publish_finished(store, &id, finished);
    }
}

async fn settle_before_command(
    store: &Store,
    command: &Command,
    active: &mut HashMap<String, Active>,
    tasks: &mut JoinSet<(String, Result<()>)>,
    worker_ids: &mut HashMap<tokio::task::Id, String>,
    errors: &mpsc::Sender<String>,
    finished: &mpsc::Sender<Finished>,
) {
    // A command and a finished task can both be ready in the select loop.
    // Release finished owners before checking whether a control is allowed.
    while let Some(result) = tasks.try_join_next_with_id() {
        finish_worker(store, active, worker_ids, result, errors, finished);
    }
    let id = match command {
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
        | Command::ReorderJob(id, _) => id,
        Command::Authentication(update) => &update.job,
        Command::Link(link_recovery::Command::Check { job, .. }) => job,
        _ => return,
    };
    if !active.contains_key(id)
        || !store
            .get(id)
            .is_ok_and(|job| job.is_some_and(|j| j.status != Status::Downloading))
    {
        return;
    }
    // The engine publishes its terminal state before asynchronous cleanup ends.
    // Honor an action on that visible state once the old worker has fully exited;
    // otherwise it can be rejected, or an old cleanup could race with a new run.
    // Other transfer tasks keep running, and command order remains unchanged.
    while active.contains_key(id) {
        let Some(result) = tasks.join_next_with_id().await else {
            break;
        };
        finish_worker(store, active, worker_ids, result, errors, finished);
    }
}

fn publish_finished(store: &Store, id: &str, sender: &mpsc::Sender<Finished>) {
    if let Ok(Some(job)) = store.get(id)
        && let Some(event) = Finished::from_job(&job)
    {
        let _ = sender.send(event);
    }
}
// Pause can arrive after an attempt has saved its retry, but before JoinSet
// returns it. Apply user intent after the worker releases ownership as well.
fn settle_worker(store: &Store, worker: &Active, id: &str) -> Result<()> {
    if worker.user_paused
        && let Some(mut job) = store.get(id)?
        && job.status == Status::Queued
        && (job.retry.next_at.is_some() || job.quota_waiting)
    {
        job.status = Status::Paused;
        job.retry.next_at = None;
        job.quota_waiting = false;
        job.error = None;
        store.save(&job)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
