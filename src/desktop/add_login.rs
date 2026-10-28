//! Login entered in Add download; a protected job stays paused until the vault save succeeds.
use super::*;

pub(super) struct Draft {
    pub method: AuthKind,
    pub username: Zeroizing<String>,
    pub secret: Zeroizing<String>,
    pub job: Option<Job>,
    pub request: Option<Uuid>,
    pub reveal: bool,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            method: AuthKind::Basic,
            username: Zeroizing::new(String::new()),
            secret: Zeroizing::new(String::new()),
            job: None,
            request: None,
            reveal: false,
        }
    }
}

impl Draft {
    pub fn credentials(&self, ftp: bool) -> anyhow::Result<Credentials> {
        let credentials = match self.method {
            AuthKind::Basic => {
                Credentials::basic(self.username.to_string(), self.secret.to_string())
            }
            kind @ (AuthKind::Ntlm | AuthKind::Negotiate) if !ftp => {
                Credentials::enterprise(kind, self.username.to_string(), self.secret.to_string())
            }
            AuthKind::Bearer if !ftp => Credentials::bearer(self.secret.to_string()),
            _ => anyhow::bail!(tr("Use a username and password for FTPS.")),
        };
        credentials.validate()?;
        Ok(credentials)
    }
}

impl Desktop {
    pub(super) fn add_login_fields(&mut self, ui: &mut egui::Ui, p: Palette) -> bool {
        let mut editing = false;
        let ftp = download_engine::ftp::is_url(&self.url);
        let previous = self.add_login.method;
        if !ftp {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(
                    &mut self.add_login.method,
                    AuthKind::Basic,
                    tr("Username / password"),
                );
                ui.selectable_value(
                    &mut self.add_login.method,
                    AuthKind::Bearer,
                    tr("Bearer token"),
                );
                ui.selectable_value(&mut self.add_login.method, AuthKind::Ntlm, "NTLM");
                ui.selectable_value(&mut self.add_login.method, AuthKind::Negotiate, "Negotiate");
            });
        }
        if previous != self.add_login.method {
            self.add_login.username = Zeroizing::new(String::new());
            self.add_login.secret = Zeroizing::new(String::new());
        }
        let focused = ui.memory(|memory| memory.focused());
        if matches!(
            self.add_login.method,
            AuthKind::Basic | AuthKind::Ntlm | AuthKind::Negotiate
        ) {
            ui.columns(2, |columns| {
                columns[0].label(if self.add_login.method == AuthKind::Basic {
                    tr("Username")
                } else {
                    tr("Domain\\username or user@domain")
                });
                let mut username = view::line_edit(&mut *self.add_login.username)
                    .id(egui::Id::new("add-login-username"))
                    .char_limit(512)
                    .vertical_align(egui::Align::Center)
                    .desired_width(f32::INFINITY)
                    .show(&mut columns[0]);
                editing |= focused == Some(username.response.id);
                username.state.clear_undoer();
                username.state.store(columns[0].ctx(), username.response.id);
                columns[1].label(tr("Password"));
                let mut password = view::line_edit(&mut *self.add_login.secret)
                    .id(egui::Id::new("add-login-secret"))
                    .password(true)
                    .char_limit(2048)
                    .vertical_align(egui::Align::Center)
                    .desired_width(f32::INFINITY)
                    .show(&mut columns[1]);
                editing |= focused == Some(password.response.id);
                password.state.clear_undoer();
                password.state.store(columns[1].ctx(), password.response.id);
            });
        } else {
            ui.label(tr("Token (without the Bearer prefix)"));
            let mut token = view::line_edit(&mut *self.add_login.secret)
                .id(egui::Id::new("add-login-secret"))
                .password(true)
                .char_limit(2048)
                .vertical_align(egui::Align::Center)
                .desired_width(f32::INFINITY)
                .show(ui);
            editing |= focused == Some(token.response.id);
            token.state.clear_undoer();
            token.state.store(ui.ctx(), token.response.id);
        }
        let help = ui.label(
            RichText::new(if matches!(self.add_login.method, AuthKind::Ntlm | AuthKind::Negotiate) {
                tr("For this server only. Choose Direct or a Custom HTTP proxy in Preferences → Connection → Proxy. Saved in Windows Credential Manager.")
            } else { tr("For this file server only. Saved in Windows Credential Manager.") })
                .small()
                .color(p.muted),
        );
        if std::mem::take(&mut self.add_login.reveal) {
            help.scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
            ui.memory_mut(|memory| {
                memory.request_focus(egui::Id::new(
                    if matches!(
                        self.add_login.method,
                        AuthKind::Basic | AuthKind::Ntlm | AuthKind::Negotiate
                    ) {
                        "add-login-username"
                    } else {
                        "add-login-secret"
                    },
                ))
            });
        }
        editing
    }

    pub(super) fn submit_add_login(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !download_engine::hls::is_playlist_url(&self.url)
                && !download_engine::hls::is_site_video(&self.url)
                && !download_engine::mega::is_url(&self.url)
                && !download_engine::wetransfer::is_url(&self.url),
            tr("Use login is available for direct file links only.")
        );
        let credentials = self
            .add_login
            .credentials(download_engine::ftp::is_url(&self.url))?;
        if let Some(job) = self.add_login.job.clone() {
            self.request_add_credentials(job, credentials)
        } else {
            self.start_add_worker(Some(credentials))
        }
    }

    pub(super) fn request_add_credentials(
        &mut self,
        job: Job,
        credentials: Credentials,
    ) -> anyhow::Result<()> {
        let request = Uuid::new_v4();
        self.scheduler.send(Command::Authentication(auth::Update {
            request,
            job: job.id,
            expected_url: job.url,
            credentials: Some(credentials),
            restart_confirmed: false,
            resume: true,
        }))?;
        self.add_login.request = Some(request);
        Ok(())
    }

    pub(super) fn add_login_event(&mut self, event: auth::Event) {
        if self.add_login.request != Some(event.request) {
            return;
        }
        self.add_login.request = None;
        match event.result {
            Err(error) => {
                self.form_error = Some(error);
                self.add_login.reveal = true;
            }
            Ok(warning) => {
                if let Some(job) = self.add_login.job.take() {
                    self.finish_add_download(job);
                    self.notice = warning.map(Notice::error);
                }
            }
        }
    }
}
