use super::*;
use download_engine::proxy::{self as service, Login, LoginChange, Mode, Settings};

#[derive(Clone, Copy, PartialEq)]
enum LoginChoice {
    Keep,
    Replace,
    Remove,
}
pub(super) struct Form {
    original: Settings,
    draft: Settings,
    login: LoginChoice,
    username: Zeroizing<String>,
    password: Zeroizing<String>,
    error: Option<String>,
}
impl Desktop {
    pub(super) fn open_proxy(&mut self) {
        if self.dialog.is_some() {
            return;
        }
        match self.store.proxy_settings() {
            Ok(settings) => {
                self.proxy = Some(Form {
                    login: if settings.credential.is_some() {
                        LoginChoice::Keep
                    } else {
                        LoginChoice::Remove
                    },
                    original: settings.clone(),
                    draft: settings,
                    username: Zeroizing::default(),
                    password: Zeroizing::default(),
                    error: None,
                });
                self.dialog = Some(Dialog::Proxy);
            }
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
    }
    pub(super) fn proxy_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(form) = &mut self.proxy else {
            self.dialog = None;
            return;
        };
        let (mut save, mut close) = (false, false);
        let response = form_dialog::modal("proxy-settings", p).show(ctx, |ui| {
                ui.set_width(480.0);
                close = form_dialog::title(ui, tr("Proxy settings"), form_dialog::Icon::Symbol("\u{e71b}"), p, true);
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-165.0).max(200.0)).show(ui, |ui| {
                    ui.label(tr("Choose a route for files, video streams and website scans."));
                    ui.horizontal_wrapped(|ui| {
                        for mode in [Mode::System, Mode::Direct, Mode::Custom] { ui.radio_value(&mut form.draft.mode, mode, tr(mode.label())); }
                    });
                    if form.draft.mode == Mode::Custom {
                        ui.add_space(6.0);
                        ui.label(tr("Proxy address"));
                        if ui.add(view::line_edit(&mut form.draft.endpoint).hint_text("https://proxy.example.com:8443").char_limit(2048).desired_width(f32::INFINITY)).changed() {
                            form.draft.allow_unencrypted_login = false;
                        }
                        ui.label(RichText::new("http:// · https:// · socks5:// (local DNS) · socks5h:// (proxy DNS)").small().color(p.muted));
                        egui::ComboBox::from_id_salt("proxy-auth-method").selected_text(tr(form.draft.authentication.label())).show_ui(ui, |ui| {
                            for method in [service::Authentication::Basic, service::Authentication::Ntlm, service::Authentication::Negotiate] {
                                ui.selectable_value(&mut form.draft.authentication, method, tr(method.label()));
                            }
                        });
                        if form.draft.authentication != service::Authentication::Basic {
                            ui.label(RichText::new(tr("Use a company HTTP proxy and domain login. Negotiate tries Kerberos, then NTLM.")).small().color(p.muted));
                        }
                        let previous = form.login;
                        ui.horizontal_wrapped(|ui| {
                            ui.radio_value(&mut form.login, LoginChoice::Remove, tr("No login"));
                            if form.original.credential.is_some() { ui.radio_value(&mut form.login, LoginChoice::Keep, tr("Keep saved login")); }
                            ui.radio_value(&mut form.login, LoginChoice::Replace, tr("Enter login"));
                        });
                        if previous != form.login {
                            form.username = Zeroizing::default(); form.password = Zeroizing::default();
                        }
                        if form.login == LoginChoice::Replace {
                            ui.label(tr("Proxy username"));
                            secret_input(ui, "proxy-username", &mut form.username, false, 512);
                            ui.label(tr("Proxy password"));
                            secret_input(ui, "proxy-password", &mut form.password, true, 2048);
                        }
                        if form.login != LoginChoice::Remove {
                            ui.label(RichText::new(tr("Saved in Windows Credential Manager, separately from website logins.")).small().color(p.muted));
                            if service::endpoint(&form.draft.endpoint).is_ok_and(|u| !service::encrypted_or_loopback(&u)) {
                                ui.colored_label(p.warning, if form.draft.authentication == service::Authentication::Basic { tr("This proxy sends the login without encryption. Use only a trusted proxy.") } else { tr("This HTTP proxy connection is unencrypted. Use only a trusted proxy.") });
                                ui.square_checkbox(&mut form.draft.allow_unencrypted_login, tr("Allow unencrypted login to this proxy"));
                            }
                        }
                        ui.label(RichText::new(tr("Connections fail if this proxy is unavailable.")).small().color(p.muted));
                    } else if form.draft.mode == Mode::System {
                        ui.label(tr("Use the operating system / environment proxy configuration."));
                    } else { ui.label(tr("Connect directly without an HTTP/SOCKS proxy.")); }
                    ui.add_space(8.0);
                    ui.label(RichText::new(tr("Pause and resume active downloads to use changed settings.")).color(p.muted));
                    ui.label(RichText::new(tr("FTP/FTPS connects directly. Use a proxy you trust.")).small().color(p.muted));
                    if let Some(error) = &form.error { ui.colored_label(p.warning, error); }
                });
                save = form_dialog::footer(ui, tr("Save proxy settings"), None, p);
            });
        if close || response.should_close() {
            self.proxy = None;
            self.dialog = None;
        } else if save {
            let change = match form.login {
                LoginChoice::Keep => LoginChange::Keep,
                LoginChoice::Remove => LoginChange::Remove,
                LoginChoice::Replace => LoginChange::Replace(Login::new(
                    form.username.to_string(),
                    form.password.to_string(),
                )),
            };
            match self
                .store
                .save_proxy_settings(&form.original, form.draft.clone(), change)
            {
                Ok(warning) => {
                    self.notice = Some(Notice::outcome(
                        warning,
                        tr("Proxy settings saved. New transfers will use this route.").into(),
                    ));
                    self.proxy = None;
                    self.dialog = None;
                }
                Err(error) => form.error = Some(error.to_string()),
            }
        }
    }
    #[cfg(feature = "screenshot")]
    pub(super) fn proxy_sample(&mut self, enterprise: bool) {
        self.open_proxy();
        if let Some(form) = &mut self.proxy {
            form.draft.mode = Mode::Custom;
            form.draft.endpoint = "https://proxy.example.com:8443".into();
            form.login = LoginChoice::Replace;
            *form.username = "download-user".into();
            *form.password = "synthetic-example".into();
            if enterprise {
                form.draft.endpoint = "http://proxy.example.com:8080".into();
                form.draft.authentication = service::Authentication::Negotiate;
                *form.username = "EXAMPLE\\download-user".into();
            }
        }
    }
}
fn secret_input(ui: &mut egui::Ui, id: &str, value: &mut String, password: bool, limit: usize) {
    let mut input = view::line_edit(value)
        .id(egui::Id::new(id))
        .password(password)
        .char_limit(limit)
        .desired_width(f32::INFINITY)
        .show(ui);
    input.state.clear_undoer();
    input.state.store(ui.ctx(), input.response.id);
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::{click, fixture, frame, text_rect};
    use super::*;
    #[test]
    fn proxy_settings_validate_cancel_and_persist_without_changing_jobs() {
        let (_root, mut app, ctx) = fixture();
        app.view = View::Queues;
        app.open_proxy();
        click(&mut app, &ctx, "Custom proxy");
        click(&mut app, &ctx, "Save proxy settings");
        assert_eq!(app.dialog, Some(Dialog::Proxy));
        assert!(app.proxy.as_ref().unwrap().error.is_some());
        click(&mut app, &ctx, "×");
        assert_eq!(app.store.proxy_settings().unwrap(), Settings::default());
        app.open_proxy();
        click(&mut app, &ctx, "Direct connection");
        click(&mut app, &ctx, "Save proxy settings");
        assert!(app.dialog.is_none() && app.proxy.is_none());
        assert_eq!(app.store.proxy_settings().unwrap().mode, Mode::Direct);
        assert!(app.store.list().unwrap().is_empty());
        app.open_proxy();
        assert_eq!(app.proxy.as_ref().unwrap().draft.mode, Mode::Direct);
        click(&mut app, &ctx, "×");
    }
    #[test]
    fn proxy_login_is_masked_and_dialog_fits_both_themes_at_minimum_size() {
        for dark in [false, true] {
            for method in [
                service::Authentication::Basic,
                service::Authentication::Negotiate,
            ] {
                let (_root, mut app, ctx) = fixture();
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                app.open_proxy();
                let form = app.proxy.as_mut().unwrap();
                form.draft.mode = Mode::Custom;
                form.draft.authentication = method;
                form.draft.endpoint = "http://proxy.example.com:8080".into();
                form.login = LoginChoice::Replace;
                *form.username = "synthetic-user".into();
                *form.password = "never-visible-canary".into();
                for _ in 0..3 {
                    let output = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
                    assert!(output.shapes.iter().all(|shape| !matches!(&shape.shape,egui::epaint::Shape::Text(t) if t.galley.job.text.contains("never-visible-canary"))));
                }
                let output = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
                assert!(text_rect(&output, "Save proxy settings").bottom() <= 600.0);
                assert!(
                    text_rect(&output, "Allow unencrypted login to this proxy").bottom()
                        < text_rect(&output, "Save proxy settings").top()
                );
                click(&mut app, &ctx, "No login");
                assert!(
                    app.proxy.as_ref().unwrap().username.is_empty()
                        && app.proxy.as_ref().unwrap().password.is_empty()
                );
                click(&mut app, &ctx, "×");
                assert!(app.proxy.is_none());
            }
        }
    }
}
