use super::*;
use download_engine::updates::{self as service, Release};
use std::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct Task {
    cancel: CancellationToken,
    result: mpsc::Receiver<anyhow::Result<Release>>,
}
impl Drop for Task {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[derive(Default)]
pub(super) struct State {
    automatic: bool,
    task: Option<Task>,
    release: Option<Release>,
    error: Option<String>,
}
impl State {
    fn start(&mut self, store: Store, ctx: egui::Context) {
        if self.task.is_some() {
            return;
        }
        self.release = None;
        self.error = None;
        let (sender, result) = mpsc::channel();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        match std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let loaded = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(anyhow::Error::from)
                    .and_then(|runtime| runtime.block_on(service::check(&store, &token)));
                let _ = sender.send(loaded);
                ctx.request_repaint();
            }) {
            Ok(_) => self.task = Some(Task { cancel, result }),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    fn poll(&mut self) {
        let Some(task) = &self.task else {
            return;
        };
        let result = match task.result.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(anyhow::anyhow!("Update worker stopped")),
        };
        self.task = None;
        match result {
            Ok(release) => self.release = Some(release),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    #[cfg(any(test, feature = "screenshot"))]
    fn sample(mode: &str) -> Self {
        if mode == "updates-error" {
            return Self {
                error: Some("Could not connect to GitHub".into()),
                ..Self::default()
            };
        }
        let available = matches!(mode, "updates-available" | "updates-launch");
        let version = if available {
            "99.0.0"
        } else {
            env!("CARGO_PKG_VERSION")
        };
        Self {
            automatic: mode == "updates-launch",
            release: Some(Release {
                version: version.into(),
                page_url: format!("{}/releases/tag/v{version}", service::REPOSITORY_URL),
                newer: available,
            }),
            ..Self::default()
        }
    }
}

impl Desktop {
    /// Called once by the desktop entry point, independently of Windows startup.
    pub fn check_updates_on_launch(&mut self, ctx: &egui::Context) {
        if !self.interface.check_updates_on_launch {
            return;
        }
        // Native UI captures use synthetic release results and never contact GitHub.
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
            || std::env::var_os("EFRAME_SCREENSHOT_TO").is_some()
        {
            return;
        }
        let mut state = State {
            automatic: true,
            ..State::default()
        };
        state.start(self.store.clone(), ctx.clone());
        self.extras.launch_update_check = Some(state);
    }

    pub(super) fn poll_launch_updates(&mut self, ctx: &egui::Context) {
        if !self.interface.check_updates_on_launch || self.exit_requested {
            self.extras.launch_update_check = None;
            return;
        }
        let Some(state) = &mut self.extras.launch_update_check else {
            return;
        };
        state.poll();
        let available = state.release.as_ref().is_some_and(|release| release.newer);
        if state.task.is_none() && !available {
            // An offline launch, failed check, or current version needs no popup.
            self.extras.launch_update_check = None;
        } else if available
            && self.dialog.is_none()
            && self.preferences_window.is_none()
            && self.extras.exit_deadline.is_none()
            && self.extras.shutdown_deadline.is_none()
            && !egui::Popup::is_any_open(ctx)
            && ctx.input(|i| {
                i.viewport().focused != Some(false) && i.viewport().minimized != Some(true)
            })
        {
            self.extras.update_check = self.extras.launch_update_check.take();
            self.dialog = Some(Dialog::CheckUpdates);
        }
    }

    pub(super) fn open_updates(&mut self) {
        if self.dialog == Some(Dialog::CheckUpdates) {
            return;
        }
        // A manual check adopts the launch worker/result instead of issuing a duplicate request.
        let mut state = self.extras.launch_update_check.take().unwrap_or_default();
        state.automatic = false;
        self.extras.update_check = Some(state);
        self.dialog = Some(Dialog::CheckUpdates);
    }

    #[cfg(feature = "screenshot")]
    pub(super) fn updates_sample(&mut self, mode: &str) {
        self.extras.update_check = Some(State::sample(mode));
        self.dialog = Some(Dialog::CheckUpdates);
    }

    pub(super) fn updates_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let state = self.extras.update_check.get_or_insert_with(State::default);
        state.poll();
        if state.task.is_none() && state.release.is_none() && state.error.is_none() {
            state.start(self.store.clone(), ctx.clone());
        }
        let (mut close, mut retry) = (false, false);
        let response = form_dialog::modal("check-updates", p).show(ctx, |ui| {
            close = form_dialog::header(
                ui,
                if state.automatic {
                    tr("A new version is available")
                } else {
                    tr("Check for updates")
                },
                "Download Engine",
                form_dialog::Icon::Download,
                p,
                true,
            );
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("Installed version"));
                ui.strong(env!("CARGO_PKG_VERSION"));
            });
            ui.add_space(8.0);
            if state.task.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr("Checking GitHub…"));
                });
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            if let Some(release) = &state.release {
                if !state.automatic {
                    ui.strong(if release.newer {
                        tr("A new version is available")
                    } else {
                        tr("No newer release is available")
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(tr("Latest stable version"));
                    ui.strong(&release.version);
                });
            }
            if let Some(error) = &state.error {
                ui.colored_label(p.warning, tr("Unable to check for updates"));
                ui.add(egui::Label::new(error).wrap());
            }
            ui.add_space(10.0);
            ui.add(
                egui::Label::new(
                    RichText::new(tr(
                        "Download and install updates from the official GitHub release page.",
                    ))
                    .small()
                    .color(p.muted),
                )
                .wrap(),
            );
            if state.task.is_none() {
                form_dialog::controls(ui, |ui| {
                    if let Some(release) = &state.release
                        && form_dialog::primary_button(
                            ui,
                            if state.automatic {
                                tr("Get update")
                            } else {
                                tr("Open release page")
                            },
                        )
                        .clicked()
                    {
                        ctx.open_url(egui::OpenUrl::new_tab(&release.page_url));
                        close = state.automatic;
                    }
                    if state.automatic {
                        close |= form_dialog::secondary_button(ui, tr("Later")).clicked();
                    } else {
                        retry = form_dialog::secondary_button(ui, tr("Check again")).clicked();
                    }
                });
            }
        });
        if close || response.should_close() {
            self.extras.update_check = None;
            self.dialog = None;
        } else if retry {
            state.start(self.store.clone(), ctx.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::interaction_tests::{fixture, frame, text_rect};

    fn pending_launch(
        app: &mut Desktop,
    ) -> (mpsc::Sender<anyhow::Result<Release>>, CancellationToken) {
        let (sender, result) = mpsc::channel();
        let cancel = CancellationToken::new();
        app.extras.launch_update_check = Some(State {
            automatic: true,
            task: Some(Task {
                cancel: cancel.clone(),
                result,
            }),
            ..State::default()
        });
        (sender, cancel)
    }

    #[test]
    fn launch_check_waits_for_other_dialogs_and_only_prompts_for_new_releases() {
        let (_temp, mut app, ctx) = fixture();
        assert!(app.interface.check_updates_on_launch);
        let selected = app.selected.clone();
        let (sender, _) = pending_launch(&mut app);
        app.dialog = Some(Dialog::About);
        sender
            .send(Ok(State::sample("updates-available").release.unwrap()))
            .unwrap();
        app.poll_launch_updates(&ctx);
        assert_eq!(app.dialog, Some(Dialog::About));
        assert!(app.extras.launch_update_check.is_some());
        app.dialog = None;
        app.poll_launch_updates(&ctx);
        assert_eq!(app.dialog, Some(Dialog::CheckUpdates));
        assert!(app.extras.update_check.as_ref().unwrap().automatic);
        assert!(app.extras.launch_update_check.is_none());
        assert_eq!(app.selected, selected);

        for result in [
            Ok(State::sample("updates-current").release.unwrap()),
            Err(anyhow::anyhow!("offline")),
        ] {
            app.dialog = None;
            app.extras.update_check = None;
            let (sender, _) = pending_launch(&mut app);
            sender.send(result).unwrap();
            app.poll_launch_updates(&ctx);
            assert!(app.dialog.is_none());
            assert!(app.extras.launch_update_check.is_none());
        }
    }

    #[test]
    fn manual_check_adopts_launch_worker_and_disabling_or_exiting_cancels_it() {
        let (_temp, mut app, ctx) = fixture();
        let (sender, cancel) = pending_launch(&mut app);
        app.open_updates();
        assert!(app.extras.launch_update_check.is_none());
        assert!(!cancel.is_cancelled());
        assert!(!app.extras.update_check.as_ref().unwrap().automatic);
        app.open_updates();
        assert!(
            !cancel.is_cancelled(),
            "Opening an existing check must not restart it"
        );
        sender
            .send(Ok(State::sample("updates-available").release.unwrap()))
            .unwrap();
        app.extras.update_check.as_mut().unwrap().poll();
        assert!(
            app.extras
                .update_check
                .as_ref()
                .unwrap()
                .release
                .as_ref()
                .unwrap()
                .newer
        );
        for exiting in [false, true] {
            app.dialog = None;
            app.extras.update_check = None;
            app.interface.check_updates_on_launch = exiting;
            app.exit_requested = exiting;
            let (sender, cancel) = pending_launch(&mut app);
            app.poll_launch_updates(&ctx);
            assert!(app.extras.launch_update_check.is_none());
            assert!(cancel.is_cancelled());
            assert!(sender.send(Err(anyhow::anyhow!("stale"))).is_err());
        }
    }

    #[test]
    fn launch_prompt_later_escape_and_update_preserve_downloads_and_do_not_reopen() {
        for dark in [false, true] {
            for action in ["Later", "Get update", "Escape"] {
                let (_temp, mut app, ctx) = fixture();
                app.interface.appearance = if dark {
                    Appearance::Dark
                } else {
                    Appearance::Light
                };
                app.configure_appearance(&ctx);
                let selected = app.selected.clone();
                let jobs = app.jobs.len();
                app.extras.launch_update_check = Some(State::sample("updates-launch"));
                let size = egui::vec2(860.0, 600.0);
                frame(&mut app, &ctx, size, vec![]);
                let output = frame(&mut app, &ctx, size, vec![]);
                for label in [
                    "A new version is available",
                    "Installed version",
                    "Latest stable version",
                    "Get update",
                    "Later",
                ] {
                    let rect = text_rect(&output, label);
                    assert!(
                        egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                        "{label}"
                    );
                }
                let mut opened = false;
                if action == "Escape" {
                    frame(
                        &mut app,
                        &ctx,
                        size,
                        vec![egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }],
                    );
                } else {
                    let pos = text_rect(&output, action).center();
                    for pressed in [true, false] {
                        let out = frame(
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
                        opened |= out.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::OpenUrl(url) if url.url == format!("{}/releases/tag/v99.0.0", service::REPOSITORY_URL)));
                    }
                }
                assert_eq!(opened, action == "Get update");
                assert!(app.dialog.is_none() && app.extras.update_check.is_none());
                frame(&mut app, &ctx, size, vec![]);
                assert!(
                    app.dialog.is_none(),
                    "Dismissed launch check must not reopen"
                );
                assert_eq!(app.selected, selected);
                assert_eq!(app.jobs.len(), jobs);
            }
        }
    }

    #[test]
    fn update_states_fit_and_open_only_the_release_page() {
        for dark in [false, true] {
            for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
                for (mode, expected) in [
                    ("updates-available", "A new version is available"),
                    ("updates-current", "No newer release is available"),
                    ("updates-error", "Unable to check for updates"),
                ] {
                    let (_temp, mut app, ctx) = fixture();
                    app.interface.appearance = if dark {
                        Appearance::Dark
                    } else {
                        Appearance::Light
                    };
                    app.configure_appearance(&ctx);
                    let selected = app.selected.clone();
                    app.dialog = Some(Dialog::CheckUpdates);
                    app.extras.update_check = Some(State::sample(mode));
                    frame(&mut app, &ctx, size, vec![]);
                    let output = frame(&mut app, &ctx, size, vec![]);
                    for label in [
                        "Check for updates",
                        "Installed version",
                        expected,
                        "Check again",
                    ] {
                        let rect = text_rect(&output, label);
                        assert!(
                            egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                            "{label}"
                        );
                    }
                    if mode != "updates-error" {
                        let pos = text_rect(&output, "Open release page").center();
                        for pressed in [true, false] {
                            let out = frame(
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
                            if !pressed {
                                assert!(out.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::OpenUrl(url) if url.url == app.extras.update_check.as_ref().unwrap().release.as_ref().unwrap().page_url)));
                            }
                        }
                    }
                    assert_eq!(app.selected, selected);
                    frame(
                        &mut app,
                        &ctx,
                        size,
                        vec![egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }],
                    );
                    assert!(app.dialog.is_none() && app.extras.update_check.is_none());
                }
            }
        }
    }

    #[test]
    fn closing_check_cancels_worker_and_discards_stale_result() {
        let (_temp, mut app, ctx) = fixture();
        app.open_updates();
        let (sender, result) = mpsc::channel();
        let cancel = CancellationToken::new();
        app.extras.update_check.as_mut().unwrap().task = Some(Task {
            cancel: cancel.clone(),
            result,
        });
        let size = egui::vec2(860.0, 600.0);
        frame(&mut app, &ctx, size, vec![]);
        let out = frame(&mut app, &ctx, size, vec![]);
        text_rect(&out, "Checking GitHub…");
        app.dialog = Some(Dialog::About);
        frame(&mut app, &ctx, size, vec![]);
        assert!(cancel.is_cancelled());
        assert!(sender.send(Err(anyhow::anyhow!("late result"))).is_err());
        app.open_updates();
        assert!(app.extras.update_check.as_ref().unwrap().error.is_none());
    }
}
