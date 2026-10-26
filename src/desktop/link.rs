//! Replacement-link checks and their cancellation/apply lifecycle.
use super::*;

impl Desktop {
    pub(super) fn open_link(&mut self, job: &Job) {
        if job.hls.is_some() {
            self.open_hls(&job.url);
            return;
        }
        self.dialog = Some(Dialog::UpdateLink);
        self.form_error = None;
        self.focus_url = true;
        self.link = Some(LinkDialog {
            job: job.id.clone(),
            name: job.name.clone(),
            url: String::new(),
            request: None,
            state: LinkState::Editing,
            page: job.source_page.clone().unwrap_or_default(),
            browser: None,
            browser_poll: Instant::now(),
        });
    }

    pub(super) fn cancel_link_check(&mut self) {
        let request = self.link.as_mut().and_then(|link| {
            link.browser = None;
            link.state = LinkState::Editing;
            link.request.take()
        });
        if let Some(request) = request {
            self.command(Command::Link(LinkCommand::Cancel(request)));
        }
    }

    pub(super) fn check_link(&mut self) {
        let Some(link) = &mut self.link else {
            return;
        };
        if !matches!(link.state, LinkState::Editing) {
            return;
        }
        let result = validate_url(&link.url).and_then(|url| {
            let request = Uuid::new_v4();
            self.scheduler.send(Command::Link(LinkCommand::Check {
                request,
                job: link.job.clone(),
                url: url.to_string(),
            }))?;
            link.request = Some(request);
            link.state = LinkState::Checking {
                checked: 0,
                total: 0,
            };
            Ok(())
        });
        self.form_error = result.err().map(|error| error.to_string());
    }

    pub(super) fn link_event(&mut self, event: LinkEvent) {
        let request = match &event {
            LinkEvent::Progress { request, .. }
            | LinkEvent::Checked { request, .. }
            | LinkEvent::Applied { request, .. }
            | LinkEvent::Failed { request, .. } => *request,
        };
        let Some(link) = &mut self.link else {
            return;
        };
        if link.request != Some(request) {
            return;
        }
        match event {
            LinkEvent::Progress { checked, total, .. } => {
                link.state = LinkState::Checking { checked, total }
            }
            LinkEvent::Checked { outcome, .. } => link.state = LinkState::Ready(outcome),
            LinkEvent::Failed { message, .. } => {
                link.browser = None;
                link.request = None;
                link.state = LinkState::Editing;
                self.form_error = Some(message);
            }
            LinkEvent::Applied { warning, .. } => {
                self.link = None;
                self.dialog = None;
                self.form_error = None;
                self.notice = Some(Notice::outcome(
                    warning,
                    tr("Link updated. The download will continue when its queue has a free slot.")
                        .into(),
                ));
                self.last_refresh = Instant::now() - Duration::from_secs(1);
            }
        }
    }

    pub(super) fn link_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(link) = &mut self.link else {
            self.dialog = None;
            return;
        };
        let mut check = false;
        let mut change = false;
        let mut cancel = false;
        let mut apply = None;
        let mut browse = false;
        let applying = matches!(link.state, LinkState::Applying);
        let response = form_dialog::modal("update-download-link", p).show(ctx, |ui| {
                ui.set_width(480.0);
                cancel = form_dialog::title(ui, tr("Update download link"), form_dialog::Icon::Symbol("\u{e71b}"), p, !applying);
                ui.add(egui::Label::new(RichText::new(&link.name).strong()).truncate()).on_hover_text(&link.name);
                ui.label(RichText::new(tr("Keep the filename, save folder, and queue position.")).small().color(p.muted));
                ui.add_space(12.0);
                ui.label(tr("New file URL"));
                let input = ui.add_enabled(matches!(link.state, LinkState::Editing) && link.browser.is_none(),
                    view::line_edit(&mut link.url).desired_width(f32::INFINITY)
                        .hint_text(tr("Paste a fresh HTTP or HTTPS download link")));
                if self.focus_url { input.request_focus(); self.focus_url = false; }
                ui.add_space(8.0);
                match &link.state {
                    LinkState::Editing => {
                        ui.label(tr("We will compare the saved data with the new link before resuming. This uses download bandwidth and may take a while."));
                        ui.add_space(8.0);
                        if link.browser.is_some() {
                            ui.horizontal(|ui| {ui.spinner();ui.label(tr("Waiting for the fresh browser download…"));});
                            ui.label(tr("Click the same file on the page. The extension pauses the matching download for review here. If needed, open its popup and allow recovery access."));
                            if ui.button(tr("Stop waiting")).clicked() {link.browser=None;}
                        } else {
                            egui::CollapsingHeader::new(tr("Find a fresh link in the browser")).default_open(true).show(ui,|ui| {
                                ui.label(tr("Original webpage"));
                                ui.add(view::line_edit(&mut link.page).hint_text("https://example.com/download-page").desired_width(f32::INFINITY).char_limit(16384));
                                browse=ui.button(tr("Open page and wait for link")).clicked();
                                ui.label(RichText::new(tr("Use the same browser with Download Engine's extension. Recovery keeps the existing download and asks before resuming.")).small().color(p.muted));
                            });
                        }
                    }
                    LinkState::Checking { checked, total } => {
                        ui.horizontal(|ui| { ui.spinner(); ui.label(tr("Checking replacement link…")); });
                        if *total > 0 {
                            ui.add(egui::ProgressBar::new(*checked as f32 / *total as f32)
                                .text(tf!("{} of {} checked", bytes(*checked), bytes(*total))));
                        }
                        ui.label(RichText::new(tr("Your original link and saved data are kept until you confirm.")).small().color(p.muted));
                    }
                    LinkState::Ready(LinkOutcome::Resume { saved, total }) => {
                        ui.label(RichText::new(tr("Saved data matches")).strong().color(p.accent));
                        ui.label(tf!("All {} of saved data were verified against the new link. Continue downloading the {} file.", bytes(*saved), bytes(*total)));
                    }
                    LinkState::Ready(LinkOutcome::Fresh) => {
                        ui.label(tr("The link is available. There is no saved data to resume; the file will start from the beginning."));
                    }
                    LinkState::Ready(LinkOutcome::RestartRequired(reason)) => {
                        ui.label(RichText::new(tr("Restart required")).strong().color(p.warning));
                        ui.label(reason);
                        ui.label(tr("Updating will discard this download's partial data and start from the beginning. Completed files are kept."));
                    }
                    LinkState::Applying => { ui.horizontal(|ui| { ui.spinner(); ui.label(tr("Updating download…")); }); }
                }
                if let Some(error) = &self.form_error { ui.label(RichText::new(error).small().color(p.warning)); }
                form_dialog::controls(ui, |ui| {
                    match &link.state {
                        LinkState::Editing => { check = ui.add_enabled_ui(link.browser.is_none(), |ui| form_dialog::primary_button(ui, tr("Check link"))).inner.clicked(); }
                        LinkState::Ready(outcome) => {
                            let (label, restart) = match outcome {
                                LinkOutcome::Resume { .. } => (tr("Update and resume"), false),
                                LinkOutcome::Fresh => (tr("Update and start"), false),
                                LinkOutcome::RestartRequired(_) => (tr("Update and restart"), true),
                            };
                            if form_dialog::primary_button(ui, label).clicked() { apply = Some(restart); }
                            change = form_dialog::secondary_button(ui, tr("Change link")).clicked();
                        }
                        _ => {}
                    }
                });
            });
        if !applying && (cancel || response.should_close()) {
            self.cancel_link_check();
            self.link = None;
            self.dialog = None;
        } else if change {
            self.cancel_link_check();
            self.form_error = None;
            self.focus_url = true;
        } else if browse {
            if let Some(link) = &mut self.link {
                match self.store.begin_browser_recovery(&link.job, &link.page) {
                    Ok(lease) => {
                        self.form_error = None;
                        link.browser = Some(lease);
                        ctx.open_url(egui::OpenUrl::new_tab(link.page.clone()));
                    }
                    Err(error) => self.form_error = Some(error.to_string()),
                }
            }
        } else if check {
            if let Some(link) = &mut self.link {
                link.browser = None;
            }
            self.check_link();
        } else if let Some(restart) = apply
            && let Some(link) = &mut self.link
            && let Some(request) = link.request
        {
            match self
                .scheduler
                .send(Command::Link(LinkCommand::Apply { request, restart }))
            {
                Ok(()) => link.state = LinkState::Applying,
                Err(error) => self.form_error = Some(error.to_string()),
            }
        }
    }

    pub(super) fn poll_browser_recovery(&mut self, ctx: &egui::Context) {
        let Some(link) = &mut self.link else { return };
        if link.browser_poll.elapsed() < Duration::from_secs(1) {
            return;
        }
        let Some(lease) = &link.browser else { return };
        link.browser_poll = Instant::now();
        let result = lease.refresh();
        match result {
            Ok(session)
                if session.state == "offered" && matches!(link.state, LinkState::Editing) =>
            {
                let request = Uuid::new_v4();
                match self
                    .scheduler
                    .send(Command::Link(LinkCommand::CheckBrowser {
                        request,
                        recovery: session.id,
                    })) {
                    Ok(()) => {
                        link.url = session.candidate.map(|c| c.url).unwrap_or_default();
                        link.request = Some(request);
                        link.state = LinkState::Checking {
                            checked: 0,
                            total: 0,
                        };
                        self.form_error = None;
                        self.show_window(ctx);
                    }
                    Err(error) => {
                        link.browser = None;
                        self.form_error = Some(error.to_string());
                    }
                }
            }
            Ok(session) if session.state == "cancelled" => {
                self.cancel_link_check();
                self.form_error=Some(tr("Browser recovery ended. The browser copy is released; find a fresh link again when ready.").into());
            }
            Err(error) => {
                self.cancel_link_check();
                self.form_error = Some(tf!(
                    "Browser recovery could not continue: {error}",
                    error = error
                ));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::desktop::interaction_tests::{click, fixture};
    #[test]
    fn browser_recovery_dialog_starts_only_on_request_and_cancels_its_intent_on_close() {
        let (_root, mut app, ctx) = fixture();
        let mut job = app.jobs[0].clone();
        job.source_page = Some("https://source.test/downloads".into());
        app.store.enqueue(&job).unwrap();
        app.open_link(&job);
        assert!(app.store.pending_browser_recovery().unwrap().is_none());
        assert_eq!(
            app.link.as_ref().unwrap().page,
            "https://source.test/downloads"
        );
        click(&mut app, &ctx, "Open page and wait for link");
        let session = app.store.pending_browser_recovery().unwrap().unwrap();
        assert_eq!(session.job_id, job.id);
        click(&mut app, &ctx, "×");
        assert!(app.link.is_none());
        assert_eq!(
            app.store
                .browser_recovery_status(session.id)
                .unwrap()
                .unwrap()
                .state,
            "cancelled"
        );
        assert_eq!(app.store.get(&job.id).unwrap().unwrap().url, job.url);
    }
}
