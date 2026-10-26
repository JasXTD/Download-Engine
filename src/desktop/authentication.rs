//! Explicit per-download authentication and correlated save results.
use super::*;

impl Desktop {
    pub(super) fn open_authentication(&mut self, job: &Job) {
        if job
            .ftp
            .as_ref()
            .is_some_and(|f| f.security == download_engine::ftp::Security::Plain)
        {
            self.notice = Some(Notice::error(
                tr("Saved logins require FTPS. Plain FTP supports anonymous downloads only.")
                    .into(),
            ));
            return;
        }
        if job.hls.is_some() {
            self.notice = Some(Notice::error(tr("Video streams currently use anonymous requests. Browser login is not supported for streams yet.").into()));
            return;
        }
        self.dialog = Some(Dialog::Authentication);
        self.form_error = None;
        self.authentication = Some(AuthenticationDialog {
            job: job.id.clone(),
            name: job.name.clone(),
            url: job.url.clone(),
            method: Some(
                job.authentication
                    .as_ref()
                    .map_or(AuthKind::Basic, |b| b.kind),
            ),
            username: Zeroizing::new(String::new()),
            secret: Zeroizing::new(String::new()),
            restart_needed: auth::needs_restart(&self.store, job),
            restart_confirmed: false,
            resume: true,
            request: None,
        });
    }

    pub(super) fn save_authentication(&mut self) {
        let Some(form) = &mut self.authentication else {
            return;
        };
        if form.request.is_some() {
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                !form.restart_needed || form.restart_confirmed,
                tr("Confirm restarting before changing this download's authentication.")
            );
            let credentials = match form.method {
                Some(AuthKind::Basic) => Some(Credentials::basic(
                    form.username.to_string(),
                    form.secret.to_string(),
                )),
                Some(kind @ (AuthKind::Ntlm | AuthKind::Negotiate)) => {
                    Some(Credentials::enterprise(
                        kind,
                        form.username.to_string(),
                        form.secret.to_string(),
                    ))
                }
                Some(AuthKind::Bearer) => Some(Credentials::bearer(form.secret.to_string())),
                Some(AuthKind::Browser) => anyhow::bail!(
                    "Sign in to the site and send this file again using Use this site\'s login in the browser extension."
                ),
                None => None,
            };
            if let Some(credentials) = &credentials {
                credentials.validate()?;
            }
            let request = Uuid::new_v4();
            self.scheduler.send(Command::Authentication(auth::Update {
                request,
                job: form.job.clone(),
                expected_url: form.url.clone(),
                credentials,
                restart_confirmed: form.restart_confirmed,
                resume: form.resume,
            }))?;
            form.request = Some(request);
            Ok(())
        })();
        self.form_error = result.err().map(|error| error.to_string());
    }

    pub(super) fn authentication_event(&mut self, event: auth::Event) {
        if self.add_login.request == Some(event.request) {
            self.add_login_event(event);
            return;
        }
        let Some(form) = &mut self.authentication else {
            return;
        };
        if form.request != Some(event.request) {
            return;
        }
        match event.result {
            Err(error) => {
                form.request = None;
                self.form_error = Some(error);
            }
            Ok(warning) => {
                let message = if form.resume {
                    tr(
                        "Authentication saved. The download will start when its queue has a free slot.",
                    )
                } else {
                    tr("Authentication saved. The download remains paused.")
                };
                self.notice = Some(Notice::outcome(warning, message.into()));
                self.authentication = None; // Clear the draft's managed secret buffers.
                self.dialog = None;
                self.form_error = None;
                self.last_refresh = Instant::now() - Duration::from_secs(1);
            }
        }
    }

    pub(super) fn authentication_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(form) = &mut self.authentication else {
            self.dialog = None;
            return;
        };
        let saving = form.request.is_some();
        let mut submit = false;
        let mut cancel = false;
        let response = form_dialog::modal("authentication", p).show(ctx, |ui| {
                ui.set_width(480.0);
                cancel = form_dialog::title(ui, tr("Authentication"), form_dialog::Icon::Symbol("\u{e72e}"), p, !saving);
                form_dialog::body(ui, "authentication-body", |ui| {
                ui.add(egui::Label::new(RichText::new(&form.name).strong()).truncate()).on_hover_text(&form.name);
                ui.label(RichText::new(tf!("Server: {}", auth::origin(&form.url).unwrap_or_else(|_| tr("Invalid URL").into()))).small().color(p.muted));
                ui.add_space(8.0);
                ui.add_enabled_ui(!saving, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let previous = form.method;
                        ui.selectable_value(&mut form.method, Some(AuthKind::Basic), tr("Username / password"));
                        if !download_engine::ftp::is_url(&form.url) {
                            ui.selectable_value(&mut form.method, Some(AuthKind::Bearer), tr("Bearer token"));
                            ui.selectable_value(&mut form.method, Some(AuthKind::Ntlm), "NTLM");
                            ui.selectable_value(&mut form.method, Some(AuthKind::Negotiate), "Negotiate");
                        }
                        ui.selectable_value(&mut form.method, None, tr("None"));
                        if form.method != previous {
                            form.username = Zeroizing::new(String::new());
                            form.secret = Zeroizing::new(String::new());
                        }
                    });
                    ui.add_space(8.0);
                    if matches!(form.method, Some(AuthKind::Basic | AuthKind::Ntlm | AuthKind::Negotiate)) {
                        ui.label(if form.method == Some(AuthKind::Basic) { tr("Username") } else { tr("Domain\\username or user@domain") });
                        let mut input = view::line_edit(&mut *form.username)
                            .id(egui::Id::new("authentication-username"))
                            .desired_width(f32::INFINITY).char_limit(512).show(ui);
                        input.state.clear_undoer();
                        input.state.store(ui.ctx(), input.response.id);
                        ui.label(tr("Password"));
                    } else if form.method == Some(AuthKind::Bearer) { ui.label(tr("Token (without the Bearer prefix)")); }
                    if form.method == Some(AuthKind::Browser) {
                        ui.label(tr("This download uses a protected browser login."));
                        ui.label(tr("To refresh it, sign in to the site and send the file again using Use this site's login in the extension. Select None to remove this saved login."));
                    } else if form.method.is_some() {
                        let mut input = view::line_edit(&mut *form.secret).password(true)
                            .id(egui::Id::new("authentication-secret"))
                            .desired_width(f32::INFINITY).char_limit(2048).show(ui);
                        // A masked TextEdit still keeps plain-text undo snapshots by default.
                        // Do not retain them after the frame or restore a previous login.
                        input.state.clear_undoer();
                        input.state.store(ui.ctx(), input.response.id);
                        ui.label(RichText::new(tr("Saved in Windows Credential Manager for this download and server. Removed when the download completes or you remove it.")).small().color(p.muted));
                        let protection = if download_engine::ftp::is_url(&form.url) {
                            tr("FTPS encrypts your login and file data. The server certificate must be valid.")
                        } else {
                            tr("Remote servers require HTTPS. Redirects to other servers are blocked.")
                        };
                        ui.label(RichText::new(protection).small().color(p.muted));
                    } else {
                        ui.label(tr("Remove saved credentials and download without authentication."));
                    }
                    if matches!(form.method, Some(AuthKind::Ntlm | AuthKind::Negotiate)) {
                        ui.label(RichText::new(tr("Windows uses only this login. Negotiate selects Kerberos when available, otherwise NTLM. Choose Direct or a Custom HTTP proxy in Preferences → Connection → Proxy.")).small().color(p.muted));
                    }
                    if form.restart_needed {
                        ui.add_space(8.0);
                        ui.label(RichText::new(tr("Changing authentication requires starting from the beginning.")).color(p.warning));
                        ui.square_checkbox(&mut form.restart_confirmed, tr("Discard this download's partial data and restart"));
                    }
                    ui.add_space(8.0);
                    ui.square_checkbox(&mut form.resume, tr("Continue the download after saving"));
                });
                if let Some(error) = &self.form_error { ui.label(RichText::new(error).small().color(p.warning)); }
                });
                form_dialog::controls(ui, |ui| {
                    let enabled = !saving && form.method != Some(AuthKind::Browser) && (!form.restart_needed || form.restart_confirmed);
                    submit = form_dialog::primary_enabled(ui, enabled, if form.resume { tr("Save and continue") } else { tr("Save") }).clicked();
                    if saving { ui.spinner(); }
                });
            });
        if !saving && (cancel || response.should_close()) {
            self.authentication = None;
            self.dialog = None;
            self.form_error = None;
        } else if submit {
            self.save_authentication();
        }
    }
}
