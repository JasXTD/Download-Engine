use super::*;
use download_engine::external_open::{Inbox, Pending, Target};
use egui::Stroke;

#[derive(Default)]
pub(super) struct State {
    reading: Option<background_io::Task<Inbox>>,
    dismissing: Option<background_io::Task<()>>,
    ready: Option<Pending>,
    active: Option<Pending>,
    next_read: Option<Instant>,
    ignore_read: bool,
    registration: Option<download_engine::torrent_shell::Status>,
    registration_task: Option<background_io::Task<download_engine::torrent_shell::Status>>,
    registration_error: Option<String>,
}
impl Desktop {
    pub(super) fn offer_browser_torrent_file(&mut self, job: &Job) {
        if job.status != Status::Complete
            || job.torrent.is_some()
            || (job.source_page.is_none() && !job.request_id.starts_with("capture:"))
        {
            return;
        }
        let Some(path) = job.saved_path.as_ref().filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
        }) else {
            return;
        };
        if let Err(error) = self
            .store
            .submit_external_opens(&[Target::TorrentFile(path.clone())])
        {
            self.notice = Some(Notice::error(error.to_string()));
        }
    }
    #[cfg(feature = "screenshot")]
    pub(super) fn sample_external_torrent(&mut self, target: Target) {
        self.external_open.active = Some(Pending {
            id: "screenshot-only".into(),
            target,
        });
    }
    #[cfg(feature = "screenshot")]
    pub(super) fn external_open_capture_ready(&self) -> bool {
        self.external_open.reading.is_none()
            && self.external_open.dismissing.is_none()
            && self.external_open.registration_task.is_none()
    }
    pub(super) fn torrent_windows_setup(&mut self, ui: &mut egui::Ui, p: Palette) {
        use download_engine::torrent_shell::{Action, Status};
        if !cfg!(windows) {
            return;
        }
        if let Some(result) = self
            .external_open
            .registration_task
            .as_mut()
            .and_then(background_io::Task::poll)
        {
            self.external_open.registration_task = None;
            match result {
                Ok(status) => {
                    self.external_open.registration = Some(status);
                    self.external_open.registration_error = None;
                }
                Err(error) => self.external_open.registration_error = Some(format!("{error:#}")),
            }
        }
        ui.add_space(16.0);
        ui.label(
            RichText::new(tr("OPEN FROM WINDOWS"))
                .small()
                .color(p.muted),
        );
        ui.add_space(5.0);
        let mut action = None;
        let section = egui::Frame::new()
            .fill(p.background)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(8)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_min_width((ui.available_width() - 1.0).max(0.0));
                ui.label(RichText::new(tr("Torrent files and magnet links")).strong());
                let explanation = tr("Choose Download Engine in Windows Default Apps. Each item opens for review before downloading.");
                ui.label(RichText::new(explanation).small().color(p.muted));
                ui.add_space(8.0);
                let current = matches!(self.external_open.registration, Some(Status::ThisCopy));
                match &self.external_open.registration {
                    Some(Status::ThisCopy) => {
                        ui.label(RichText::new(tr("This copy is registered")).color(p.accent));
                    }
                    Some(Status::OtherCopy(path)) => {
                        ui.label(tr("Another copy is registered:"));
                        let path = path.display().to_string();
                        let mut text = path.as_str();
                        ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY));
                    }
                    Some(Status::NotRegistered) => { ui.label(tr("Not registered")); }
                    None => { ui.label(tr("Checking registration…")); }
                }
                let busy = self.external_open.registration_task.is_some();
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let label = if matches!(self.external_open.registration, Some(Status::OtherCopy(_))) {
                            tr("Use this copy")
                        } else {
                            tr("Register this copy")
                        };
                        if !current && ui.button(label).clicked() {
                            action = Some(Action::Register);
                        }
                        if current && ui.button(tr("Choose defaults in Windows")).clicked() {
                            action = Some(Action::Defaults);
                        }
                        if current && ui.button(tr("Remove registration")).clicked() {
                            action = Some(Action::Remove);
                        }
                        if ui.button(tr("Refresh")).clicked() {
                            action = Some(Action::Refresh);
                        }
                    });
                });
                let immediate = tr("Registration applies immediately. Your default apps stay unchanged until you choose them in Windows.");
                ui.label(RichText::new(immediate).small().color(p.muted));
                if let Some(error) = &self.external_open.registration_error {
                    ui.colored_label(p.warning, error);
                }
            });
        // The screenshot fixture scrolls the real Preferences surface to the
        // new controls. Normal app navigation keeps the user's scroll position.
        #[cfg(feature = "screenshot")]
        if std::env::var("IDM_EX_TEST_FEATURE").as_deref() == Ok("torrent-windows-preferences") {
            ui.scroll_to_rect(section.response.rect, Some(egui::Align::Center));
        }
        #[cfg(not(feature = "screenshot"))]
        let _ = section;
        if action.is_none()
            && self.external_open.registration.is_none()
            && self.external_open.registration_error.is_none()
            && self.external_open.registration_task.is_none()
        {
            action = Some(Action::Refresh);
        }
        if let Some(action) = action {
            match background_io::Task::spawn(Some(ui.ctx().clone()), move || {
                download_engine::torrent_shell::apply(action)
            }) {
                Ok(task) => self.external_open.registration_task = Some(task),
                Err(error) => self.external_open.registration_error = Some(error.to_string()),
            }
        }
    }
    pub(super) fn external_open_origin(&self) -> Option<&'static str> {
        self.external_open
            .active
            .as_ref()
            .map(|request| match request.target {
                Target::TorrentFile(_) => "Opened from Windows",
                Target::Magnet(_) => "Opened from a magnet link",
            })
    }
    pub(super) fn poll_external_opens(&mut self, ctx: &egui::Context) {
        let state = &mut self.external_open;
        if let Some(result) = state
            .dismissing
            .as_mut()
            .and_then(background_io::Task::poll)
        {
            state.dismissing = None;
            match result {
                Ok(()) => {
                    state.active = None;
                    state.ready = None;
                    state.next_read = None;
                }
                Err(error) => {
                    self.notice = Some(Notice::error(error.to_string()));
                    state.next_read = Some(Instant::now() + Duration::from_secs(2));
                }
            }
        }
        // Retain ownership until durable dismissal succeeds. In particular, a
        // closed dialog must not let a stale polling result reopen the request.
        if self.dialog != Some(Dialog::Add)
            && let Some(active) = state.active.as_ref()
        {
            if state.dismissing.is_none() && state.next_read.is_none_or(|at| Instant::now() >= at) {
                let store = self.store.clone();
                let id = active.id.clone();
                state.ignore_read = state.reading.is_some();
                match background_io::Task::spawn(Some(ctx.clone()), move || {
                    store.dismiss_external_open(&id)
                }) {
                    Ok(task) => state.dismissing = Some(task),
                    Err(error) => {
                        self.notice = Some(Notice::error(error.to_string()));
                        state.next_read = Some(Instant::now() + Duration::from_secs(2));
                    }
                }
            }
            return;
        }
        if let Some(result) = state.reading.as_mut().and_then(background_io::Task::poll) {
            state.reading = None;
            if std::mem::take(&mut state.ignore_read) {
                state.next_read = None;
            } else {
                match result {
                    Ok(inbox) => {
                        if state.active.is_none() {
                            state.ready = inbox.next;
                        }
                        if inbox.newly_received > 0 {
                            self.hide_on_start = false;
                            self.show_window(ctx);
                            if self.dialog.is_some() {
                                self.notice = Some(Notice::info(
                                    tr("A torrent is waiting. Finish this dialog to review it.")
                                        .into(),
                                ));
                            }
                        }
                    }
                    Err(error) => self.notice = Some(Notice::error(error.to_string())),
                }
            }
        }
        if self.external_open.active.is_none()
            && self.dialog.is_none()
            && let Some(pending) = self.external_open.ready.take()
        {
            self.forget_torrent_draft();
            self.url = pending.target.text().expect("validated request").to_owned();
            self.name.clear();
            self.expected_sha256.clear();
            self.add_authentication = false;
            self.add_login = add_login::Draft::default();
            self.add_ftp_security = None;
            self.open_add();
            self.external_open.active = Some(pending);
            self.show_window(ctx);
        }
        let state = &mut self.external_open;
        if state.reading.is_none()
            && state.dismissing.is_none()
            && state.next_read.is_none_or(|at| Instant::now() >= at)
        {
            state.next_read = Some(Instant::now() + Duration::from_secs(1));
            let store = self.store.clone();
            match background_io::Task::spawn(Some(ctx.clone()), move || {
                store.receive_external_opens()
            }) {
                Ok(task) => state.reading = Some(task),
                Err(error) => self.notice = Some(Notice::error(error.to_string())),
            }
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::fixture;
    use super::*;
    #[test]
    fn browser_saved_torrent_is_a_deduplicated_review_offer_with_torrents_off() {
        check_browser_torrent_offer(false);
    }
    #[test]
    fn captured_torrent_without_a_referrer_still_opens_for_review() {
        check_browser_torrent_offer(true);
    }
    fn check_browser_torrent_offer(captured: bool) {
        let (dir, mut app, ctx) = fixture();
        let before = app.store.list().unwrap().len();
        let mut job = app
            .store
            .new_download(
                "https://example.test/sample.torrent",
                Some("sample.torrent"),
                Some(dir.path()),
                "browser-torrent-fixture".into(),
            )
            .unwrap();
        job.status = Status::Complete;
        job.saved_path = Some(dir.path().join("sample.torrent"));
        app.offer_browser_torrent_file(&job);
        assert_eq!(
            app.store.receive_external_opens().unwrap().count,
            0,
            "ordinary downloads are not browser offers"
        );
        if captured {
            job.request_id = "capture:browser-torrent-fixture".into();
        } else {
            job.source_page = Some("https://example.test/page".into());
        }
        app.offer_browser_torrent_file(&job);
        app.offer_browser_torrent_file(&job);
        assert_eq!(app.store.receive_external_opens().unwrap().count, 1);
        settle(&mut app, &ctx, |a| a.external_open.active.is_some());
        assert_eq!(app.dialog, Some(Dialog::Add));
        assert!(!app.store.torrent_settings().unwrap().enabled);
        assert!(!app.store.torrents.is_running());
        assert_eq!(app.store.list().unwrap().len(), before);
    }
    fn settle(app: &mut Desktop, ctx: &egui::Context, predicate: impl Fn(&Desktop) -> bool) {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            app.poll_external_opens(ctx);
            if predicate(app) {
                return;
            }
            assert!(Instant::now() < until, "external review did not settle");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn external_review_preserves_open_dialog_and_resets_old_credentials() {
        let (dir, mut app, ctx) = fixture();
        let target = Target::parse(std::ffi::OsStr::new("external.torrent"), dir.path()).unwrap();
        let submitted = app
            .store
            .submit_external_opens(std::slice::from_ref(&target))
            .unwrap();
        app.dialog = Some(Dialog::Batch);
        app.name = "old name".into();
        app.expected_sha256 = "old checksum".into();
        app.add_authentication = true;
        settle(&mut app, &ctx, |a| a.external_open.ready.is_some());
        assert_eq!(app.dialog, Some(Dialog::Batch));
        assert!(app.store.external_opens_received(&submitted).unwrap());
        app.dialog = None;
        settle(&mut app, &ctx, |a| a.external_open.active.is_some());
        assert_eq!(app.dialog, Some(Dialog::Add));
        assert_eq!(app.url, target.text().unwrap());
        assert!(app.name.is_empty());
        assert!(app.expected_sha256.is_empty());
        assert!(!app.add_authentication);
        assert!(!app.torrents.settings.enabled);
        app.dialog = None;
        settle(&mut app, &ctx, |a| a.external_open.active.is_none());
        assert_eq!(app.store.receive_external_opens().unwrap().count, 0);
    }
    #[test]
    fn cancel_discards_an_inflight_snapshot_instead_of_reopening_the_review() {
        let (dir, mut app, ctx) = fixture();
        let target = Target::parse(std::ffi::OsStr::new("external.torrent"), dir.path()).unwrap();
        app.store.submit_external_opens(&[target]).unwrap();
        settle(&mut app, &ctx, |a| a.external_open.active.is_some());
        // Deliver a snapshot taken before cancellation only after the original
        // request has been dismissed, reproducing the asynchronous race.
        let stale = app.store.receive_external_opens().unwrap();
        let (send, recv) = std::sync::mpsc::channel();
        app.external_open.reading = Some(
            background_io::Task::spawn(None, move || {
                recv.recv_timeout(Duration::from_secs(5))?;
                Ok(stale)
            })
            .unwrap(),
        );
        app.external_open.next_read = None;
        app.dialog = None;
        settle(&mut app, &ctx, |a| a.external_open.active.is_none());
        send.send(()).unwrap();
        settle(&mut app, &ctx, |a| !a.external_open.ignore_read);
        assert!(app.dialog.is_none());
        assert!(app.external_open.ready.is_none());
        assert_eq!(app.store.receive_external_opens().unwrap().count, 0);
    }
}
