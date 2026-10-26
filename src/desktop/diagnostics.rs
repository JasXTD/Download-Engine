//! Logs page actions share the native folder picker and keep export I/O off the UI.
use super::*;
use download_engine::diagnostics;
use std::{sync::mpsc, thread};

#[derive(Default)]
pub(super) struct State {
    pending: Option<mpsc::Receiver<anyhow::Result<PathBuf>>>,
    worker: Option<thread::JoinHandle<()>>,
    #[cfg(test)]
    pub opened: Option<PathBuf>,
}
impl State {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
}
impl Drop for State {
    fn drop(&mut self) {
        // An accepted export finishes before normal shutdown; do not leave a
        // truncated report behind by abandoning its writer thread.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Desktop {
    fn reveal_diagnostic_folder(&mut self, folder: PathBuf) -> anyhow::Result<()> {
        #[cfg(test)]
        {
            self.diagnostics.opened = Some(folder);
        }
        #[cfg(not(test))]
        {
            std::process::Command::new(download_engine::paths::file_browser()?)
                .arg(folder)
                .spawn()?;
        }
        Ok(())
    }
    pub(super) fn open_diagnostic_logs(&mut self) {
        let folder = diagnostics::directory(&self.store.paths);
        let result = std::fs::create_dir_all(&folder)
            .map_err(anyhow::Error::from)
            .and_then(|()| self.reveal_diagnostic_folder(folder));
        if let Err(error) = result {
            self.notice = Some(Notice::error(tf!(
                "Cannot open logs: {error}",
                error = error
            )));
        }
    }
    pub(super) fn export_diagnostics(&mut self) {
        if self.diagnostics.busy() {
            return;
        }
        match self.folder_picker.choose_titled(
            "",
            &self.store.paths.downloads,
            tr("Choose where to save diagnostics"),
        ) {
            Ok(Some(folder)) => {
                let paths = self.store.paths.clone();
                let (sender, receiver) = mpsc::channel();
                match thread::Builder::new()
                    .name("diagnostic-export".into())
                    .spawn(move || {
                        let result = diagnostics::export(&paths, &PathBuf::from(folder));
                        let _ = sender.send(result);
                    }) {
                    Ok(worker) => {
                        self.diagnostics.worker = Some(worker);
                        self.diagnostics.pending = Some(receiver);
                        self.notice = Some(Notice::info(tr("Exporting diagnostics…").into()));
                    }
                    Err(error) => {
                        self.notice = Some(Notice::error(tf!(
                            "Cannot start diagnostics export: {error}",
                            error = error
                        )))
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot choose export folder: {error}",
                    error = error
                )))
            }
        }
    }
    pub(super) fn poll_diagnostic_export(&mut self) {
        let Some(receiver) = &self.diagnostics.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(anyhow::anyhow!(tr(
                "The diagnostics export stopped unexpectedly"
            ))),
        };
        self.diagnostics.pending = None;
        if let Some(worker) = self.diagnostics.worker.take() {
            let _ = worker.join();
        }
        match result {
            Ok(path) => {
                self.notice = Some(Notice::success(tf!(
                    "Diagnostics saved to {}",
                    path.display()
                )));
            }
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot export diagnostics: {error}",
                    error = error
                )))
            }
        }
    }
}
