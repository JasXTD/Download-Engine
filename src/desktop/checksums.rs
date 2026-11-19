use super::*;
use download_engine::integrity::manifest;
use std::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Cancelling a read should never block the window on a slow drive.
pub(super) struct Reader<T> {
    pub(super) cancel: CancellationToken,
    result: mpsc::Receiver<anyhow::Result<T>>,
}

impl<T: Send + 'static> Reader<T> {
    pub(super) fn start(
        ctx: Option<egui::Context>,
        work: impl FnOnce(CancellationToken) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<Self> {
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let (send, result) = mpsc::channel();
        let language = download_engine::i18n::current();
        std::thread::Builder::new()
            .name("verify-saved-files".into())
            .spawn(move || {
                let _language = download_engine::i18n::scope(language);
                let _ = send.send(work(token));
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            })?;
        Ok(Self { cancel, result })
    }
    pub(super) fn poll(&self) -> Option<anyhow::Result<T>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!(
                "Verification worker stopped unexpectedly"
            ))),
            Err(mpsc::TryRecvError::Empty) => None,
        }
    }
}
impl<T> Drop for Reader<T> {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

struct Row {
    entry: manifest::Entry,
    selected: bool,
    result: Option<String>,
}
pub(super) struct Batch {
    queue: String,
    path: PathBuf,
    loading: Option<Reader<Vec<manifest::Entry>>>,
    running: Option<Reader<()>>,
    updates: Option<mpsc::Receiver<(usize, String)>>,
    rows: Vec<Row>,
    error: Option<String>,
}

impl Batch {
    fn poll(&mut self) -> bool {
        if let Some(result) = self.loading.as_ref().and_then(Reader::poll) {
            self.loading = None;
            match result {
                Ok(entries) => {
                    self.rows = entries
                        .into_iter()
                        .map(|entry| Row {
                            selected: entry.target.is_some(),
                            entry,
                            result: None,
                        })
                        .collect()
                }
                Err(error) => self.error = Some(format!("{error:#}")),
            }
        }
        let finished = self.running.as_ref().and_then(Reader::poll);
        if let Some(updates) = &self.updates {
            for (index, message) in updates.try_iter() {
                self.rows[index].result = Some(message);
            }
        }
        if let Some(result) = finished {
            self.running = None;
            self.updates = None;
            if let Err(error) = result {
                self.error = Some(tr(&error.to_string()).to_owned());
            }
            return true;
        }
        false
    }

    fn run(&mut self, store: Store, ctx: egui::Context) -> anyhow::Result<()> {
        let queue = self.queue.clone();
        let (send, updates) = mpsc::channel();
        let selected: Vec<_> = self
            .rows
            .iter_mut()
            .enumerate()
            .filter(|(_, row)| row.selected)
            .filter_map(|(index, row)| {
                row.result = None;
                Some((index, row.entry.target.clone()?, row.entry.digest.clone()))
            })
            .collect();
        let repaint = ctx.clone();
        self.running = Some(Reader::start(Some(ctx), move |cancel| {
            for (index, target, digest) in selected {
                anyhow::ensure!(!cancel.is_cancelled(), "Verification cancelled");
                let message = match manifest::verify(&store, &queue, &target, &digest, &cancel) {
                    Ok(result) => tr(result.label()).to_owned(),
                    Err(error) => format!("{error:#}"),
                };
                if send.send((index, message)).is_err() {
                    break;
                }
                repaint.request_repaint();
            }
            anyhow::ensure!(!cancel.is_cancelled(), "Verification cancelled");
            Ok(())
        })?);
        self.updates = Some(updates);
        self.error = None;
        Ok(())
    }
}

impl Desktop {
    pub(super) fn choose_checksum_file(&mut self) {
        if self.dialog.is_some() {
            return;
        }
        match self.folder_picker.open_file(
            tr("Verify checksum file…"),
            &self.store.paths.downloads,
            ("SHA256SUMS, *.sha256", "SHA256SUMS*;*.sha256"),
        ) {
            Ok(Some(path)) => self.open_checksum_file(path),
            Ok(None) => {}
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
    }

    pub(super) fn open_checksum_file(&mut self, path: PathBuf) {
        if self.dialog.is_some() {
            self.notice = Some(Notice::info(
                tr("Finish the current dialog before reviewing dropped links").into(),
            ));
            return;
        }
        let (store, queue, file) = (self.store.clone(), self.queue_id.clone(), path.clone());
        match Reader::start(self.background_io.context.clone(), move |_| {
            manifest::preview(&store, &queue, &file)
        }) {
            Ok(loading) => {
                self.extras.checksum_batch = Some(Batch {
                    queue: self.queue_id.clone(),
                    path,
                    loading: Some(loading),
                    running: None,
                    updates: None,
                    rows: vec![],
                    error: None,
                });
                self.dialog = Some(Dialog::ChecksumBatch);
            }
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
    }

    pub(super) fn checksum_batch_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(batch) = &mut self.extras.checksum_batch else {
            self.dialog = None;
            return;
        };
        let queue_name = self
            .queues
            .iter()
            .find(|q| q.id == batch.queue)
            .map(|q| q.name.as_str())
            .unwrap_or(&batch.queue)
            .to_owned();
        let refresh = batch.poll();
        let busy = batch.loading.is_some() || batch.running.is_some();
        let mut close = false;
        let response = form_dialog::modal("checksum-batch", p).show(ctx, |ui| {
            ui.set_width(530.0_f32.min(ctx.content_rect().width() - 40.0).max(200.0));
            close = form_dialog::title(
                ui,
                tr("Verify checksum file…"),
                form_dialog::Icon::Symbol("\u{e73e}"),
                p,
                true,
            );
            ui.add(
                egui::Label::new(batch.path.file_name().unwrap_or_default().to_string_lossy())
                    .truncate(),
            )
            .on_hover_text(batch.path.display().to_string());
            ui.weak(format!("{} · {queue_name}", tr("Queue")));
            if batch.loading.is_some() {
                ui.spinner();
            }
            egui::ScrollArea::vertical().max_height(300.0).show_rows(
                ui,
                24.0,
                batch.rows.len(),
                |ui, range| {
                    for index in range {
                        let row = &mut batch.rows[index];
                        ui.push_id(index, |ui| {
                            ui.horizontal(|ui| {
                                let detail = row
                                    .entry
                                    .target
                                    .as_ref()
                                    .map(|t| t.path.display().to_string())
                                    .unwrap_or_else(|| row.entry.name.clone());
                                ui.allocate_ui(
                                    egui::vec2(ui.available_width() * 0.53, 22.0),
                                    |ui| {
                                        ui.style_mut().wrap_mode =
                                            Some(egui::TextWrapMode::Truncate);
                                        ui.add_enabled(
                                            !busy && row.entry.target.is_some(),
                                            egui::Checkbox::new(&mut row.selected, &row.entry.name),
                                        )
                                        .on_hover_text(detail);
                                    },
                                );
                                let status = row
                                    .result
                                    .as_deref()
                                    .unwrap_or_else(|| tr(row.entry.status));
                                ui.add(egui::Label::new(status).truncate())
                                    .on_hover_text(status);
                            })
                        });
                    }
                },
            );
            if let Some(error) = &batch.error {
                ui.colored_label(p.warning, error);
            }
            if let Some(work) = &batch.running {
                ui.horizontal(|ui| {
                    ui.spinner();
                    if ui.button(tr("Cancel calculation")).clicked() {
                        work.cancel.cancel();
                    }
                });
            } else if ui
                .add_enabled(
                    !busy && batch.rows.iter().any(|r| r.selected),
                    egui::Button::new(tr("Verify selected files")),
                )
                .clicked()
                && let Err(error) = batch.run(self.store.clone(), ctx.clone())
            {
                batch.error = Some(error.to_string());
            }
        });
        if busy {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        if close || response.should_close() {
            self.extras.checksum_batch = None;
            self.dialog = None;
        }
        if refresh {
            self.force_refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};

    #[test]
    fn checksum_batch_previews_then_verifies_from_the_dialog() {
        let (temp, mut app, ctx) = fixture();
        let mut job = app.jobs[1].clone();
        std::fs::create_dir_all(&job.directory).unwrap();
        let path = job.directory.join("guide.pdf");
        std::fs::write(&path, b"abc").unwrap();
        job.saved_path = Some(path);
        let job = app.store.enqueue(&job).unwrap();
        let file = temp.path().join("SHA256SUMS");
        let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        std::fs::write(&file, format!("{hash}  guide.pdf\n{hash}  elsewhere.pdf")).unwrap();
        app.open_checksum_file(file);
        let deadline = Instant::now() + Duration::from_secs(8);
        let render = |app: &mut Desktop| {
            assert!(Instant::now() < deadline, "checksum dialog timed out");
            std::thread::yield_now();
            frame(app, &ctx, egui::vec2(860.0, 600.0), vec![])
        };
        while app
            .extras
            .checksum_batch
            .as_ref()
            .unwrap()
            .loading
            .is_some()
        {
            render(&mut app);
        }
        let preview = render(&mut app);
        text_rect(&preview, "No matching download");
        let batch = app.extras.checksum_batch.as_ref().unwrap();
        assert!(batch.rows[0].selected && !batch.rows[1].selected);
        assert!(
            app.store
                .get(&job.id)
                .unwrap()
                .unwrap()
                .integrity
                .expected
                .is_none()
        );
        click(&mut app, &ctx, "Verify selected files");
        while app
            .extras
            .checksum_batch
            .as_ref()
            .unwrap()
            .running
            .is_some()
        {
            render(&mut app);
        }
        text_rect(&render(&mut app), "SHA-256 matched");
        assert_eq!(
            app.store.get(&job.id).unwrap().unwrap().integrity.matches(),
            Some(true)
        );
    }
}
