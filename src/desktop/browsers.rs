//! Browser registration and connection instructions.
use super::*;

impl Desktop {
    pub fn open_browser_setup(&mut self) {
        self.dialog = Some(Dialog::Browsers);
        self.refresh_browser_registration();
    }

    pub(super) fn refresh_browser_registration(&mut self) {
        self.browser_notice = None;
        let setup = BrowserSetup::discover();
        self.browser_expected = setup
            .as_ref()
            .ok()
            .and_then(|setup| setup.expected_extension_id(self.browser));
        match setup.and_then(|setup| setup.status(self.browser)) {
            Ok(status) => {
                if self.browser != Browser::Firefox {
                    // Prefer this copy's folder ID: it is what the browser shows after
                    // loading the folder that Open extension folder opens.
                    let registered = match &status {
                        RegistrationStatus::Registered { extension_id } => Some(extension_id),
                        _ => None,
                    };
                    let index = id_slot(self.browser);
                    if self.browser_ids[index].is_empty()
                        && let Some(id) = self.browser_expected.as_ref().or(registered)
                    {
                        self.browser_ids[index] = id.clone();
                    }
                }
                self.browser_status = Some(status);
            }
            Err(error) => {
                download_engine::diagnostics::error(
                    download_engine::diagnostics::Event::BrowserSetupFailed,
                    &error,
                );
                self.browser_status = None;
                self.browser_notice = Some((
                    true,
                    tf!("Cannot read registration: {error:#}", error = error),
                ));
            }
        }
    }

    pub(super) fn browser_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let mut close = false;
        let response = form_dialog::modal("browser-setup", p).show(ctx, |ui| {
            ui.set_width(540.0);
            close = form_dialog::title(
                ui,
                tr("Browser integration"),
                form_dialog::Icon::Symbol("\u{e774}"),
                p,
                true,
            );
            let previous = self.browser;
            ui.horizontal(|ui| {
                // The selector comes first so the description wraps in the space left of
                // it instead of running underneath it in longer languages.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::Frame::new()
                        .fill(p.background)
                        .stroke(egui::Stroke::new(1.0, p.border))
                        .corner_radius(6)
                        .inner_margin(egui::Margin::symmetric(4, 3))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for browser in Browser::ALL.into_iter().rev() {
                                ui.selectable_value(&mut self.browser, browser, browser.label());
                            }
                        });
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(tr(
                                    "Connect Chrome, Edge, Brave or Firefox in three quick steps.",
                                ))
                                .small()
                                .color(p.muted),
                            )
                            .wrap(),
                        );
                    });
                });
            });
            if previous != self.browser {
                self.refresh_browser_registration();
            }
            ui.add_space(6.0);
            let step_card = |ui: &mut egui::Ui, add_contents: &mut dyn FnMut(&mut egui::Ui)| {
                egui::Frame::new()
                    .fill(p.background)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 6.0;
                        add_contents(ui);
                    });
            };
            // Keep the scroll area sized to the viewport, not the previous modal height.
            let body_height = (ctx.content_rect().height() - 180.0).max(280.0);
            egui::ScrollArea::vertical()
                .min_scrolled_height(body_height)
                .max_height(body_height)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    step_card(ui, &mut |ui| {
                        let mut page = match self.browser {
                            Browser::Chrome => "chrome://extensions",
                            Browser::Edge => "edge://extensions",
                            Browser::Brave => "brave://extensions",
                            Browser::Firefox => "about:debugging#/runtime/this-firefox",
                        };
                        ui.strong("1. Load the extension");
                        // An immutable text buffer keeps the address selectable and copyable.
                        ui.add(
                            view::line_edit(&mut page)
                                .id_salt("browser-extension-page")
                                .font(egui::TextStyle::Monospace)
                                .text_color(p.text)
                                .desired_width(f32::INFINITY)
                                .margin(egui::vec2(8.0, 4.0)),
                        );
                        ui.label(
                            RichText::new(if self.browser == Browser::Firefox {
                                tr("Open the page above, click Load Temporary Add-on, and select manifest.json.")
                            } else {
                                tr("Open the page above, enable Developer mode, and click Load unpacked.")
                            })
                            .small()
                            .color(p.muted),
                        );
                        if self.browser != Browser::Firefox {
                            ui.label(
                                RichText::new(tr("This extension can only be used in Developer mode."))
                                    .small()
                                    .color(p.warning),
                            );
                        }
                        ui.horizontal(|ui| {
                            if ui.button(tr("Copy extensions page")).clicked() {
                                ctx.copy_text(page.into());
                            }
                            if ui.button(tr("Open extension folder")).clicked() {
                                let result = BrowserSetup::discover().and_then(|setup| {
                                    let folder = setup.extension_folder(self.browser);
                                    anyhow::ensure!(
                                        folder.is_dir(),
                                        tr("Extension folder is missing. Use the packaged application")
                                    );
                                    std::process::Command::new(download_engine::paths::file_browser()?)
                                        .arg(folder)
                                        .spawn()?;
                                    Ok(())
                                });
                                if let Err(error) = result {
                                    self.browser_notice = Some((true, format!("{error:#}")));
                                }
                            }
                        });
                    });
                    step_card(ui, &mut |ui| {
                        let (badge, detail, tone) = match &self.browser_status {
                            Some(RegistrationStatus::Missing) => {
                                (tr("Not registered"), None, p.muted)
                            }
                            Some(RegistrationStatus::Registered { .. }) => {
                                (tr("Registered"), None, p.success)
                            }
                            Some(RegistrationStatus::OtherCopy { manifest }) => (
                                tr("Registered to another copy"),
                                Some(tf!(
                                    "Disconnect it first: {}",
                                    manifest.display()
                                )),
                                p.warning,
                            ),
                            Some(RegistrationStatus::NeedsRepair { message }) => (
                                tr("Needs repair"),
                                Some(message.clone()),
                                p.warning,
                            ),
                            None => (tr("Unavailable"), None, p.warning),
                        };
                        ui.horizontal(|ui| {
                            ui.strong("2. Connect to Download Engine");
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                egui::Frame::new()
                                    .fill(tone.gamma_multiply(0.14))
                                    .stroke(egui::Stroke::new(1.0, tone.gamma_multiply(0.35)))
                                    .corner_radius(255)
                                    .inner_margin(egui::Margin::symmetric(8, 2))
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(badge).size(11.0).strong().color(tone));
                                    });
                            });
                        });
                        if self.browser != Browser::Firefox {
                            let index = id_slot(self.browser);
                            ui.add(
                                view::line_edit(&mut self.browser_ids[index])
                                    .hint_text(tr("Paste 32-letter extension ID from the extensions page"))
                                    .desired_width(f32::INFINITY)
                                    .char_limit(64),
                            );
                            if self.browser_expected.is_some() {
                                ui.label(
                                    RichText::new(tr("Filled in from this copy's extension folder. If you loaded the extension from another folder, paste the ID your browser shows."))
                                        .small()
                                        .color(p.muted),
                                );
                            }
                            if let (
                                Some(RegistrationStatus::Registered { extension_id }),
                                Some(expected),
                            ) = (&self.browser_status, &self.browser_expected)
                                && extension_id != expected
                            {
                                ui.label(
                                    RichText::new(tf!(
                                        "Connected to a different extension ID ({id}). If your browser shows the ID above, click Connect browser to update it.",
                                        id = extension_id
                                    ))
                                    .small()
                                    .color(p.warning),
                                );
                            }
                        } else {
                            ui.label(
                                RichText::new(tr("Firefox uses a built-in extension ID."))
                                    .small()
                                    .color(p.muted),
                            );
                        }
                        if let Some(detail) = detail {
                            ui.label(RichText::new(detail).small().color(tone));
                        }
                        let other_copy = matches!(
                            self.browser_status,
                            Some(RegistrationStatus::OtherCopy { .. })
                        );
                        let own_copy = matches!(
                            self.browser_status,
                            Some(
                                RegistrationStatus::Registered { .. }
                                    | RegistrationStatus::NeedsRepair { .. }
                            )
                        );
                        ui.horizontal(|ui| {
                            if form_dialog::primary_enabled(ui, !other_copy, tr("Connect browser"))
                                .clicked()
                            {
                                let id = if self.browser == Browser::Firefox {
                                    ""
                                } else {
                                    &self.browser_ids[id_slot(self.browser)]
                                };
                                let result = BrowserSetup::discover()
                                    .and_then(|setup| setup.register(self.browser, id));
                                log_registration(
                                    &result,
                                    download_engine::diagnostics::Event::BrowserRegistered,
                                );
                                self.refresh_browser_registration();
                                self.browser_notice = Some(match result {
                                    Ok(()) => (
                                        false,
                                        tr("Connected. Click Check connection in the extension popup.")
                                            .into(),
                                    ),
                                    Err(error) => (true, format!("{error:#}")),
                                });
                            }
                            if ui
                                .add_enabled(own_copy, egui::Button::new(tr("Disconnect")))
                                .clicked()
                            {
                                let result = BrowserSetup::discover()
                                    .and_then(|setup| setup.unregister(self.browser));
                                log_registration(
                                    &result,
                                    download_engine::diagnostics::Event::BrowserUnregistered,
                                );
                                self.refresh_browser_registration();
                                self.browser_notice = Some(match result {
                                    Ok(_) => (
                                        false,
                                        tr("Browser registration removed.").into(),
                                    ),
                                    Err(error) => (true, format!("{error:#}")),
                                });
                            }
                            if ui.button(tr("Refresh status")).clicked() {
                                self.refresh_browser_registration();
                            }
                        });
                        if let Some((error, message)) = &self.browser_notice {
                            ui.label(
                                RichText::new(message)
                                    .small()
                                    .color(if *error { p.warning } else { p.success }),
                            );
                        }
                    });
                    step_card(ui, &mut |ui| {
                        ui.strong("3. Check the connection");
                        ui.label(
                            RichText::new(
                                tr("Open the extension popup and click Check connection to enable automatic capture."),
                            )
                            .small()
                            .color(p.muted),
                        );
                    });
                });
        });
        if close || response.should_close() {
            self.dialog = None;
        }
    }
}

/// Each Chromium browser keeps its own extension ID draft; Firefox has a built-in ID.
fn id_slot(browser: Browser) -> usize {
    match browser {
        Browser::Edge => 1,
        Browser::Brave => 2,
        Browser::Chrome | Browser::Firefox => 0,
    }
}

fn log_registration<T>(result: &anyhow::Result<T>, event: download_engine::diagnostics::Event) {
    use download_engine::diagnostics::{self, Details, Event, Level};
    match result {
        Ok(_) => diagnostics::record(Level::Info, event, Details::default()),
        Err(error) => diagnostics::error(Event::BrowserSetupFailed, error),
    }
}
