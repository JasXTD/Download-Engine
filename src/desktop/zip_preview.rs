use super::*;
use download_engine::zip_preview::{self as service, Preview};
use std::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct Task {
    cancel: CancellationToken,
    result: mpsc::Receiver<anyhow::Result<Preview>>,
}

impl Drop for Task {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(super) struct State {
    job: Job,
    from_add: bool,
    preview: Option<Preview>,
    task: Option<Task>,
    error: Option<String>,
    search: String,
}
impl State {
    fn start(&mut self, store: Store, ctx: egui::Context) {
        if self.task.is_some() {
            return;
        }
        let (sender, result) = mpsc::channel();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let job = self.job.clone();
        self.preview = None;
        self.error = None;
        match std::thread::Builder::new()
            .name("zip-preview".into())
            .spawn(move || {
                let loaded = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(anyhow::Error::from)
                    .and_then(|runtime| runtime.block_on(service::inspect(&store, &job, &token)));
                let _ = sender.send(loaded);
                ctx.request_repaint();
            }) {
            Ok(_) => self.task = Some(Task { cancel, result }),
            Err(error) => {
                self.error = Some(tf!("Cannot start ZIP preview: {error}", error = error))
            }
        }
    }
    fn poll(&mut self) {
        let Some(task) = &self.task else {
            return;
        };
        let result = match task.result.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(anyhow::anyhow!(tr(
                "ZIP preview worker stopped. Try again."
            ))),
        };
        self.task = None;
        match result {
            Ok(preview) => self.preview = Some(preview),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}
impl Desktop {
    pub(super) fn open_zip_draft(&mut self) {
        match self.store.new_download(
            &self.url,
            Some(&self.name),
            self.folder_override().as_deref(),
            Uuid::new_v4().to_string(),
        ) {
            Ok(job) => self.set_zip(job, true),
            Err(error) => self.form_error = Some(error.to_string()),
        }
    }
    pub(super) fn open_zip_job(&mut self, job: &Job) {
        if self.dialog.is_some() {
            return;
        }
        self.set_zip(job.clone(), false);
    }
    fn set_zip(&mut self, job: Job, from_add: bool) {
        self.zip_preview = Some(State {
            job,
            from_add,
            preview: None,
            task: None,
            error: None,
            search: String::new(),
        });
        self.dialog = Some(Dialog::ZipPreview);
    }
    pub(super) fn zip_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(state) = &mut self.zip_preview else {
            self.dialog = None;
            return;
        };
        state.poll();
        if state.task.is_none() && state.preview.is_none() && state.error.is_none() {
            state.start(self.store.clone(), ctx.clone());
        }
        let (mut close, mut retry, mut cancel) = (false, false, false);
        let response = form_dialog::modal("zip-preview", p).show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width()-90.0).clamp(480.0, 800.0));
                close = form_dialog::title(ui, tr("ZIP contents"), form_dialog::Icon::Symbol("archive"), p, true);
                ui.add(egui::Label::new(&state.job.name).truncate());
                ui.label(RichText::new(tr("Remote file list only. Nothing is extracted or opened; contents have not been scanned.")).small().color(p.muted));
                if let Some(preview) = &state.preview {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(tf!("{} entries", preview.entries.len()));
                        ui.label(tf!("Archive: {} · Expanded: {} · Preview read: {}", bytes(preview.archive_size), bytes(preview.expanded_size), bytes(preview.metadata_bytes)));
                    });
                    ui.add(view::line_edit(&mut state.search).hint_text(tr("Filter filenames…")).desired_width(f32::INFINITY).char_limit(256));
                    let search = state.search.to_lowercase();
                    let visible: Vec<_> = preview.entries.iter().filter(|e| search.is_empty() || e.name.to_lowercase().contains(&search)).collect();
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(egui::vec2(ui.available_width()-190.0,20.0),egui::Layout::left_to_right(egui::Align::Center),|ui| {
                            ui.set_min_width(ui.available_width());
                            ui.label(tr("File / folder"));
                        });
                        ui.label(tr("Expanded / Packed"));
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().id_salt("zip-entries").max_height((ctx.content_rect().height()-285.0).clamp(100.0, 420.0))
                        .show_rows(ui, 30.0, visible.len(), |ui, rows| {
                            for index in rows {
                                let entry = visible[index];
                                ui.horizontal(|ui| {
                                    let available = ui.available_width();
                                    ui.allocate_ui_with_layout(egui::vec2(available-190.0,26.0),egui::Layout::left_to_right(egui::Align::Center),|ui| {
                                        ui.set_min_width(ui.available_width());
                                        ui.add(egui::Label::new(&entry.name).truncate()).on_hover_text(&entry.name);
                                    });
                                    ui.label(if entry.directory { tr("Folder").into() } else { format!("{} / {}", bytes(entry.size), bytes(entry.compressed_size)) });
                                    if entry.encrypted { ui.label("🔒").on_hover_text(tr("Password-protected entry")); }
                                    if entry.unusual_path { ui.colored_label(p.warning, "!").on_hover_text(tr("Unusual or hidden path characters; preview does not extract this entry")); }
                                });
                            }
                        });
                    if visible.is_empty() { ui.label(tr("No matching entries.")); }
                }
                if state.task.is_some() {
                    ui.horizontal(|ui| { ui.spinner(); ui.label(tr("Reading ZIP directory…"));  });
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                if let Some(error) = &state.error { ui.add(egui::Label::new(RichText::new(error).color(p.warning)).wrap());  }
                if state.error.is_some() || state.task.is_some() || state.from_add {
                    form_dialog::controls(ui, |ui| {
                        if state.error.is_some() { retry = form_dialog::primary_button(ui, tr("Retry preview")).clicked(); }
                        if state.task.is_some() { cancel = form_dialog::secondary_button(ui, tr("Cancel preview")).clicked(); }
                        if state.from_add { close |= form_dialog::secondary_button(ui, tr("Back to download")).clicked(); }
                    });
                }
            });
        if close || response.should_close() {
            let from_add = state.from_add;
            self.zip_preview = None;
            self.dialog = from_add.then_some(Dialog::Add);
        } else if cancel {
            state.task = None;
            state.error = Some(tr("ZIP preview cancelled.").into());
        } else if retry {
            state.start(self.store.clone(), ctx.clone());
        }
    }
    #[cfg(feature = "screenshot")]
    pub(super) fn zip_sample(&mut self) {
        if let Ok(job) = self.store.new_download(
            "https://example.com/release.zip",
            None,
            None,
            Uuid::new_v4().to_string(),
        ) {
            self.set_zip(job, true);
            let entries = [
                "release/",
                "release/README.txt",
                "release/manual.pdf",
                "release/application.exe",
                "release/data/settings.json",
            ]
            .into_iter()
            .enumerate()
            .map(|(i, name)| service::Entry {
                name: name.into(),
                size: if i == 0 { 0 } else { 2048 * i as u64 },
                compressed_size: 1024 * i as u64,
                directory: i == 0,
                encrypted: false,
                unusual_path: false,
            })
            .collect();
            self.zip_preview.as_mut().unwrap().preview = Some(Preview {
                entries,
                archive_size: 15360,
                expanded_size: 20480,
                metadata_bytes: 15360,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::{click, fixture, frame, text_rect};
    use super::*;
    #[test]
    fn preview_returns_to_the_same_add_draft_without_queueing_and_cancel_discards_task() {
        let (_root, mut app, ctx) = fixture();
        app.url = "https://example.com/archive.zip".into();
        app.name = "my archive.zip".into();
        app.open_add();
        // Open the preview state, then provide a known result before any frame
        // can start a network request. Network parsing is tested on loopback.
        app.open_zip_draft();
        assert_eq!(app.dialog, Some(Dialog::ZipPreview));
        let state = app.zip_preview.as_mut().unwrap();
        let token = CancellationToken::new();
        let (_tx, rx) = mpsc::channel();
        state.task = Some(Task {
            cancel: token.clone(),
            result: rx,
        });
        click(&mut app, &ctx, "Back to download");
        assert!(token.is_cancelled());
        assert!(app.zip_preview.is_none());
        assert_eq!(app.dialog, Some(Dialog::Add));
        assert_eq!(app.url, "https://example.com/archive.zip");
        assert_eq!(app.name, "my archive.zip");
        assert!(app.store.list().unwrap().is_empty());
    }
    #[test]
    fn preview_entries_filter_and_fit_at_minimum_size() {
        for dark in [false, true] {
            let (_root, mut app, ctx) = fixture();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let job = app.jobs[0].clone();
            app.open_zip_job(&job);
            let state = app.zip_preview.as_mut().unwrap();
            state.preview = Some(Preview {
                entries: vec![service::Entry {
                    name: "directory/a long name.txt".into(),
                    size: 123,
                    compressed_size: 67,
                    directory: false,
                    encrypted: false,
                    unusual_path: false,
                }],
                archive_size: 200,
                expanded_size: 123,
                metadata_bytes: 200,
            });
            for _ in 0..2 {
                frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
            }
            let output = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
            let name = text_rect(&output, "directory/a long name.txt");
            let heading = text_rect(&output, "File / folder");
            assert!(
                name.left() < 120.0 && (name.left() - heading.left()).abs() < 1.0,
                "ZIP names must align with the left column heading"
            );
            assert!(text_rect(&output, "×").bottom() <= 600.0);
            app.zip_preview.as_mut().unwrap().search = "does-not-exist".into();
            let output = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
            text_rect(&output, "No matching entries.");
            click(&mut app, &ctx, "×");
            assert!(app.zip_preview.is_none() && app.dialog.is_none());
        }
    }
}
