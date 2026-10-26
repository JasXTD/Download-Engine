use super::*;
use std::{collections::VecDeque, sync::mpsc};

struct Request {
    job: Job,
    settings: Option<Settings>,
    external_report: Option<Report>,
    for_open: bool,
    batch: Option<uuid::Uuid>,
}
struct Task {
    id: String,
    path: Option<PathBuf>,
    sha256: Option<String>,
    external: bool,
    reviewing: bool,
    cancel: CancellationToken,
    result: mpsc::Receiver<Result<Outcome>>,
    batch: Option<uuid::Uuid>,
}
impl Drop for Task {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// One scanner process at a time, independently of download concurrency.
#[derive(Default)]
pub struct Service {
    queue: VecDeque<Request>,
    active: Option<Task>,
}
impl Service {
    pub fn busy(&self) -> bool {
        self.active.is_some() || !self.queue.is_empty()
    }
    pub fn status(&self, id: &str) -> Option<&'static str> {
        if let Some(task) = self.active.as_ref().filter(|t| t.id == id) {
            Some(if task.reviewing {
                "Verifying the reviewed file…"
            } else if task.external {
                "Requesting antivirus scan…"
            } else {
                "Scanning…"
            })
        } else if let Some(request) = self.queue.iter().find(|r| r.job.id == id) {
            Some(if request.external_report.is_some() {
                "Waiting to verify the reviewed file…"
            } else if request
                .settings
                .as_ref()
                .is_some_and(|settings| settings.provider.external_review())
            {
                "Waiting to request antivirus scan…"
            } else {
                "Waiting to scan…"
            })
        } else {
            None
        }
    }
    pub fn enqueue(&mut self, job: Job, settings: Settings, for_open: bool) {
        // An Open request needs its own fresh scan/permit if an automatic scan is
        // already running. Prioritize it without interrupting the current scan.
        if self.active.as_ref().is_some_and(|t| {
            t.id == job.id && t.path == job.saved_path && t.sha256 == job.integrity.actual
        }) && !for_open
        {
            return;
        }
        if !for_open
            && self.queue.iter().any(|r| {
                r.job.id == job.id
                    && r.job.saved_path == job.saved_path
                    && r.job.integrity.actual == job.integrity.actual
                    && r.for_open
                    && r.external_report.is_none()
            })
        {
            return;
        }
        self.queue.retain(|r| r.job.id != job.id);
        let request = Request {
            job,
            settings: Some(settings),
            external_report: None,
            for_open,
            batch: None,
        };
        if for_open {
            self.queue.push_front(request);
        } else {
            self.queue.push_back(request);
        }
    }
    pub fn enqueue_external_review(&mut self, job: Job, report: Report) {
        self.queue.retain(|request| request.job.id != job.id);
        self.queue.push_front(Request {
            job,
            settings: None,
            external_report: Some(report),
            for_open: true,
            batch: None,
        });
    }
    /// A batch never replaces an existing automatic scan or Open request.
    pub fn enqueue_batch(&mut self, job: Job, settings: Settings, batch: uuid::Uuid) -> bool {
        if self.status(&job.id).is_some() {
            return false;
        }
        self.queue.push_back(Request {
            job,
            settings: Some(settings),
            external_report: None,
            for_open: false,
            batch: Some(batch),
        });
        true
    }
    pub fn batch_pending(&self, batch: uuid::Uuid) -> bool {
        self.active
            .as_ref()
            .is_some_and(|task| task.batch == Some(batch))
            || self
                .queue
                .iter()
                .any(|request| request.batch == Some(batch))
    }
    /// Do not cancel unrelated scans, including a later Open of the same file.
    pub fn cancel_batch(&mut self, batch: uuid::Uuid) {
        self.queue.retain(|request| request.batch != Some(batch));
        if let Some(task) = &self.active
            && task.batch == Some(batch)
        {
            task.cancel.cancel();
        }
    }
    pub fn cancel(&mut self, id: &str) {
        self.queue.retain(|r| r.job.id != id);
        if let Some(task) = &self.active
            && task.id == id
        {
            task.cancel.cancel();
        }
    }
    pub fn cancel_all(&mut self) {
        self.queue.clear();
        if let Some(task) = &self.active {
            task.cancel.cancel();
        }
    }
    pub fn poll(&mut self, store: &Store) -> Option<(String, Result<Outcome>)> {
        self.poll_owned(store).map(|(id, _, result)| (id, result))
    }
    pub fn poll_owned(
        &mut self,
        store: &Store,
    ) -> Option<(String, Option<uuid::Uuid>, Result<Outcome>)> {
        let result = self
            .active
            .as_ref()
            .and_then(|task| match task.result.try_recv() {
                Ok(result) => Some((
                    task.id.clone(),
                    task.batch,
                    result.map(|mut outcome| {
                        // A cancellation can arrive after the worker has sent its
                        // reply but before the UI consumes it. Revoke that permit too.
                        if task.cancel.is_cancelled() {
                            outcome.permit = None;
                            outcome.report.verdict = Verdict::Cancelled;
                            outcome.report.message =
                                "This scan request was cancelled. Scan again before opening."
                                    .into();
                        }
                        outcome
                    }),
                )),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some((
                    task.id.clone(),
                    task.batch,
                    Err(anyhow::anyhow!("The scanner worker stopped unexpectedly")),
                )),
            });
        if result.is_some() {
            self.active = None;
            return result;
        }
        if self.active.is_none()
            && let Some(request) = self.queue.pop_front()
        {
            let id = request.job.id.clone();
            let batch = request.batch;
            let path = request.job.saved_path.clone();
            let sha256 = request.job.integrity.actual.clone();
            let reviewing = request.external_report.is_some();
            let external = reviewing
                || request
                    .settings
                    .as_ref()
                    .is_some_and(|settings| settings.provider.external_review());
            let cancel = CancellationToken::new();
            let worker_cancel = cancel.clone();
            let (tx, rx) = mpsc::channel();
            let store = store.clone();
            match std::thread::Builder::new()
                .name("antivirus-scan".into())
                .spawn(move || {
                    let result = if let Some(report) = request.external_report {
                        external_review_outcome(&store, &request.job, report, &worker_cancel)
                    } else {
                        scan(
                            &store,
                            &request.job,
                            request.settings.as_ref().expect("scanner settings"),
                            &worker_cancel,
                            request.for_open,
                        )
                    };
                    let _ = tx.send(result);
                }) {
                Ok(_) => {
                    self.active = Some(Task {
                        id,
                        path,
                        sha256,
                        external,
                        reviewing,
                        cancel,
                        result: rx,
                        batch,
                    })
                }
                Err(error) => return Some((id, batch, Err(error.into()))),
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopping_a_batch_does_not_cancel_a_later_open_of_the_same_file() {
        let (_temp, _store, job, settings) = super::super::tests::fixture();
        let mut service = Service::default();
        let batch = uuid::Uuid::new_v4();
        assert!(service.enqueue_batch(job.clone(), settings.clone(), batch));
        service.enqueue(job.clone(), settings.clone(), true);
        service.cancel_batch(batch);
        assert!(!service.batch_pending(batch));
        assert_eq!(service.status(&job.id), Some("Waiting to scan…"));
        assert!(service.queue.front().unwrap().for_open);
        assert!(!service.enqueue_batch(job, settings, batch));
    }
    #[test]
    fn a_synchronized_version_is_scanned_even_while_the_previous_copy_is_scanning() {
        let (_temp, _store, old, settings) = super::super::tests::fixture();
        let (_sender, result) = mpsc::channel();
        let mut service = Service {
            queue: VecDeque::new(),
            active: Some(Task {
                id: old.id.clone(),
                path: old.saved_path.clone(),
                sha256: old.integrity.actual.clone(),
                external: false,
                reviewing: false,
                cancel: CancellationToken::new(),
                result,
                batch: None,
            }),
        };
        service.enqueue(old.clone(), settings.clone(), false);
        assert!(service.queue.is_empty());
        let mut updated = old.clone();
        updated.saved_path = Some(old.directory.join("updated.bin"));
        service.enqueue(updated.clone(), settings.clone(), false);
        assert_eq!(service.queue.len(), 1);
        assert_eq!(service.queue[0].job.saved_path, updated.saved_path);
        service.enqueue(updated.clone(), settings.clone(), false);
        assert_eq!(service.queue.len(), 1);
        updated.saved_path = old.saved_path.clone();
        updated.integrity.actual = Some("changed bytes at a reused filename".into());
        service.enqueue(updated.clone(), settings, false);
        assert_eq!(service.queue.len(), 1);
        assert_eq!(
            service.queue[0].job.integrity.actual,
            updated.integrity.actual
        );
    }
    #[test]
    fn cancellation_revokes_a_success_reply_already_waiting_for_the_ui() {
        let (_temp, store, job, settings) = super::super::tests::fixture();
        let outcome = super::super::scan_using(
            &store,
            &job,
            &settings,
            &CancellationToken::new(),
            true,
            |_, _, _| Ok((Verdict::NoThreatsFound, "fixture".into())),
        )
        .unwrap();
        assert!(outcome.permit.is_some());
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(outcome))
            .unwrap_or_else(|_| panic!("Test receiver closed"));
        let mut service = Service {
            queue: VecDeque::new(),
            active: Some(Task {
                id: job.id.clone(),
                path: job.saved_path.clone(),
                sha256: job.integrity.actual.clone(),
                external: false,
                reviewing: false,
                cancel: CancellationToken::new(),
                result: rx,
                batch: None,
            }),
        };
        service.cancel(&job.id);
        let (_, result) = service.poll(&store).unwrap();
        let outcome = result.unwrap();
        assert!(outcome.permit.is_none());
        assert_eq!(outcome.report.verdict, Verdict::Cancelled);
        assert!(!service.busy());
        std::fs::write(job.saved_path.as_ref().unwrap(), b"File guard released").unwrap();
    }

    #[test]
    fn external_review_rehashes_on_the_worker_and_returns_a_guarded_permit() {
        for external_provider in [
            Provider::Malwarebytes,
            Provider::Kaspersky,
            Provider::Bitdefender,
        ] {
            let (_temp, store, job, mut settings) = super::super::tests::fixture();
            settings.provider = external_provider;
            store.save_antivirus_settings(&settings).unwrap();
            let report = super::super::scan_using(
                &store,
                &job,
                &settings,
                &CancellationToken::new(),
                true,
                |_, _, _| Ok((Verdict::ScanRequested, "request".into())),
            )
            .unwrap()
            .report;
            let mut service = Service::default();
            service.enqueue_external_review(job.clone(), report);
            assert_eq!(
                service.status(&job.id),
                Some("Waiting to verify the reviewed file…")
            );
            assert!(service.poll(&store).is_none());
            assert_eq!(
                service.status(&job.id),
                Some("Verifying the reviewed file…")
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let outcome = loop {
                if let Some((id, result)) = service.poll(&store) {
                    assert_eq!(id, job.id);
                    break result.unwrap();
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            assert!(outcome.permit.is_some());
        }
    }
}
