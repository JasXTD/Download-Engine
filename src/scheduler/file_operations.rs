//! Bounded blocking workers own relocation handles; the scheduler keeps polling.
use super::*;
use std::collections::VecDeque;

const MAX_OPERATIONS: usize = 4;
pub(super) type Completion = Result<(tokio::task::Id, Result<()>), tokio::task::JoinError>;
struct Operation {
    job: String,
    request: Option<String>,
    cancel: CancellationToken,
}
#[derive(Default)]
pub(super) struct Controller {
    tasks: JoinSet<Result<()>>,
    owners: HashMap<tokio::task::Id, Operation>,
    recovery: VecDeque<String>,
}
impl Controller {
    pub fn recover(&mut self, store: &Store) -> Result<()> {
        self.recovery = crate::file_relocation::pending(store)?.into();
        self.dispatch_recovery(store);
        Ok(())
    }
    fn dispatch_recovery(&mut self, store: &Store) {
        while self.owners.len() < MAX_OPERATIONS {
            let Some(job) = self.recovery.pop_front() else {
                break;
            };
            let database = store.clone();
            let id = job.clone();
            let cancel = CancellationToken::new();
            let token = cancel.clone();
            let task = self
                .tasks
                .spawn_blocking(move || crate::file_relocation::recover(&database, &id, &token));
            self.owners.insert(
                task.id(),
                Operation {
                    job,
                    request: None,
                    cancel,
                },
            );
        }
    }
    pub fn busy(&self, id: &str) -> bool {
        self.owners.values().any(|op| op.job == id) || self.recovery.iter().any(|job| job == id)
    }
    pub fn check(&self, store: &Store, command: &Command) -> Result<bool> {
        if matches!(command, Command::PauseAll) {
            for op in self.owners.values() {
                op.cancel.cancel();
            }
        }
        if let Some(id) = commands::job_id(command) {
            if matches!(command, Command::Pause(_)) && self.busy(id) {
                for op in self.owners.values().filter(|op| op.job == id) {
                    op.cancel.cancel();
                }
                return Ok(true);
            }
            anyhow::ensure!(
                !self.busy(id) && !crate::file_relocation::is_pending(store, id)?,
                "A file operation is running or needs recovery before changing this download"
            );
        }
        Ok(false)
    }
    pub fn start(
        &mut self,
        store: &Store,
        command: Command,
        request: Option<String>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.owners.len() < MAX_OPERATIONS,
            "Too many file operations are running; try again after one finishes"
        );
        let job = commands::job_id(&command)
            .context("Missing file operation download")?
            .to_owned();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let database = store.clone();
        let task = self
            .tasks
            .spawn_blocking(move || commands::relocate_command(&database, command, &token));
        self.owners.insert(
            task.id(),
            Operation {
                job,
                request,
                cancel,
            },
        );
        Ok(())
    }
    pub fn running(&self) -> bool {
        !self.tasks.is_empty()
    }
    pub async fn next(&mut self) -> Option<Completion> {
        self.tasks.join_next_with_id().await
    }
    pub fn complete(
        &mut self,
        store: &Store,
        completion: Completion,
        errors: &mpsc::Sender<String>,
    ) {
        let (task, result) = match completion {
            Ok((id, result)) => (id, result),
            Err(error) => (
                error.id(),
                Err(anyhow::anyhow!(
                    "File operation stopped unexpectedly: {error}; any journaled operation will be recovered on restart"
                )),
            ),
        };
        if let Some(op) = self.owners.remove(&task) {
            if let Some(request) = op.request {
                let result = result.and_then(|()| {
                    crate::control::job_summary(
                        store,
                        &store.get(&op.job)?.context("Download was removed")?,
                    )
                });
                if let Err(error) = store.finish_control(&request, result) {
                    let _ = errors.send(format!("Cannot save file operation receipt: {error:#}"));
                }
            } else if let Err(error) = result {
                let _ = errors.send(format!("File operation for {}: {error:#}", op.job));
            }
        }
        self.dispatch_recovery(store);
    }
    pub async fn shutdown(&mut self, store: &Store, errors: &mpsc::Sender<String>) {
        // Queued recovery remains durable for next startup. Never abandon an
        // active worker while it still owns a file or could publish a copy.
        self.recovery.clear();
        for op in self.owners.values() {
            op.cancel.cancel();
        }
        while let Some(completion) = self.next().await {
            self.complete(store, completion, errors);
        }
    }
}
