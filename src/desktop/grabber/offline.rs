use super::*;
use download_engine::offline_website::{self as archive, Config, Progress, Report};
use std::{path::PathBuf, thread};

pub(super) struct State {
    url: String,
    depth: u8,
    page_limit: usize,
    folder: String,
    megabytes: usize,
    external_assets: bool,
    task: Option<Task>,
    progress: Progress,
    report: Option<Report>,
    error: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            url: String::new(),
            depth: 1,
            page_limit: 50,
            folder: String::new(),
            megabytes: 100,
            external_assets: true,
            task: None,
            progress: Progress::default(),
            report: None,
            error: None,
        }
    }
}
struct Task {
    cancel: CancellationToken,
    updates: mpsc::Receiver<Progress>,
    result: mpsc::Receiver<anyhow::Result<Report>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Drop for Task {
    fn drop(&mut self) {
        self.cancel.cancel();
        // Finish finalizing an intentional partial copy on close/exit. The worker
        // owns its output and cannot publish into a newly-opened dialog.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl State {
    pub(super) fn seed_url(&mut self, url: &str) {
        if self.url.is_empty() {
            self.url = url.to_owned();
        }
    }
    fn poll(&mut self) {
        let Some(task) = &self.task else { return };
        for progress in task.updates.try_iter() {
            self.progress = progress;
        }
        match task.result.try_recv() {
            Ok(Ok(report)) => {
                self.report = Some(report);
                self.task = None;
            }
            Ok(Err(error)) => {
                self.error = Some(format!("{error:#}"));
                self.task = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.error = Some(tr("The offline capture worker stopped unexpectedly").into());
                self.task = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
    fn start(&mut self, config: Config, ctx: &egui::Context) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.task.is_none(),
            tr("An offline capture is already running")
        );
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let context = ctx.clone();
        let (sender, result) = mpsc::channel();
        let (update, updates) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("offline-website".into())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(anyhow::Error::from)
                    .and_then(|runtime| {
                        runtime.block_on(archive::save(config, token, |progress| {
                            let _ = update.try_send(progress);
                            context.request_repaint();
                        }))
                    });
                let _ = sender.send(result);
                context.request_repaint();
            })?;
        self.task = Some(Task {
            cancel,
            result,
            updates,
            worker: Some(worker),
        });
        self.report = None;
        self.error = None;
        self.progress = Progress::default();
        Ok(())
    }
}
pub(super) fn mode_switch(ui: &mut egui::Ui, offline: &mut bool, running: bool) -> bool {
    let previous = *offline;
    ui.add_enabled_ui(!running, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(offline, false, tr("Find downloadable files"));
            ui.selectable_value(offline, true, tr("Offline website"));
        });
    });
    ui.add_space(4.0);
    *offline != previous
}
impl Desktop {
    pub(in crate::desktop::grabber) fn offline_website_dialog(
        &mut self,
        ctx: &egui::Context,
        p: Palette,
    ) {
        let draft = self.grabber.as_mut().expect("grabber exists");
        let state = &mut draft.offline;
        state.poll();
        let running = state.task.is_some();
        let mut close = false;
        let mut start = false;
        let mut open = None;
        let response = form_dialog::modal(DIALOG_ID, p).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 100.0).clamp(330.0, 620.0));
            close |= form_dialog::title(ui, tr("Website grabber"), form_dialog::Icon::Symbol("\u{e774}"), p, true);
            mode_switch(ui, &mut draft.offline_mode, running);
            egui::ScrollArea::vertical().id_salt("offline-website-body")
                .max_height((ctx.content_rect().height() - 240.0).max(180.0)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                if let Some(report) = &state.report {
                    ui.heading(tr(report.stop.label()));
                    ui.label(tf!("{} pages · {} assets · {:.1} MB received", report.pages, report.assets, report.bytes as f64 / 1048576.0));
                    ui.label(RichText::new(tr("Open index.html to browse your saved copy.")).color(p.success));
                    ui.add(egui::Label::new(report.folder.display().to_string()).wrap());
                    if report.issue_count > 0 {
                        ui.colored_label(p.warning, tf!("{} links or resources were not captured. See the report for details.", report.issue_count));
                    }
                    if report.interactive_pages > 0 {
                        ui.label(tf!("Interactive content was disabled on {} pages.", report.interactive_pages));
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(tr("View capture report")).clicked() { open = Some(report.folder.join("report.html")); }
                        if ui.button(tr("Open folder")).clicked() { open = Some(report.folder.clone()); }
                    });
                } else {
                    ui.label(tr("Save pages, styles, images and fonts for offline browsing."));
                    egui::Frame::new().fill(p.surface).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(8)
                        .inner_margin(12).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add_enabled_ui(!running, |ui| {
                            ui.label(tr("Starting page"));
                            ui.add(view::line_edit(&mut state.url).desired_width(f32::INFINITY)
                                .hint_text("https://example.com/guide/").char_limit(16384));
                            ui.label(tr("Save in folder"));
                            self.folder_picker.field(ui, &mut state.folder, &self.store.paths.downloads, &mut state.error);
                            ui.label(RichText::new(tr("A new website folder is created here. Blank uses your Downloads folder.")).small().color(p.muted));
                            ui.horizontal_wrapped(|ui| {
                                ui.label(tr("Link depth")); ui.add(egui::DragValue::new(&mut state.depth).range(0..=archive::MAX_DEPTH));
                                ui.add_space(8.0);
                                ui.label(tr("Page limit")); ui.add(egui::DragValue::new(&mut state.page_limit).range(1..=archive::MAX_PAGES));
                                ui.add_space(8.0);
                                ui.label(tr("Size limit (MB)")); ui.add(egui::DragValue::new(&mut state.megabytes).range(1..=archive::MAX_MEGABYTES));
                            });
                            ui.square_checkbox(&mut state.external_assets, tr("Include styles and images from other websites"));
                        });
                    });
                    ui.label(RichText::new(tr("Depth 0 saves one page and its assets. Linked pages stay on the starting website.")).small().color(p.muted));
                    ui.label(RichText::new(tr("Static copy: scripts, forms, embedded players and sign-in are disabled. Missing pages open the capture report.")).small().color(p.muted));
                    if running {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(tf!("Saving · {} pages · {} assets · {:.1} MB", state.progress.pages, state.progress.assets, state.progress.bytes as f64 / 1048576.0));
                        });
                        ui.add(egui::Label::new(&state.progress.current).truncate()).on_hover_text(&state.progress.current);
                    }
                }
                if let Some(error) = &state.error { ui.colored_label(p.warning, error); }
            });
            form_dialog::controls(ui, |ui| {
                if running {
                    let cancelled = state.task.as_ref().is_some_and(|t| t.cancel.is_cancelled());
                    if form_dialog::secondary_enabled(ui, !cancelled, if cancelled { tr("Stopping…") } else { tr("Stop capture") }).clicked() {
                        state.task.as_ref().unwrap().cancel.cancel();
                    }
                } else if let Some(report) = &state.report {
                    if form_dialog::primary_button(ui, tr("Open offline website")).clicked() { open = Some(report.entry()); }
                    if form_dialog::secondary_button(ui, tr("New capture")).clicked() { state.report = None; }
                } else { start = form_dialog::primary_button(ui, tr("Save offline copy")).clicked(); }
            });
        });
        if close || response.should_close() {
            self.grabber = None;
            self.dialog = None;
        } else if start {
            let folder = if state.folder.trim().is_empty() {
                self.store.paths.downloads.clone()
            } else {
                PathBuf::from(state.folder.trim())
            };
            let result = Config::new(&state.url, folder, state.depth, state.page_limit)
                .and_then(|mut config| {
                    config.megabytes = state.megabytes;
                    config.external_assets = state.external_assets;
                    config.with_proxy(&self.store)
                })
                .and_then(|config| state.start(config, ctx));
            if let Err(error) = result {
                state.error = Some(format!("{error:#}"));
            }
        } else if let Some(path) = open {
            #[cfg(not(test))]
            let result = if path.is_dir() {
                download_engine::paths::file_browser().and_then(|browser| {
                    std::process::Command::new(browser)
                        .arg(path)
                        .spawn()
                        .map(|_| ())
                        .map_err(Into::into)
                })
            } else {
                download_engine::paths::open_download(&path)
            };
            #[cfg(test)]
            let result = if path.exists() {
                Ok(())
            } else {
                Err(anyhow::anyhow!(tr("Saved copy no longer exists")))
            };
            if let Err(error) = result {
                state.error = Some(tf!("Cannot open the saved copy: {error}", error = error));
            }
        }
    }
    #[cfg(feature = "screenshot")]
    pub(in crate::desktop) fn offline_website_sample(&mut self, report: bool) {
        self.open_grabber();
        if let Some(draft) = &mut self.grabber {
            draft.offline_mode = true;
            draft.offline.url = "https://example.com/guide/".into();
            if report {
                draft.offline.report = Some(Report {
                    folder: self.store.paths.downloads.join("Example-guide"),
                    start_url: draft.offline.url.clone(),
                    pages: 8,
                    assets: 24,
                    bytes: 3_145_728,
                    stop: archive::StopReason::Finished,
                    issues: vec![],
                    issue_count: 2,
                    interactive_pages: 3,
                    files: vec![],
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};

    #[test]
    fn switching_grabber_tabs_keeps_dialog_and_backdrop_visible_on_every_frame() {
        for dark in [true, false] {
            for animation_time in [0.0, 0.15] {
                for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
                    let (_temp, mut app, ctx) = fixture();
                    ctx.set_theme(if dark {
                        egui::ThemePreference::Dark
                    } else {
                        egui::ThemePreference::Light
                    });
                    ctx.all_styles_mut(|style| style.animation_time = animation_time);
                    app.open_grabber();
                    // Let the initial dialog opening finish, then inspect every
                    // frame of tab changes instead of skipping to settled output.
                    for _ in 0..24 {
                        frame(&mut app, &ctx, size, vec![]);
                    }
                    let assert_visible = |output: &egui::FullOutput| {
                        let backdrop = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                        assert!(
                            output.shapes.iter().any(|shape| {
                                matches!(&shape.shape, egui::Shape::Rect(rect)
                                if rect.rect == backdrop
                                    && rect.fill == Color32::from_black_alpha(100))
                            }),
                            "Grabber backdrop disappeared or faded: dark={dark}, animation={animation_time}, size={size:?}"
                        );
                        text_rect(output, "Website grabber");
                    };
                    for _ in 0..3 {
                        for (label, offline) in [
                            ("Offline website", true),
                            ("Find downloadable files", false),
                        ] {
                            let output = frame(&mut app, &ctx, size, vec![]);
                            assert_visible(&output);
                            let pos = text_rect(&output, label).center();
                            for pressed in [true, false] {
                                let output = frame(
                                    &mut app,
                                    &ctx,
                                    size,
                                    vec![
                                        egui::Event::PointerMoved(pos),
                                        egui::Event::PointerButton {
                                            pos,
                                            button: egui::PointerButton::Primary,
                                            pressed,
                                            modifiers: egui::Modifiers::NONE,
                                        },
                                    ],
                                );
                                assert_visible(&output);
                            }
                            assert_eq!(app.grabber.as_ref().unwrap().offline_mode, offline);
                            for _ in 0..12 {
                                assert_visible(&frame(&mut app, &ctx, size, vec![]));
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn saving_through_the_dialog_finishes_a_real_copy_without_adding_downloads() {
        use std::io::{Read, Write};
        let (root, mut app, ctx) = fixture();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = [0u8; 4096];
            assert!(stream.read(&mut request).unwrap() > 0);
            let body = "<html><head><title>Saved through UI</title></head><body><h1>Offline</h1></body></html>";
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        app.open_grabber();
        let draft = app.grabber.as_mut().unwrap();
        draft.url = url;
        draft.offline.folder = root.path().join("copies").display().to_string();
        click(&mut app, &ctx, "Offline website");
        click(&mut app, &ctx, "Save offline copy");
        assert!(app.grabber.as_ref().unwrap().offline.task.is_some());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.grabber.as_ref().unwrap().offline.task.is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "Capture worker did not finish"
            );
            app.grabber.as_mut().unwrap().offline.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        server.join().unwrap();
        let state = &app.grabber.as_ref().unwrap().offline;
        assert!(state.error.is_none(), "{:?}", state.error);
        let report = state.report.as_ref().unwrap();
        assert_eq!(report.pages, 1);
        let entry = report.entry();
        assert!(
            std::fs::read_to_string(&entry)
                .unwrap()
                .contains("Saved through UI")
        );
        assert!(
            app.store
                .queue_jobs(download_engine::queue::MAIN_QUEUE)
                .unwrap()
                .is_empty()
        );
        click(&mut app, &ctx, "×");
        assert!(app.grabber.is_none() && entry.exists());
    }
    #[test]
    fn offline_mode_validates_without_queueing_and_preserves_discovery_settings() {
        let (_temp, mut app, ctx) = fixture();
        app.open_grabber();
        app.grabber.as_mut().unwrap().extensions = "pdf".into();
        click(&mut app, &ctx, "Offline website");
        click(&mut app, &ctx, "Save offline copy");
        assert!(app.grabber.as_ref().unwrap().offline.error.is_some());
        assert!(app.grabber.as_ref().unwrap().offline.task.is_none());
        assert!(
            app.store
                .queue_jobs(download_engine::queue::MAIN_QUEUE)
                .unwrap()
                .is_empty()
        );
        click(&mut app, &ctx, "Find downloadable files");
        assert_eq!(app.grabber.as_ref().unwrap().extensions, "pdf");
        let draft = app.grabber.as_mut().unwrap();
        draft.url = "https://example.com/discovery/".into();
        draft.depth = 3;
        draft.page_limit = 80;
        click(&mut app, &ctx, "Offline website");
        let offline = &mut app.grabber.as_mut().unwrap().offline;
        assert_eq!(offline.url, "https://example.com/discovery/");
        offline.url = "https://example.com/different-copy/".into();
        offline.depth = 0;
        offline.page_limit = 1;
        click(&mut app, &ctx, "Find downloadable files");
        let draft = app.grabber.as_ref().unwrap();
        assert_eq!(draft.url, "https://example.com/discovery/");
        assert_eq!((draft.depth, draft.page_limit), (3, 80));
    }
    #[test]
    fn offline_form_fits_small_and_large_windows_in_both_themes() {
        for dark in [true, false] {
            for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
                let (_temp, mut app, ctx) = fixture();
                ctx.set_theme(if dark {
                    egui::ThemePreference::Dark
                } else {
                    egui::ThemePreference::Light
                });
                app.open_grabber();
                app.grabber.as_mut().unwrap().offline_mode = true;
                frame(&mut app, &ctx, size, vec![]);
                let out = frame(&mut app, &ctx, size, vec![]);
                for label in [
                    "Starting page",
                    "Save in folder",
                    "Save offline copy",
                    "Offline website",
                    "×",
                ] {
                    let rect = text_rect(&out, label);
                    assert!(
                        rect.min.x >= 0.0
                            && rect.min.y >= 0.0
                            && rect.max.x <= size.x
                            && rect.max.y <= size.y,
                        "{label}: {rect:?}"
                    );
                }
            }
        }
    }
}
