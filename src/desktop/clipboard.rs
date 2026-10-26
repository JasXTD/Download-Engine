//! Offer copied file links in Add download: a tray notification plus a card in
//! the main window. clipboard_links.rs decides what is read and what is offered.
use super::*;
use download_engine::notifications::LinkOffer;
use egui::{Align, Layout, Stroke, vec2};

pub(super) struct Offer {
    url: String,
    name: String,
    /// Host and protocol; the category label is translated when drawn.
    site: String,
    category: Category,
}
impl Offer {
    fn detail(&self) -> String {
        format!("{} · {}", tr(self.category.label()), self.site)
    }
}

#[derive(Default)]
pub(super) struct State {
    /// Set only for the real window. Tests and screenshots never listen.
    native: Option<egui::Context>,
    #[cfg(windows)]
    monitor: Option<crate::clipboard_links::Monitor>,
    failed: bool,
    pub(super) offer: Option<Offer>,
}

impl Desktop {
    pub fn attach_clipboard(&mut self, ctx: &egui::Context) {
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
            || std::env::var_os("EFRAME_SCREENSHOT_TO").is_some()
        {
            return;
        }
        self.clipboard.native = Some(ctx.clone());
        self.sync_clipboard_listener();
    }

    /// The listener exists only while the setting is on. Turning it off also
    /// drops a waiting link.
    fn sync_clipboard_listener(&mut self) {
        if !self.interface.clipboard_links {
            #[cfg(windows)]
            {
                self.clipboard.monitor = None;
            }
            self.clipboard.failed = false;
            self.clipboard.offer = None;
            return;
        }
        #[cfg(windows)]
        if self.clipboard.monitor.is_none()
            && !self.clipboard.failed
            && let Some(ctx) = &self.clipboard.native
        {
            match crate::clipboard_links::Monitor::new(ctx.clone()) {
                Ok(monitor) => self.clipboard.monitor = Some(monitor),
                Err(error) => {
                    // Retry only after the setting is switched off and on again.
                    self.clipboard.failed = true;
                    self.notice = Some(Notice::error(tf!(
                        "Clipboard links are unavailable: {error}",
                        error = format!("{error:#}")
                    )));
                }
            }
        }
    }

    pub(super) fn clipboard_background(&mut self) {
        self.sync_clipboard_listener();
        #[cfg(windows)]
        if let Some(url) = self
            .clipboard
            .monitor
            .as_ref()
            .and_then(|monitor| monitor.links.try_iter().last())
        {
            self.offer_clipboard_link(url);
        }
    }

    /// Keep only the newest link and announce it through the tray when available.
    pub(super) fn offer_clipboard_link(&mut self, url: String) {
        let Ok(job) = Job::new(
            &url,
            None,
            self.store.paths.downloads.clone(),
            "clipboard-offer".into(),
        ) else {
            return;
        };
        let parsed = reqwest::Url::parse(&url).ok();
        let host = parsed
            .as_ref()
            .and_then(|url| url.host_str())
            .unwrap_or_default()
            .to_owned();
        let protocol = parsed
            .as_ref()
            .map(|url| url.scheme().to_uppercase())
            .unwrap_or_default();
        if let Some(tray) = self.tray.as_ref().filter(|tray| tray.available())
            && let Err(error) = tray.offer_link(LinkOffer {
                name: job.name.clone(),
                host: host.clone(),
            })
        {
            self.notice = Some(Notice::error(error.to_string()));
        }
        self.clipboard.offer = Some(Offer {
            site: format!("{host} · {protocol}"),
            category: job.category(),
            name: job.name,
            url,
        });
    }

    /// Open Add download with the copied link. Nothing starts until the user
    /// chooses Start download or Add to queue there.
    pub(super) fn review_clipboard_link(&mut self) {
        if self.dialog.is_some() {
            // The card appears once the current dialog closes.
            if self.clipboard.offer.is_some() {
                self.notice = Some(Notice::info(
                    tr("Finish the current dialog to review the copied link.").into(),
                ));
            }
            return;
        }
        let Some(offer) = self.clipboard.offer.take() else {
            return;
        };
        self.url = offer.url;
        self.name.clear();
        self.expected_sha256.clear();
        self.add_ftp_security = None;
        self.add_authentication = false;
        self.add_login = add_login::Draft::default();
        self.open_add();
    }

    pub(super) fn clipboard_card(&mut self, ctx: &egui::Context, p: Palette) {
        if self.dialog.is_some() {
            return;
        }
        let Some(offer) = &self.clipboard.offer else {
            return;
        };
        let mut download = false;
        let mut ignore = false;
        // Sits above the 34 px status bar, inside the window's right edge.
        egui::Area::new(egui::Id::new("clipboard-offer"))
            .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-16.0, -46.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(p.background)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(340.0);
                        ui.spacing_mut().item_spacing.y = 8.0;
                        ui.allocate_ui_with_layout(
                            vec2(ui.available_width(), 24.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.label(
                                    RichText::new(tr("Copied link")).size(12.0).color(p.muted),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let close =
                                        view::close_button(ui, 24.0, tr("Ignore this link"));
                                    close.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            close.enabled(),
                                            tr("Ignore this link"),
                                        )
                                    });
                                    ignore = close.clicked();
                                });
                            },
                        );
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(34.0, 34.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 6, p.selected);
                            let glyph = shell::category_icon(offer.category);
                            view::icon(ui, rect.center(), glyph, 19.0, p.blue);
                            let detail = offer.detail();
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.add(
                                    egui::Label::new(RichText::new(&offer.name).strong())
                                        .truncate(),
                                )
                                .on_hover_text(&offer.name);
                                ui.add(
                                    egui::Label::new(RichText::new(&detail).small().color(p.muted))
                                        .truncate(),
                                )
                                .on_hover_text(&detail);
                            });
                        });
                        ui.allocate_ui_with_layout(
                            vec2(ui.available_width(), 32.0),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                download =
                                    form_dialog::primary_button(ui, tr("Download…")).clicked()
                            },
                        );
                    });
            });
        if ignore {
            self.clipboard.offer = None;
        } else if download {
            self.review_clipboard_link();
        }
    }

    #[cfg(feature = "screenshot")]
    pub(super) fn clipboard_sample(&mut self) {
        self.offer_clipboard_link(
            "https://downloads.example.com/releases/Studio-installer-2.4-x64-offline-setup.zip"
                .into(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::{click, fixture, frame, text_rect};
    use super::*;

    fn shows(app: &mut Desktop, ctx: &egui::Context, label: &str) -> bool {
        let size = egui::vec2(860.0, 600.0);
        frame(app, ctx, size, vec![]);
        frame(app, ctx, size, vec![]).shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.job.text == label)
        })
    }

    #[test]
    fn copied_link_card_opens_add_download_prefilled_without_starting_anything() {
        let (_temp, mut app, ctx) = fixture();
        let jobs = app.store.list().unwrap().len();
        app.name = "stale name.bin".into();
        app.expected_sha256 = "ab".repeat(32);
        app.add_authentication = true;
        app.offer_clipboard_link("https://downloads.example.com/archive-2.4.zip".into());
        let offer = app.clipboard.offer.as_ref().unwrap();
        assert_eq!(offer.name, "archive-2.4.zip");
        assert_eq!(offer.detail(), "Compressed · downloads.example.com · HTTPS");
        assert!(shows(&mut app, &ctx, "Copied link"));
        click(&mut app, &ctx, "Download…");
        assert_eq!(app.dialog, Some(Dialog::Add));
        assert_eq!(app.url, "https://downloads.example.com/archive-2.4.zip");
        assert!(app.name.is_empty() && app.expected_sha256.is_empty());
        assert!(!app.add_authentication);
        assert!(app.clipboard.offer.is_none());
        assert_eq!(app.store.list().unwrap().len(), jobs, "Nothing is queued");
    }

    #[test]
    fn settings_row_switches_clipboard_links_and_saves_the_choice() {
        let (_temp, mut app, ctx) = fixture();
        app.open_preferences_window();
        app.preferences_window.as_mut().unwrap().category = preferences_window::Category::Browser;
        app.offer_clipboard_link("https://example.com/archive.zip".into());
        // A tall window draws every Settings group without scrolling.
        let size = egui::vec2(1280.0, 2400.0);
        for expected in [false, true] {
            frame(&mut app, &ctx, size, vec![]);
            let pos = text_rect(&frame(&mut app, &ctx, size, vec![]), "Clipboard links").center();
            for pressed in [true, false] {
                frame(
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
            }
            assert_eq!(
                app.preferences_window
                    .as_ref()
                    .unwrap()
                    .interface
                    .clipboard_links,
                expected
            );
            app.save_preferences_window(&ctx);
            assert_eq!(app.interface.clipboard_links, expected);
            assert_eq!(
                app.store.interface_preferences().unwrap().clipboard_links,
                expected
            );
            app.clipboard_background();
            // Switching off drops the waiting link; switching on does not bring it back.
            assert!(app.clipboard.offer.is_none());
            app.open_preferences_window();
            app.preferences_window.as_mut().unwrap().category =
                preferences_window::Category::Browser;
        }
    }

    #[test]
    fn copied_link_waits_for_an_open_dialog_newest_wins_and_x_ignores_it() {
        let (_temp, mut app, ctx) = fixture();
        app.open_queue_dialog(Dialog::NewQueue);
        app.offer_clipboard_link("https://example.com/old.zip".into());
        app.offer_clipboard_link("https://example.com/new.msi".into());
        assert!(
            !shows(&mut app, &ctx, "Copied link"),
            "Never covers a dialog"
        );
        // A notification click while a dialog is open keeps both.
        app.review_clipboard_link();
        assert_eq!(app.dialog, Some(Dialog::NewQueue));
        assert_eq!(app.clipboard.offer.as_ref().unwrap().name, "new.msi");
        assert!(app.notice.is_some());
        app.dialog = None;
        assert!(shows(&mut app, &ctx, "new.msi"));
        click(&mut app, &ctx, "×");
        assert!(app.clipboard.offer.is_none());
        assert_eq!(app.dialog, None);
        assert!(!shows(&mut app, &ctx, "Copied link"));
    }

    #[test]
    fn copied_link_card_fits_the_minimum_window_in_long_languages() {
        use download_engine::i18n::Language;
        let (_temp, mut app, ctx) = fixture();
        app.offer_clipboard_link(
            "https://downloads.example.com/releases/Studio-installer-2.4-x64-offline-setup.zip"
                .into(),
        );
        let size = egui::vec2(860.0, 600.0);
        for language in [Language::English, Language::German, Language::Russian] {
            app.interface.language = language;
            let _scope = download_engine::i18n::scope(language);
            frame(&mut app, &ctx, size, vec![]);
            let out = frame(&mut app, &ctx, size, vec![]);
            let window = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            for label in [tr("Copied link"), tr("Download…")] {
                let rect = text_rect(&out, label);
                assert!(window.contains_rect(rect), "{language:?}: {label}");
                assert!(rect.bottom() < size.y - 34.0, "Above the status bar");
            }
            // A waiting card follows a language change, including its category label.
            let detail = app.clipboard.offer.as_ref().unwrap().detail();
            assert!(detail.starts_with(tr("Compressed")), "{detail}");
            assert!(window.contains_rect(text_rect(&out, &detail)));
        }
    }

    #[test]
    fn turning_the_setting_off_drops_a_waiting_link_and_tests_never_listen() {
        let (_temp, mut app, _ctx) = fixture();
        assert!(app.interface.clipboard_links);
        app.offer_clipboard_link("https://example.com/archive.zip".into());
        app.clipboard_background();
        // Without attach_clipboard no native listener is created.
        #[cfg(windows)]
        assert!(app.clipboard.monitor.is_none());
        assert!(app.clipboard.offer.is_some());
        app.interface.clipboard_links = false;
        app.clipboard_background();
        assert!(app.clipboard.offer.is_none());
    }
}
