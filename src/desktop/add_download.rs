//! Validate and review a new direct file download before queueing it.
use super::*;

impl Desktop {
    pub(super) fn open_add(&mut self) {
        self.dialog = Some(Dialog::Add);
        self.focus_url = true;
        self.add_queue = Some(self.queue_id.clone());
        self.add_login.reveal = self.add_authentication;
        self.form_error = None;
        self.load_download_folders();
    }

    pub(super) fn add_queue_id(&self) -> &str {
        self.add_queue.as_deref().unwrap_or(&self.queue_id)
    }

    pub(super) fn add(&mut self) {
        if self.add_login.request.is_some() {
            return;
        }
        if self.add_authentication {
            self.form_error = self
                .submit_add_login()
                .err()
                .map(|error| format!("{error:#}"));
            self.add_login.reveal |= self.form_error.is_some();
            return;
        }
        let page = self.url.trim().to_owned();
        if download_engine::hls::is_playlist_url(&self.url)
            || download_engine::hls::is_site_video(&self.url)
            || self.has_site_reader(&page)
        {
            let url = self.url.clone();
            let folder = self
                .folder_override()
                .map(|p| p.to_string_lossy().into_owned());
            let queue = self.add_queue_id().to_owned();
            self.add_queue = None;
            self.dialog = None;
            self.open_browser_stream(&url, &queue, folder.as_deref());
            return;
        }
        let result = self.prepare_add_download().and_then(|job| {
            self.store
                .enqueue_batch(&[job], self.add_queue_id())
                .map(|mut jobs| jobs.remove(0))
        });
        match result {
            Ok(job) => {
                self.finish_add_download(job);
                self.refresh();
            }
            Err(error) => self.form_error = Some(error.to_string()),
        }
    }

    pub(super) fn prepare_add_download(&self) -> anyhow::Result<Job> {
        let mut job = self.store.new_download(
            &self.url,
            Some(&self.name),
            self.folder_override().as_deref(),
            Uuid::new_v4().to_string(),
        )?;
        if let Some(ftp) = &mut job.ftp {
            if ftp.security != download_engine::ftp::Security::Plain
                && let Some(security) = self.add_ftp_security
            {
                ftp.security = security;
                let mut url = reqwest::Url::parse(&job.url)?;
                if security == download_engine::ftp::Security::Explicit && url.port().is_none() {
                    url.set_port(Some(21))
                        .map_err(|_| anyhow::anyhow!(tr("Invalid FTPS port")))?;
                    job.url = url.to_string();
                }
            }
            anyhow::ensure!(
                !self.add_authentication || ftp.security != download_engine::ftp::Security::Plain,
                tr("Saved logins require FTPS. Plain FTP supports anonymous downloads only.")
            );
        }
        job.integrity.expected = download_engine::integrity::parse_expected(&self.expected_sha256)?;
        Ok(job)
    }

    pub(super) fn finish_add_download(&mut self, job: Job) {
        self.expect_added_status(&job.id);
        if let Some(queue) = self.add_queue.take() {
            self.queue_id = queue;
        }
        self.set_selection(Some(job.id.clone()));
        self.filter = Filter::All;
        self.search.clear();
        self.view = View::Downloads;
        self.category = None;
        self.url.clear();
        self.add_ftp_security = None;
        self.name.clear();
        self.expected_sha256.clear();
        self.dialog = None;
        self.form_error = None;
        self.notice = None;
        self.last_refresh = Instant::now() - Duration::from_secs(1);
        self.add_authentication = false;
        self.add_login = add_login::Draft::default();
    }

    pub(super) fn add_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let focused = ctx.memory(|memory| memory.focused());
        let popup_was_open = egui::Popup::is_any_open(ctx);
        let saving = self.add_login.request.is_some();
        let locked = self.add_login.job.is_some();
        let mut editing_text = false;
        let mut submit = false;
        let mut cancel = false;
        let mut preview_zip = false;
        let response = form_dialog::modal("add-download", p).show(ctx, |ui| {
                cancel = form_dialog::header(ui, tr("Add download"),
                    tr("Paste a direct file link: HTTP, HTTPS, FTP or FTPS."),
                    form_dialog::Icon::Download, p, !saving);
                ui.add_enabled_ui(!saving, |ui| {
                    egui::ScrollArea::vertical().id_salt("add-form")
                        // A centered modal's previous size must not shrink the form's viewport.
                        .min_scrolled_height((ctx.content_rect().height() - 220.0).max(150.0))
                        .max_height((ctx.content_rect().height() - 220.0).max(150.0))
                        .show(ui, |ui| {
                            let reveal_login = self.add_login.reveal;
                            ui.spacing_mut().interact_size.y = view::CONTROL_HEIGHT;
                            let preview = Job::new(&self.url, Some(&self.name), self.store.paths.downloads.clone(), "preview".into()).ok();
                            let parsed = reqwest::Url::parse(self.url.trim()).ok();
                            let ftp = parsed.as_ref().and_then(download_engine::ftp::State::new);
                            let plain_ftp = ftp.as_ref().is_some_and(|value| value.security == download_engine::ftp::Security::Plain);
                            let stream = download_engine::hls::is_playlist_url(&self.url) || download_engine::hls::is_site_video(&self.url);
                            if (plain_ftp || stream) && !locked {
                                self.add_authentication = false;
                                self.add_login = add_login::Draft::default();
                            }
                            ui.add_enabled_ui(!locked, |ui| {
                                ui.label(RichText::new(tr("File URL")).small().color(p.muted));
                                ui.horizontal(|ui| {
                                    // A translated Paste may be wider; the field gives way so the row still ends at the edge.
                                    let paste_width = view::button_width(ui, tr("Paste")).max(58.0);
                                    let input = ui.add(
                                        view::line_edit(&mut self.url)
                                            .id(egui::Id::new("add-file-url"))
                                            .hint_text("https://example.com/file.zip")
                                            .desired_width((ui.available_width() - ui.spacing().item_spacing.x - paste_width).max(100.0))
                                            .vertical_align(egui::Align::Center),
                                    );
                                    editing_text |= focused == Some(input.id);
                                    if self.focus_url {
                                        input.request_focus();
                                        self.focus_url = false;
                                    }
                                    if input.changed() {
                                        self.add_ftp_security = None;
                                        // Never carry a password or token over to an edited destination.
                                        self.add_login = add_login::Draft::default();
                                    }
                                    if ui.add_sized([paste_width, view::CONTROL_HEIGHT], egui::Button::new(tr("Paste"))).clicked() {
                                        input.request_focus();
                                        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                                    }
                                });
                                if let Some(job) = &preview {
                                    ui.add_space(8.0);
                                    egui::Frame::new().fill(p.surface).corner_radius(6).inner_margin(10).show(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            let (rect, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                                            ui.painter().rect_filled(rect, 6, p.selected);
                                            let glyph = shell::category_icon(job.category());
                                            view::icon(ui, rect.center(), glyph, 19.0, p.blue);
                                            ui.vertical(|ui| {
                                                ui.add(egui::Label::new(RichText::new(&job.name).strong()).truncate()).on_hover_text(&job.name);
                                                let server = parsed.as_ref().and_then(|url| url.host_str()).unwrap_or("");
                                                let protocol = parsed.as_ref().map(|url| url.scheme().to_uppercase()).unwrap_or_default();
                                                ui.label(RichText::new(format!("{} · {server} · {protocol}", tr(job.category().label()))).small().color(p.muted));
                                            });
                                        });
                                    });
                                }
                                if let Some(ftp) = ftp.as_ref() {
                                    if plain_ftp {
                                        ui.label(RichText::new(tr("Anonymous FTP · unencrypted. Use FTPS for a saved login.")).small().color(p.muted));
                                    } else {
                                        let security = self.add_ftp_security.get_or_insert(ftp.security);
                                        egui::ComboBox::from_id_salt("ftp-security").selected_text(tr(security.label())).show_ui(ui, |ui| {
                                            for mode in [download_engine::ftp::Security::Explicit, download_engine::ftp::Security::Implicit] {
                                                ui.selectable_value(security, mode, tr(mode.label()));
                                            }
                                        });
                                    }
                                }
                                ui.add_space(8.0);
                                ui.columns(2, |columns| {
                                    columns[0].label(RichText::new(tr("File name · optional")).small().color(p.muted));
                                    let name = columns[0].add(
                                        view::line_edit(&mut self.name)
                                            .hint_text(preview.as_ref().map(|job| job.name.as_str()).unwrap_or(tr("Use the server's file name")))
                                            .desired_width(f32::INFINITY)
                                            .vertical_align(egui::Align::Center),
                                    );
                                    editing_text |= focused == Some(name.id);
                                    columns[1].label(RichText::new(tr("Queue")).small().color(p.muted));
                                    let selected = self.queues.iter().find(|queue| queue.id == self.add_queue_id())
                                        .map(|queue| display_queue_name(&queue.id, &queue.name)).unwrap_or(tr("Main queue")).to_owned();
                                    egui::ComboBox::from_id_salt("add-queue").width(columns[1].available_width())
                                        .selected_text(short_queue_name(&selected))
                                        .show_ui(&mut columns[1], |ui| {
                                            for queue in &self.queues {
                                                ui.selectable_value(&mut self.add_queue, Some(queue.id.clone()), display_queue_name(&queue.id, &queue.name));
                                            }
                                        }).response.on_hover_text(selected);
                                });
                                ui.add_space(6.0);
                                ui.label(RichText::new(tr("Save to")).small().color(p.muted));
                                let folder = self.folder_picker.field(ui, &mut self.folder, &self.store.paths.downloads, &mut self.form_error);
                                editing_text |= focused == Some(folder.id);
                                if let Some(job) = &preview {
                                    if let Some(rules) = &self.download_folders {
                                        let destination = self.folder_override().unwrap_or_else(|| rules.destination(&self.store.paths.downloads, &job.name));
                                        let prefix = if self.folder.trim().is_empty() { tr("Automatic") } else { tr("Folder") };
                                        let path = destination.to_string_lossy();
                                        ui.add(egui::Label::new(RichText::new(format!("{prefix} → {path}")).small().color(p.muted)).truncate()).on_hover_text(path.as_ref());
                                    }
                                } else {
                                    ui.label(RichText::new(tr("Leave blank to use your download folder rules.")).small().color(p.muted));
                                }
                            });
                            ui.add_space(6.0);
                            egui::CollapsingHeader::new(tr("Advanced")).id_salt("add-advanced")
                                .default_open(self.add_authentication || !self.expected_sha256.is_empty())
                                .show(ui, |ui| {
                                    ui.add_enabled_ui(!locked, |ui| {
                                        ui.label(RichText::new(tr("Expected SHA-256 (optional)")).small().color(p.muted));
                                        let checksum = ui.add(view::line_edit(&mut self.expected_sha256)
                                            .hint_text("64 hexadecimal characters from the publisher").char_limit(128).desired_width(f32::INFINITY));
                                        editing_text |= focused == Some(checksum.id);
                                    });
                                    let login = ui.add_enabled(!locked && !plain_ftp && !stream, view::Checkbox::new(&mut self.add_authentication, tr("Use login")));
                                    if login.changed() {
                                        self.add_login = add_login::Draft::default();
                                        self.add_login.reveal = self.add_authentication;
                                    }
                                    if self.add_authentication {
                                        editing_text |= self.add_login_fields(ui, p);
                                    }
                                    if locked {
                                        ui.label(RichText::new(tr("This file stays paused until the login is saved. Closing keeps it paused.")).small().color(p.muted));
                                    }
                                    preview_zip = ui.add_enabled(
                                        !locked && parsed.as_ref().is_some_and(|url| matches!(url.scheme(), "http" | "https")) && !self.add_authentication,
                                        egui::Button::new(tr("Preview ZIP contents")),
                                    ).on_hover_text(tr("Read the ZIP file list before downloading. For protected files, use the download's Authentication and ZIP preview actions.")).clicked();
                                });
                            if let Some(error) = &self.form_error {
                                let response = ui.label(RichText::new(error).small().color(p.warning));
                                if reveal_login {
                                    response.scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
                                }
                            }
                        });
                    let action = if saving { tr("Saving login…") }
                        else if locked { tr("Retry login") }
                        else if self.store.new_download_starts(self.add_queue_id()).unwrap_or(false) { tr("Start download") }
                        else { tr("Add to queue") };
                    submit = form_dialog::footer(ui, action, Some(tr("Enter to add · Esc to cancel")), p);
                    submit |= editing_text && !popup_was_open && !egui::Popup::is_any_open(ctx)
                        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                });
            });
        if !saving && (cancel || response.should_close()) {
            self.add_login = add_login::Draft::default();
            self.add_queue = None;
            self.dialog = None;
            self.form_error = None;
        } else if !saving && submit {
            self.add();
        } else if preview_zip {
            self.open_zip_draft();
        }
    }
}
