//! Category contents for the staged Preferences popup.
use super::*;
use egui::{Align, Align2, FontId, Layout, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

/// Every row's trailing control (switch, arrow, dropdown or choice) ends this far
/// inside the row, so their right edges line up in any language.
const TRAILING: f32 = 16.0;

/// A control-height slot of `width` that ends at the row's trailing edge.
fn trailing_slot(row: Rect, width: f32) -> Rect {
    Rect::from_min_max(
        pos2(row.right() - TRAILING - width, row.center().y - 15.0),
        pos2(row.right() - TRAILING, row.center().y + 15.0),
    )
}

pub(super) struct Summary {
    loaded: bool,
    proxy: Result<String, String>,
    antivirus: Result<String, String>,
}
impl Default for Summary {
    fn default() -> Self {
        Self {
            loaded: false,
            proxy: Ok(tr("Loading…").into()),
            antivirus: Ok(tr("Loading…").into()),
        }
    }
}

fn section(ui: &mut egui::Ui, p: Palette, label: &str, contents: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(label).size(11.0).color(p.muted));
    ui.add_space(5.0);
    egui::Frame::new()
        .fill(p.background)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            contents(ui);
        });
    ui.add_space(16.0);
}

fn row(
    ui: &mut egui::Ui,
    p: Palette,
    title: &str,
    detail: &str,
    glyph: &str,
    summary: &str,
) -> egui::Response {
    row_with(ui, p, title, (detail, p.muted), glyph, summary, 190.0)
}

fn row_with(
    ui: &mut egui::Ui,
    p: Palette,
    title: &str,
    (detail, detail_color): (&str, egui::Color32),
    glyph: &str,
    summary: &str,
    right_width: f32,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(
        vec2(
            ui.available_width(),
            if detail.is_empty() { 36.0 } else { 54.0 },
        ),
        Sense::hover(),
    );
    let response = ui.interact(
        rect,
        ui.make_persistent_id(("settings-row", title)),
        Sense::click(),
    );
    if response.clicked() {
        response.request_focus();
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title)
    });
    let painter = ui.painter();
    if response.hovered() || response.has_focus() {
        painter.rect_filled(rect.shrink(1.0), 6, p.selected);
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect.shrink(1.0),
            6,
            Stroke::new(1.0, p.blue),
            StrokeKind::Inside,
        );
    }
    painter.line_segment(
        [
            rect.left_bottom() + vec2(12.0, 0.0),
            rect.right_bottom() - vec2(12.0, 0.0),
        ],
        Stroke::new(0.5, p.border),
    );
    let icon = Rect::from_center_size(pos2(rect.left() + 28.0, rect.center().y), vec2(30.0, 30.0));
    painter.rect_filled(icon, 6, p.track);
    view::icon(ui, icon.center(), glyph, 15.0, p.muted);
    {
        let mut labels = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    rect.min + vec2(54.0, 10.0),
                    vec2((rect.width() - right_width - 62.0).max(120.0), 42.0),
                ))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let ui = &mut labels;
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.add(
            egui::Label::new(RichText::new(title).size(13.0).strong())
                .truncate()
                .selectable(false)
                .sense(Sense::hover()),
        )
        .on_hover_text(title);
        if !detail.is_empty() {
            ui.add(
                egui::Label::new(RichText::new(detail).size(11.0).color(detail_color))
                    .truncate()
                    .selectable(false)
                    .sense(Sense::hover()),
            )
            .on_hover_text(detail);
        }
    }
    let summary_rect = Rect::from_min_max(
        pos2(rect.right() - right_width, rect.top() + 14.0),
        pos2(rect.right() - 28.0, rect.bottom() - 14.0),
    );
    ui.new_child(
        egui::UiBuilder::new()
            .max_rect(summary_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    )
    .add(
        egui::Label::new(RichText::new(summary).size(12.0).color(p.muted))
            .truncate()
            .selectable(false)
            .sense(Sense::hover())
            .halign(egui::Align::Max),
    )
    .on_hover_text(summary);
    response
}

fn navigation(
    ui: &mut egui::Ui,
    p: Palette,
    title: &str,
    detail: &str,
    glyph: &str,
    summary: &str,
) -> egui::Response {
    let response = row(ui, p, title, detail, glyph, summary);
    ui.painter().text(
        pos2(response.rect.right() - TRAILING, response.rect.center().y),
        Align2::RIGHT_CENTER,
        "›",
        FontId::proportional(19.0),
        p.muted,
    );
    response
}

fn toggle(ui: &mut egui::Ui, p: Palette, title: &str, detail: &str, glyph: &str, value: &mut bool) {
    toggle_with(ui, p, title, (detail, p.muted), glyph, value);
}

fn toggle_with(
    ui: &mut egui::Ui,
    p: Palette,
    title: &str,
    detail: (&str, egui::Color32),
    glyph: &str,
    value: &mut bool,
) -> egui::Response {
    let response = row_with(ui, p, title, detail, glyph, "", 60.0);
    if response.clicked() {
        *value = !*value;
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, title)
    });
    let track = Rect::from_min_size(
        pos2(
            response.rect.right() - TRAILING - 34.0,
            response.rect.center().y - 9.0,
        ),
        vec2(34.0, 18.0),
    );
    ui.painter()
        .rect_filled(track, 9, if *value { p.accent } else { p.track });
    let x = if *value {
        track.right() - 9.0
    } else {
        track.left() + 9.0
    };
    ui.painter().circle_filled(
        pos2(x, track.center().y),
        6.5,
        if *value { p.accent_text } else { p.muted },
    );
    response
}

impl Summary {
    pub(super) fn load(store: &Store) -> Self {
        let proxy = store
            .proxy_settings()
            .map(|settings| tr(settings.mode.label()).to_owned())
            .map_err(|error| error.to_string());
        let antivirus = store
            .antivirus_settings()
            .map(|settings| {
                if settings.enabled {
                    tr(settings.provider.label()).to_owned()
                } else {
                    tr("Off").into()
                }
            })
            .map_err(|error| error.to_string());
        Self {
            loaded: true,
            proxy,
            antivirus,
        }
    }
}

impl Desktop {
    pub(super) fn refresh_settings_summary(&mut self) {
        self.background_io.need_settings = true;
        self.last_refresh = Instant::now() - Duration::from_secs(1);
        self.request_snapshot(false);
    }

    pub(super) fn preferences_category(
        &mut self,
        ui: &mut egui::Ui,
        p: Palette,
        state: &mut preferences_window::State,
    ) -> Option<preferences_window::Editor> {
        use preferences_window::{Category, Editor};
        if !self.settings_summary.loaded {
            self.refresh_settings_summary();
        }
        let mut editor = None;
        let theme_session = &mut state.session;
        ui.label(RichText::new(state.category.label()).size(17.0).strong());
        ui.add_space(8.0);
        let interface = &mut state.interface;
        match state.category {
            Category::Appearance => {
                section(
                    ui,
                    p,
                    tr("Personalize Download Engine and choose where files are saved."),
                    |ui| {
                        let choices = [
                            (Appearance::System, tr("System")),
                            (Appearance::Light, tr("Light")),
                            (Appearance::Dark, tr("Dark")),
                            (Appearance::Custom, tr("Custom")),
                        ];
                        let stroke = ui.visuals().widgets.inactive.bg_stroke.width;
                        let width = choices
                            .iter()
                            .map(|(_, label)| view::button_width(ui, label) - 2.0 * stroke)
                            .sum::<f32>()
                            + 2.0 * stroke
                            + ui.spacing().item_spacing.x * (choices.len() - 1) as f32;
                        let theme = row_with(
                            ui,
                            p,
                            tr("Theme"),
                            (tr("Follow Windows or pick a fixed theme."), p.muted),
                            "\u{e790}",
                            "",
                            width + TRAILING,
                        );
                        let mut themes = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(trailing_slot(theme.rect, width))
                                .layout(Layout::left_to_right(Align::Center)),
                        );
                        for (appearance, label) in choices {
                            themes.selectable_value(&mut interface.appearance, appearance, label);
                        }
                        if interface.appearance == Appearance::Custom {
                            egui::Frame::new().inner_margin(12).show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 6.0;
                                self.theme_assets.gallery(
                                    ui,
                                    &mut interface.custom_theme,
                                    theme_session,
                                    &self.store.paths.data,
                                    &mut self.folder_picker,
                                    &self.store.paths.downloads,
                                );
                            });
                        }
                        let language = row(
                            ui,
                            p,
                            tr("Language"),
                            tr("Choose the display language. Save to apply."),
                            "\u{e774}",
                            "",
                        );
                        ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(trailing_slot(language.rect, 180.0))
                                .layout(Layout::right_to_left(Align::Center)),
                        )
                        .horizontal(|ui| {
                            egui::ComboBox::from_id_salt("display-language")
                                .width(180.0)
                                .selected_text(interface.language.native_name())
                                .show_ui(ui, |ui| {
                                    for language in download_engine::i18n::Language::ALL {
                                        ui.selectable_value(
                                            &mut interface.language,
                                            language,
                                            language.native_name(),
                                        );
                                    }
                                });
                        });
                        toggle(
                            ui,
                            p,
                            tr("Compact rows"),
                            tr("Fit more downloads on screen."),
                            "\u{e8fd}",
                            &mut interface.compact_rows,
                        );
                        toggle(
                            ui,
                            p,
                            tr("Details panel"),
                            tr("Show the inspector when a download is selected."),
                            "\u{e8a0}",
                            &mut interface.show_details,
                        );
                        toggle(
                            ui,
                            p,
                            tr("Sidebar labels"),
                            tr("Show names next to sidebar icons."),
                            "app",
                            &mut interface.sidebar_labels,
                        );
                        let shown = 2 + interface.columns.iter().filter(|shown| **shown).count();
                        if navigation(
                            ui,
                            p,
                            tr("Table columns"),
                            tr("Choose, reorder and resize list columns."),
                            "\u{e8a0}",
                            &tf!("{shown} of 9 shown", shown = shown),
                        )
                        .clicked()
                        {
                            editor = Some(Editor::Columns);
                        }
                    },
                );
            }
            Category::Downloads => {
                section(ui, p, tr("DOWNLOADS"), |ui| {
                    toggle(
                        ui,
                        p,
                        tr("Download status window"),
                        tr("Show progress when you start a single download."),
                        "\u{e9d9}",
                        &mut interface.auto_download_status,
                    );
                    let path = self.store.paths.downloads.to_string_lossy().into_owned();
                    if navigation(ui, p, tr("Download folders"), tr("Sort new files into folders by type. Existing files stay where they are."), "folder", &path).clicked() { editor = Some(Editor::Folders); }
                    let speed = self
                        .limits
                        .global
                        .map(speed_limit_label)
                        .unwrap_or_else(|| tr("No limit").into());
                    if navigation(
                        ui,
                        p,
                        tr("Speed limits"),
                        tr("Cap the total speed or a single file."),
                        "\u{e9d9}",
                        &speed,
                    )
                    .clicked()
                    {
                        editor = Some(Editor::Speed);
                    }
                    let quota = if self.quota_status.blocked() {
                        tr("Limit reached")
                    } else if self.quota_status.limits.enabled() {
                        tr("On")
                    } else {
                        tr("Off")
                    };
                    if navigation(
                        ui,
                        p,
                        tr("Data quotas"),
                        tr("Hourly and daily data allowances."),
                        "\u{e9d9}",
                        quota,
                    )
                    .clicked()
                    {
                        editor = Some(Editor::Quotas);
                    }
                });
            }
            Category::Connection => {
                section(ui, p, tr("Connection"), |ui| {
                    let proxy = &self.settings_summary.proxy;
                    let response = navigation(
                        ui,
                        p,
                        tr("Proxy"),
                        tr("Proxy used for HTTP and HTTPS downloads."),
                        "\u{e71b}",
                        proxy.as_deref().unwrap_or(tr("Unavailable")),
                    );
                    if let Err(error) = proxy {
                        response.clone().on_hover_text(error);
                    }
                    if response.clicked() {
                        editor = Some(Editor::Proxy);
                    }
                    if navigation(
                        ui,
                        p,
                        tr("Download connections"),
                        tr("Set connections per file and website overrides."),
                        "\u{e968}",
                        tr("Configure"),
                    )
                    .clicked()
                    {
                        editor = Some(Editor::Connections);
                    }
                });
            }
            Category::Browser => {
                section(ui, p, tr("Browser & AI"), |ui| {
                    if navigation(
                        ui,
                        p,
                        tr("Browser integration"),
                        tr("Send links and downloads from Chrome, Edge, Brave and Firefox."),
                        "\u{e774}",
                        tr("Set up browsers"),
                    )
                    .clicked()
                    {
                        editor = Some(Editor::Browsers);
                    }
                    toggle(
                        ui,
                        p,
                        tr("Clipboard links"),
                        tr("Offer to download file links you copy in other apps."),
                        "\u{e77f}",
                        &mut interface.clipboard_links,
                    );
                    if navigation(
                        ui,
                        p,
                        tr("AI connection (MCP)"),
                        tr("Connect an AI client on this computer."),
                        "\u{e839}",
                        self.mcp.label(),
                    )
                    .clicked()
                    {
                        editor = Some(Editor::Mcp);
                    }
                });
            }
            Category::Antivirus => {
                section(ui, p, tr("Antivirus"), |ui| {
                    let antivirus = &self.settings_summary.antivirus;
                    let response = navigation(
                        ui,
                        p,
                        tr("Antivirus scan"),
                        tr("Scan finished files with your selected scanner."),
                        "\u{e83d}",
                        antivirus.as_deref().unwrap_or(tr("Unavailable")),
                    );
                    if let Err(error) = antivirus {
                        response.clone().on_hover_text(error);
                    }
                    if response.clicked() {
                        editor = Some(Editor::Antivirus);
                    }
                });
            }
            Category::Startup => self.preferences_startup(ui, p, state),
        }
        editor
    }

    fn preferences_startup(
        &mut self,
        ui: &mut egui::Ui,
        p: Palette,
        state: &mut preferences_window::State,
    ) {
        use crate::startup::State;
        self.startup.refresh_if_stale();
        let available = self.startup.state().is_ok();
        let detail = match self.startup.state() {
            Ok(State::OtherCopy(_)) => {
                tr("Another Download Engine copy starts with Windows. Turn on to use this copy.")
            }
            Ok(State::Missing(_)) => {
                tr("Download Engine is set to start a missing program. Turn on to use this copy.")
            }
            Ok(State::Blocked) => tr("Turned off in Task Manager. Turn on to allow it again."),
            Ok(State::On | State::Off) => tr("Open Download Engine when you sign in to Windows."),
            Err(error) => error,
        };
        section(ui, p, tr("STARTUP"), |ui| {
            ui.add_enabled_ui(available, |ui| {
                toggle(
                    ui,
                    p,
                    tr("Start with Windows"),
                    detail,
                    "\u{e7e8}",
                    &mut state.startup,
                );
            });
            ui.add_enabled_ui(available && state.startup, |ui| {
                toggle(
                    ui,
                    p,
                    tr("Start in the tray"),
                    tr("Stay hidden in the notification area until you open it."),
                    "\u{e921}",
                    &mut state.start_hidden,
                );
            });
        });
        section(ui, p, tr("Tray & notifications"), |ui| {
            ui.add_enabled_ui(
                self.tray_available() || state.behavior.close_to_tray,
                |ui| {
                    toggle(
                        ui,
                        p,
                        tr("Close window to the tray"),
                        tr("Window behavior"),
                        "\u{e7ed}",
                        &mut state.behavior.close_to_tray,
                    );
                },
            );
            if !self.tray_available() {
                ui.label(
                    RichText::new(tr(
                        "The tray is currently unavailable; closing will exit safely.",
                    ))
                    .small()
                    .color(p.muted),
                );
            }
            toggle(
                ui,
                p,
                tr("Notify when a download finishes"),
                "",
                "\u{e7ed}",
                &mut state.behavior.notify_complete,
            );
            toggle(
                ui,
                p,
                tr("Notify when a download needs attention"),
                "",
                "\u{e7ed}",
                &mut state.behavior.notify_failed,
            );
            toggle(
                ui,
                p,
                tr("Show notifications from scripts"),
                "",
                "\u{e7ed}",
                &mut state.behavior.notify_scripts,
            );
            toggle(
                ui,
                p,
                tr("Play a sound when downloads finish"),
                "",
                "\u{e767}",
                &mut state.interface.completion_sound,
            );
        });
    }
}
