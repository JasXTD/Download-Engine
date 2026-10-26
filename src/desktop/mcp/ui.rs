//! Presentation for the desktop-owned MCP server. Actions stay in the controller.
use super::*;
use egui::{Align, Layout, RichText, vec2};

const LABEL_WIDTH: f32 = 82.0;
const CONTROL_HEIGHT: f32 = view::CONTROL_HEIGHT;
const COPY: &str = "\u{e8c8}";

fn card(ui: &mut egui::Ui, p: Palette, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(p.background)
        .stroke((1.0, p.border))
        .corner_radius(8)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.spacing_mut().interact_size.y = CONTROL_HEIGHT;
            content(ui);
        });
}

/// `height` is [`view::PRIMARY_HEIGHT`] for the server's start/stop action, which
/// swap in one place, and [`CONTROL_HEIGHT`] for the others.
fn button(
    ui: &mut egui::Ui,
    label: &str,
    glyph: &str,
    (primary, height): (bool, f32),
    p: Palette,
) -> egui::Response {
    let color = if primary { p.accent_text } else { p.text };
    let mut button = egui::Button::new(RichText::new(format!("      {label}")).color(color))
        .min_size(vec2(0.0, height));
    if primary {
        button = button.fill(p.accent);
    }
    let response = ui.add(button);
    view::icon(
        ui,
        egui::pos2(response.rect.left() + 14.0, response.rect.center().y),
        glyph,
        16.0,
        if response.enabled() { color } else { p.muted },
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), label)
    });
    response
}

fn field_label(ui: &mut egui::Ui, label: &str, help: Option<&str>, p: Palette) {
    ui.allocate_ui_with_layout(
        vec2(LABEL_WIDTH, ui.spacing().interact_size.y),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_width(LABEL_WIDTH);
            ui.label(label);
            if let Some(help) = help {
                ui.label(RichText::new("ⓘ").color(p.muted))
                    .on_hover_text(help);
            }
        },
    );
}

fn access_option(ui: &mut egui::Ui, read_only: &mut bool, value: bool, p: Palette) {
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.scope(|ui| {
        if *read_only == value {
            let widgets = &mut ui.visuals_mut().widgets;
            for visuals in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
            ] {
                visuals.bg_stroke = egui::Stroke::new(2.5, p.accent);
                visuals.fg_stroke.color = p.accent;
            }
        }
        ui.radio_value(
            read_only,
            value,
            RichText::new(if value {
                tr("View only")
            } else {
                tr("Allow changes")
            })
            .color(p.text),
        );
    });
    ui.horizontal(|ui| {
        ui.add_space(ui.spacing().icon_width + ui.spacing().icon_spacing);
        ui.add(
            egui::Label::new(
                RichText::new(if value {
                    tr("Inspect downloads, queues, scripts and logs.")
                } else {
                    tr("View and modify downloads and settings.")
                })
                .small()
                .color(p.muted),
            )
            .wrap(),
        );
    });
}

impl State {
    fn settings_card(&mut self, ui: &mut egui::Ui, p: Palette) -> Option<Action> {
        let mut action = None;
        let busy = self.pending.is_some();
        let active = self.status.active();
        let running = self.status == ServerStatus::Running;
        card(ui, p, |ui| {
            ui.horizontal(|ui| {
                let color = if running {
                    p.success
                } else if matches!(self.status, ServerStatus::Failed(_)) {
                    p.warning
                } else {
                    p.muted
                };
                let (dot, _) =
                    ui.allocate_exact_size(vec2(16.0, CONTROL_HEIGHT), egui::Sense::hover());
                if busy || matches!(self.status, ServerStatus::Starting | ServerStatus::Stopping) {
                    egui::Spinner::new().size(14.0).paint_at(ui, dot);
                } else {
                    ui.painter().circle_filled(dot.center(), 5.0, color);
                }
                ui.label(RichText::new(self.label()).color(color));
                ui.label(
                    RichText::new(if running && !self.saved.read_only {
                        tr("·  Allow changes  ·  This computer only")
                    } else if running {
                        tr("·  View only  ·  This computer only")
                    } else {
                        tr("·  This computer only")
                    })
                    .small()
                    .color(p.muted),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if active {
                        if ui
                            .add_enabled_ui(self.status != ServerStatus::Stopping, |ui| {
                                button(
                                    ui,
                                    tr("Stop server"),
                                    "\u{e71a}",
                                    (false, view::PRIMARY_HEIGHT),
                                    p,
                                )
                            })
                            .inner
                            .clicked()
                        {
                            action = Some(Action::Stop);
                        }
                    } else if ui
                        .add_enabled_ui(!busy, |ui| {
                            button(
                                ui,
                                tr("Start server"),
                                "\u{e768}",
                                (true, view::PRIMARY_HEIGHT),
                                p,
                            )
                        })
                        .inner
                        .clicked()
                    {
                        action = Some(Action::Start);
                    }
                });
            });
            if let Some(error) = &self.error {
                ui.colored_label(p.warning, error);
            }
            if let Some(notice) = &self.notice {
                ui.colored_label(p.success, notice);
            }
            if let ServerStatus::Failed(error) = &self.status {
                ui.colored_label(p.warning, error);
            }
            ui.separator();
            ui.add_enabled_ui(!active && !busy, |ui| {
                ui.horizontal(|ui| {
                    field_label(ui, tr("Port"), None, p);
                    ui.add_sized(
                        [110.0, CONTROL_HEIGHT],
                        view::line_edit(&mut self.port)
                            .id_salt("mcp-port")
                            .margin(vec2(8.0, 4.0))
                            .vertical_align(Align::Center)
                            .char_limit(5),
                    );
                    ui.label(RichText::new(tr("Default: 17842")).small().color(p.muted));
                });
                ui.spacing_mut().interact_size.y = 22.0;
                ui.spacing_mut().icon_width = 18.0;
                ui.spacing_mut().icon_width_inner = 10.0;
                ui.spacing_mut().icon_spacing = 8.0;
                ui.horizontal_top(|ui| {
                    field_label(ui, tr("AI access"), Some(tr("View only permits inspection. Allow changes can manage downloads and queues, and run saved Lua or PowerShell scripts.")), p);
                    ui.allocate_ui(vec2(ui.available_width().min(720.0), 0.0), |ui| {
                        ui.columns(2, |columns| {
                            access_option(&mut columns[0], &mut self.read_only, true, p);
                            access_option(&mut columns[1], &mut self.read_only, false, p);
                        });
                    });
                });
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.square_checkbox(&mut self.start_with_app, tr("Start with Download Engine"));
                ui.horizontal(|ui| {
                    // Line the hint up with the checkbox label, not the larger radio buttons above.
                    ui.add_space(view::CHECKBOX_SIZE + ui.spacing().icon_spacing);
                    ui.label(RichText::new(tr("Automatically start the server when Download Engine launches.")).small().color(p.muted));
                });
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !active && !busy && self.draft().ok() != Some(self.saved),
                        egui::Button::new(tr("Save settings")).min_size(vec2(0.0, CONTROL_HEIGHT)),
                    )
                    .clicked()
                {
                    action = Some(Action::Save);
                }
                ui.label(
                    RichText::new(if active {
                        tr("Stop the server to change these settings.")
                    } else {
                        tr("Start server also saves your settings.")
                    })
                    .small()
                    .color(p.muted),
                );
            });
        });
        action
    }

    fn connection_card(&mut self, ui: &mut egui::Ui, p: Palette) -> Option<Action> {
        let mut action = None;
        let busy = self.pending.is_some();
        let running = self.status == ServerStatus::Running;
        card(ui, p, |ui| {
            ui.label(RichText::new(tr("Connect your AI")).size(16.0).strong());
            ui.label(
                RichText::new(tr("Use this server URL in your AI client's MCP settings."))
                    .color(p.muted),
            );
            ui.add_space(4.0);
            let url = self.url();
            ui.horizontal(|ui| {
                field_label(ui, tr("Server URL"), None, p);
                let width = (ui.available_width() - CONTROL_HEIGHT - ui.spacing().item_spacing.x)
                    .max(100.0);
                // An immutable buffer permits selecting/copying the URL, without editing it.
                let mut display = url.as_deref().unwrap_or(tr("Enter a valid port above"));
                ui.add_sized(
                    [width, CONTROL_HEIGHT],
                    view::line_edit(&mut display)
                        .id_salt("mcp-url")
                        .font(egui::FontId::monospace(13.0))
                        .margin(vec2(10.0, 4.0))
                        .vertical_align(Align::Center),
                );
                let response = ui
                    .add_enabled(
                        !busy && url.is_some(),
                        egui::Button::new(
                            RichText::new(if cfg!(windows) { COPY } else { "⧉" })
                                .family(if cfg!(windows) {
                                    egui::FontFamily::Name("icons".into())
                                } else {
                                    egui::FontFamily::Proportional
                                })
                                .size(16.0),
                        )
                        .min_size(vec2(CONTROL_HEIGHT, CONTROL_HEIGHT)),
                    )
                    .on_hover_text(tr("Copy URL"));
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        response.enabled(),
                        tr("Copy URL"),
                    )
                });
                if response.clicked() {
                    action = Some(Action::CopyUrl);
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(LABEL_WIDTH + ui.spacing().item_spacing.x);
                ui.label(
                    RichText::new(if running {
                        tr("Use Streamable HTTP and the Authorization header from your client config.")
                    } else {
                        tr("Start the server to copy its authenticated client configuration.")
                    })
                    .small()
                    .color(p.muted),
                );
            });
            if running {
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(tr("Copy client config")).clicked() { action = Some(Action::CopyConfig); }
                        if ui.button(tr("Copy access header")).on_hover_text(tr("Paste this private value into a header named Authorization in your AI client.")).clicked() { action = Some(Action::CopyHeader); }
                        if ui.button(tr("Check connection")).clicked() { action = Some(Action::Check); }
                    });
                });
            }
            if ui
                .add_enabled(!busy, egui::Link::new(FIRST_PROMPT))
                .on_hover_text(tr("Copy this first prompt for your connected AI client"))
                .clicked()
            {
                action = Some(Action::CopyPrompt);
            }
        });
        action
    }
}

impl Desktop {
    pub(in crate::desktop) fn open_mcp(&mut self) {
        if self.dialog.is_none() {
            self.dialog = Some(Dialog::Mcp);
        }
    }

    pub(in crate::desktop) fn mcp_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let mut close = false;
        let response = form_dialog::modal("mcp-control", p).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 80.0).clamp(400.0, 740.0));
            close = form_dialog::title(
                ui,
                tr("AI connection (MCP)"),
                form_dialog::Icon::Symbol("\u{e839}"),
                p,
                true,
            );
            ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
            self.mcp_controls(ui, p);
        });
        if close || response.should_close() {
            // The controller owns the listener, not its presentation. Closing
            // this popup keeps the running server and any unsaved editor draft.
            self.dialog = None;
        }
    }

    fn mcp_controls(&mut self, ui: &mut egui::Ui, p: Palette) {
        self.mcp.poll(&self.store, ui.ctx());
        let mut action = None;
        let nested = self.preferences_window.is_some();
        ui.label(
            RichText::new(tr(
                "Let an AI client work with Download Engine on this computer.",
            ))
            .color(p.muted),
        );
        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .id_salt("mcp-control-body")
            .auto_shrink([false, false])
            .max_height((ui.ctx().content_rect().height() - if nested { 210.0 } else { 250.0 }).clamp(200.0, 560.0))
            .show(ui, |ui| {
                action = self.mcp.settings_card(ui, p).or(action);
                ui.add_space(10.0);
                action = self.mcp.connection_card(ui, p).or(action);
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tr("Runs while the app is open, including in the tray.")).small().color(p.muted));
                });
                ui.label(RichText::new(tr("Stopping MCP disconnects AI clients. Downloads already started keep running.")).small().color(p.muted));
                #[cfg(feature = "screenshot")]
                if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
                    && std::env::var_os("IDM_EX_TEST_MCP_BOTTOM").is_some()
                {
                    ui.scroll_to_cursor(Some(Align::BOTTOM));
                }
            });
        if !nested {
            ui.add_space(6.0);
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if button(
                    ui,
                    tr("Setup guide"),
                    "\u{e82d}",
                    (false, CONTROL_HEIGHT),
                    p,
                )
                .clicked()
                {
                    action = Some(Action::Guide);
                }
                if button(ui, tr("Open logs"), "\u{e9d9}", (false, CONTROL_HEIGHT), p).clicked() {
                    action = Some(Action::Logs);
                }
            });
        }
        match action {
            Some(Action::Guide) => {
                self.dialog = None;
                self.open_guide("MCP integration");
            }
            Some(Action::Logs) => {
                self.dialog = None;
                self.view = View::Logs;
                self.logs.request_refresh();
            }
            Some(action) => self.mcp.act(action, &self.store, ui.ctx()),
            None => {}
        }
    }
}
