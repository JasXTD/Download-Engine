// Native presentation only. Download commands still go through the scheduler.
use super::*;
use download_engine::table_layout::Column;
use eframe::egui::{
    Align, Align2, FontFamily, FontId, Layout, Rect, Sense, Stroke, Vec2, pos2, vec2,
};

const HELP: &str = "\u{e897}";
const FILE: &str = "\u{e7c3}";
const FOLDER: &str = "folder";
const PLAY: &str = "\u{e768}";
const PAUSE: &str = "\u{e769}";
/// Width the download list keeps for its vertical scrollbar.
const SCROLLBAR_STRIP: f32 = 14.0;

pub(super) fn icon(ui: &egui::Ui, at: egui::Pos2, glyph: &str, size: f32, color: Color32) {
    // Draw these shapes directly instead of relying on optional Windows glyphs.
    let point = |x: f32, y: f32| at + vec2(x - 10.0, y - 10.0) * (size / 20.0);
    let stroke = Stroke::new(1.4, color);
    match glyph {
        "layers" => {
            ui.painter().add(egui::Shape::closed_line(
                [(10.0, 2.0), (19.0, 7.0), (10.0, 12.0), (1.0, 7.0)]
                    .into_iter()
                    .map(|(x, y)| point(x, y))
                    .collect(),
                stroke,
            ));
            for y in [11.0, 15.0] {
                ui.painter().add(egui::Shape::line(
                    [(1.0, y), (10.0, y + 5.0), (19.0, y)]
                        .into_iter()
                        .map(|(x, y)| point(x, y))
                        .collect(),
                    stroke,
                ));
            }
            return;
        }
        "folder" => {
            ui.painter().add(egui::Shape::closed_line(
                [
                    (1.0, 4.0),
                    (8.0, 4.0),
                    (10.0, 7.0),
                    (19.0, 7.0),
                    (19.0, 17.0),
                    (1.0, 17.0),
                ]
                .into_iter()
                .map(|(x, y)| point(x, y))
                .collect(),
                stroke,
            ));
            return;
        }
        "archive" => {
            ui.painter().rect_stroke(
                Rect::from_min_max(point(3.0, 2.0), point(17.0, 18.0)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            for y in [4.0, 7.0, 10.0, 13.0] {
                ui.painter()
                    .line_segment([point(9.0, y), point(12.0, y)], stroke);
            }
            return;
        }
        "app" => {
            for (x, y) in [(2.0, 2.0), (11.0, 2.0), (2.0, 11.0), (11.0, 11.0)] {
                ui.painter().rect_stroke(
                    Rect::from_min_max(point(x, y), point(x + 7.0, y + 7.0)),
                    1.0,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
            return;
        }
        _ => {}
    }
    let family = if cfg!(windows) {
        FontFamily::Name("icons".into())
    } else {
        FontFamily::Proportional
    };
    ui.painter().text(
        at,
        Align2::CENTER_CENTER,
        glyph,
        FontId::new(size, family),
        color,
    );
}
pub(super) fn gradient_panel(
    painter: &egui::Painter,
    rect: Rect,
    radius: f32,
    top: Color32,
    bottom: Color32,
) {
    let mut mesh = egui::Mesh::default();
    let color = |y: f32| {
        let t = ((y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        Color32::from_rgb(
            egui::lerp(top.r() as f32..=bottom.r() as f32, t) as u8,
            egui::lerp(top.g() as f32..=bottom.g() as f32, t) as u8,
            egui::lerp(top.b() as f32..=bottom.b() as f32, t) as u8,
        )
    };
    mesh.colored_vertex(rect.center(), color(rect.center().y));
    for (center, start) in [
        (rect.left_top() + vec2(radius, radius), std::f32::consts::PI),
        (
            rect.right_top() + vec2(-radius, radius),
            1.5 * std::f32::consts::PI,
        ),
        (rect.right_bottom() - vec2(radius, radius), 0.0),
        (
            rect.left_bottom() + vec2(radius, -radius),
            std::f32::consts::FRAC_PI_2,
        ),
    ] {
        for step in 0..=12 {
            let angle = start + step as f32 / 12.0 * std::f32::consts::FRAC_PI_2;
            let point = center + vec2(angle.cos(), angle.sin()) * radius;
            mesh.colored_vertex(point, color(point.y));
        }
    }
    let count = mesh.vertices.len() as u32 - 1;
    for index in 1..=count {
        mesh.add_triangle(0, index, index % count + 1);
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn action(ui: &mut egui::Ui, label: &str, glyph: &str, size: Vec2, p: Palette) -> egui::Response {
    let response = ui.add_sized(
        size,
        egui::Button::new(if label.is_empty() {
            String::new()
        } else {
            format!("    {label}")
        }),
    );
    icon(
        ui,
        pos2(response.rect.left() + 16.0, response.rect.center().y),
        glyph,
        16.0,
        if ui.is_enabled() { p.text } else { p.muted },
    );
    response
}

/// Height of single-line text boxes, dropdowns, number boxes and regular buttons.
/// Primary actions (dialog footers, page headers) stay 32px; compact areas such as
/// the toolbar, table rows and the status window keep their own sizes.
pub(super) const CONTROL_HEIGHT: f32 = 26.0;
/// Height of primary actions.
pub(super) const PRIMARY_HEIGHT: f32 = 32.0;

/// A single-line text box at [`CONTROL_HEIGHT`], its text centered vertically.
pub(super) fn line_edit(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text)
        .min_size(vec2(0.0, CONTROL_HEIGHT))
        .vertical_align(Align::Center)
}

/// Checkbox size. Checkboxes are square with small corners so they never look like
/// the round radio buttons: egui rounds a checkbox like a button, which turns a 14px
/// box into a circle.
pub(super) const CHECKBOX_SIZE: f32 = 14.0;
const CHECKBOX_CORNER: u8 = 4;

/// egui's checkbox in the app's square style. Use it, or `ui.square_checkbox`, for
/// every checkbox.
pub(super) struct Checkbox<'a>(egui::Checkbox<'a>);

impl<'a> Checkbox<'a> {
    pub(super) fn new(checked: &'a mut bool, label: impl egui::IntoAtoms<'a>) -> Self {
        Self(egui::Checkbox::new(checked, label))
    }
}

impl egui::Widget for Checkbox<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        // Restyle this one widget in place rather than in a child Ui, so it lays out
        // (and wraps to the next row) exactly like `ui.checkbox`.
        let style = ui.style().clone();
        ui.spacing_mut().icon_width = CHECKBOX_SIZE;
        ui.spacing_mut().icon_width_inner = 8.0;
        let widgets = &mut ui.visuals_mut().widgets;
        for visuals in [
            &mut widgets.noninteractive,
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
            &mut widgets.open,
        ] {
            visuals.corner_radius = CHECKBOX_CORNER.into();
        }
        let response = ui.add(self.0);
        ui.set_style(style);
        response
    }
}

pub(super) trait SquareCheckbox {
    /// `Ui::checkbox` in the app's square style.
    fn square_checkbox<'a>(
        &mut self,
        checked: &'a mut bool,
        label: impl egui::IntoAtoms<'a>,
    ) -> egui::Response;
}

impl SquareCheckbox for egui::Ui {
    fn square_checkbox<'a>(
        &mut self,
        checked: &'a mut bool,
        label: impl egui::IntoAtoms<'a>,
    ) -> egui::Response {
        self.add(Checkbox::new(checked, label))
    }
}

/// The width an `egui::Button` with this label takes. Lets a row of buttons be placed
/// so it ends exactly at an edge while keeping left-to-right keyboard order.
pub(super) fn button_width(ui: &egui::Ui, label: &str) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    ui.painter()
        .layout_no_wrap(label.to_owned(), font, Color32::WHITE)
        .size()
        .x
        + 2.0 * ui.spacing().button_padding.x
}

/// The width of the widest of these checkboxes: a label column that rows share so
/// the controls after their checkboxes start at one x in every language.
pub(super) fn checkbox_column(ui: &egui::Ui, labels: &[&str]) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    labels
        .iter()
        .map(|label| {
            ui.painter()
                .layout_no_wrap((*label).to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0, f32::max)
        + CHECKBOX_SIZE
        + ui.spacing().icon_spacing
}

/// A checkbox in a label column of `width` (see [`checkbox_column`]).
pub(super) fn column_checkbox(
    ui: &mut egui::Ui,
    width: f32,
    value: &mut bool,
    label: &str,
) -> egui::Response {
    ui.allocate_ui_with_layout(
        vec2(width, ui.spacing().interact_size.y),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.square_checkbox(value, label)
        },
    )
    .inner
}

/// The shared close control: an outlined square with a centered ×. Use it for every
/// dialog, window and panel that closes, so the X looks the same everywhere.
pub(super) fn close_button(ui: &mut egui::Ui, side: f32, hover: &str) -> egui::Response {
    let (_, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    paint_close(ui, &response);
    response.on_hover_text(hover)
}

/// Paints the shared × for a click response the caller already owns (for example
/// one with a stable id). Hover, press, focus and disabled states follow egui buttons.
pub(super) fn paint_close(ui: &egui::Ui, response: &egui::Response) {
    let p = Palette::new(ui.visuals().dark_mode);
    let visuals = ui.style().interact(response);
    let stroke =
        if response.hovered() || response.has_focus() || response.is_pointer_button_down_on() {
            visuals.bg_stroke
        } else {
            Stroke::new(1.0, p.muted)
        };
    let rect = response.rect.expand(visuals.expansion);
    ui.painter().rect(
        rect,
        4,
        visuals.weak_bg_fill,
        stroke,
        egui::StrokeKind::Inside,
    );
    let glyph = ui.painter().layout_no_wrap(
        "×".into(),
        FontId::proportional(response.rect.height() * 36.0 / 32.0),
        p.text,
    );
    // Align the visible mark itself; font line boxes include baseline space.
    let origin = response.rect.center() - glyph.mesh_bounds.center().to_vec2();
    ui.painter().galley(origin, glyph, p.text);
}

pub(super) fn dialog_header_enabled(ui: &mut egui::Ui, title: &str, enabled: bool) -> bool {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 32.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            // Long titles (file names) end with … instead of pushing the × out of view.
            let title_width = (ui.available_width() - 40.0).max(0.0);
            ui.allocate_ui_with_layout(
                vec2(title_width, 32.0),
                Layout::left_to_right(Align::Center),
                |ui| ui.add(egui::Label::new(RichText::new(title).heading()).truncate()),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let close = ui
                    .add_enabled_ui(enabled, |ui| close_button(ui, 32.0, tr("Close")))
                    .inner;
                close.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        close.enabled(),
                        tf!("Close {title}", title = title),
                    )
                });
                close.clicked()
            })
            .inner
        },
    )
    .inner
}

impl Desktop {
    pub fn configure_appearance(&self, ctx: &egui::Context) {
        let appearance = self
            .preferences_window
            .as_ref()
            .map_or(self.interface.appearance, |state| {
                state.interface.appearance
            });
        ctx.set_theme(match appearance {
            Appearance::System => egui::ThemePreference::System,
            Appearance::Light => egui::ThemePreference::Light,
            Appearance::Dark => egui::ThemePreference::Dark,
        });
    }
    pub(super) fn save_interface(&mut self, ctx: &egui::Context, value: InterfacePreferences) {
        match self.store.save_interface_preferences(value) {
            Ok(()) => {
                let language_changed = self.interface.language != value.language;
                self.interface = value;
                self.configure_appearance(ctx);
                if language_changed {
                    let _language = download_engine::i18n::scope(value.language);
                    self.refresh_settings_summary();
                    if let Some(tray) = &self.tray
                        && let Err(error) = tray.set_language(value.language)
                    {
                        self.notice = Some(Notice::error(error.to_string()));
                    }
                    ctx.request_repaint();
                }
            }
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot save interface settings: {error}",
                    error = error
                )))
            }
        }
    }
    pub(super) fn notice(&mut self, ui: &mut egui::Ui, p: Palette) {
        if let Some(message) = self.notice.clone() {
            ui.horizontal(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt("notice")
                    .max_height(40.0)
                    .max_width((ui.available_width() - 80.0).max(100.0))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(&message.message)
                                .small()
                                .color(message.color(p)),
                        );
                    });
                if ui.button(tr("Dismiss")).clicked() {
                    self.notice = None;
                }
            });
        }
    }
    fn keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() || egui::Popup::is_any_open(ctx) {
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F1)) {
            self.open_guide("Getting started");
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
            self.open_add();
            return;
        }
        self.list_keyboard(ctx);
        if self.view == View::Logs {
            self.logs.keyboard_shortcuts(ctx);
            return;
        }
        let search = egui::Id::new("filename-search");
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            self.view = View::Downloads;
            ctx.memory_mut(|memory| memory.request_focus(search));
        }
        // egui releases focus on Escape before drawing this frame.
        if ctx.memory(|memory| memory.has_focus(search) || memory.had_focus_last_frame(search))
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            if self.search.is_empty() {
                ctx.memory_mut(|memory| memory.surrender_focus(search));
            } else {
                self.search.clear();
                self.reconcile_selection();
                ctx.memory_mut(|memory| memory.request_focus(search));
            }
        }
    }

    pub(super) fn draw(&mut self, ui: &mut egui::Ui) {
        let _language = download_engine::i18n::scope(self.interface.language);
        let ctx = ui.ctx().clone();
        // A child editor closes back into the same category, with its draft intact.
        if self.dialog.is_none() && self.preferences_window.is_some() {
            self.refresh_settings_summary();
            self.dialog = Some(Dialog::PreferencesWindow);
        }
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
            && std::env::var("IDM_EX_TEST_VIEW").as_deref() == Ok("downloads-focused")
        {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("download-list")));
        }
        self.keyboard_shortcuts(&ctx);
        if self.dialog.is_some() || self.view != View::Downloads {
            self.table.cancel();
        }
        let p = Palette::new(ui.visuals().dark_mode);
        self.footer(ui, p);
        self.menu_bar(ui, p);
        self.toolbar(ui, p);
        self.sidebar(ui, p);
        if self.view == View::Downloads
            && self.interface.show_details
            && let Some(job) = self
                .jobs
                .iter()
                .find(|j| self.selected.as_deref() == Some(&j.id))
                .cloned()
        {
            self.details(ui, &job, p);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.surface).inner_margin(18))
            .show(ui, |ui| {
                if ui.ctx().content_rect().height() < 700.0 {
                    ui.spacing_mut().item_spacing.y = 4.0;
                }
                // Pages of controls get the shared control height as their minimum, as
                // dialogs do. The download list keeps its compact rows and details.
                if self.view != View::Downloads {
                    ui.spacing_mut().interact_size.y = CONTROL_HEIGHT;
                }
                self.completion_banner(ui);
                self.grabber_banner(ui, p);
                match self.view {
                    View::Downloads => self.content(ui, p),
                    View::Queues | View::Scheduler => self.overview(ui, p),
                    View::Logs => self.logs_page(ui, p),
                    View::Automation => self.automation_page(ui, p),
                    View::Guide => self.guide_page(ui, p),
                    View::Changelog => self.changelog_page(ui, p),
                }
            });
        self.clipboard_card(&ctx, p);
        match self.dialog {
            Some(Dialog::Automation) => self.automation_dialog(&ctx, p),
            Some(Dialog::ScriptSample) => self.script_sample_dialog(&ctx, p),
            Some(Dialog::ScriptReplace) => self.script_replace_dialog(&ctx, p),
            Some(Dialog::Confirm) => self.confirmation_dialog(&ctx, p),
            Some(Dialog::About) => self.about_dialog(&ctx),
            Some(Dialog::Legal(page)) => self.legal_dialog(&ctx, page),
            Some(Dialog::Proxy) => self.proxy_dialog(&ctx, p),
            Some(Dialog::Connections) => self.connections_dialog(&ctx, p),
            Some(Dialog::ZipPreview) => self.zip_dialog(&ctx, p),
            Some(Dialog::Hls) => self.hls_dialog(&ctx, p),
            Some(Dialog::BrowserVideo) => self.browser_video_dialog(&ctx, p),
            Some(Dialog::Grabber) => self.grabber_dialog(&ctx, p),
            Some(Dialog::LinkFallback) => self.link_fallback_dialog(&ctx, p),
            Some(Dialog::AntivirusSettings) => self.antivirus_settings_dialog(&ctx, p),
            Some(Dialog::AntivirusResult) => self.antivirus_result_dialog(&ctx, p),
            Some(Dialog::AntivirusScan) => self.antivirus_scan_dialog(&ctx, p),
            Some(Dialog::Checksum) => self.checksum_dialog(&ctx, p),
            Some(Dialog::Columns) => self.columns_dialog(&ctx, p),
            Some(Dialog::Add) => self.add_dialog(&ctx, p),
            Some(Dialog::Browsers) => self.browser_dialog(&ctx, p),
            Some(Dialog::UpdateLink) => self.link_dialog(&ctx, p),
            Some(Dialog::Authentication) => self.authentication_dialog(&ctx, p),
            Some(Dialog::SpeedLimits) => self.speed_limits_dialog(&ctx, p),
            Some(Dialog::DataQuotas) => self.data_quotas_dialog(&ctx, p),
            Some(Dialog::PreferencesWindow) => self.preferences_window_dialog(&ctx, p),
            Some(Dialog::Mcp) => self.mcp_dialog(&ctx, p),
            Some(Dialog::DownloadFolders) => self.download_folders_dialog(&ctx, p),
            Some(Dialog::ListTransfer) => self.list_transfer_dialog(&ctx, p),
            Some(dialog) => self.queue_dialog(&ctx, p, dialog),
            None => {}
        }
        self.download_status_windows(&ctx);
    }
}

impl Desktop {
    fn filters(&mut self, ui: &mut egui::Ui, p: Palette) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(self.filter.title()).size(17.0).strong());
            ui.label(
                RichText::new(tf!(
                    "{} files",
                    self.jobs.iter().filter(|job| self.accepts(job)).count()
                ))
                .small()
                .color(p.muted),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let control_height = CONTROL_HEIGHT;
                if !self.search.is_empty()
                    && ui
                        .add_sized([54.0, control_height], egui::Button::new(tr("Clear")))
                        .clicked()
                {
                    self.search.clear();
                    self.reconcile_selection();
                }
                if ui
                    .add_sized(
                        [206.0, control_height],
                        line_edit(&mut self.search)
                            .id(egui::Id::new("filename-search"))
                            .desired_width(190.0)
                            .margin(egui::Margin::symmetric(8, 4))
                            .hint_text(tr("Search downloads…"))
                            .char_limit(256),
                    )
                    .on_hover_text(tr("Search filenames (Ctrl+F). Escape clears the search."))
                    .changed()
                {
                    self.reconcile_selection();
                }
            });
        });
        ui.horizontal(|ui| {
            let active_queue = self.queues.iter().find(|queue| queue.id == self.queue_id);
            let queue = active_queue
                .map(|q| display_queue_name(&q.id, &q.name))
                .unwrap_or(tr("Main queue"));
            let running = active_queue.is_some_and(|q| q.running);
            let (dot_rect, _) = ui.allocate_exact_size(vec2(8.0, 14.0), Sense::hover());
            ui.painter().circle_filled(
                dot_rect.center(),
                3.5,
                if running { p.accent } else { p.orange },
            );
            ui.label(RichText::new(queue).small().strong().color(p.text));
            if let Some(q) = active_queue {
                ui.label(
                    RichText::new(tf!(
                        "· {} · max {} active",
                        if q.running {
                            tr("Running")
                        } else {
                            tr("Paused")
                        },
                        q.max_active
                    ))
                    .small()
                    .color(p.muted),
                );
            }
            if let Some(category) = self.category {
                ui.label(RichText::new("/").small().color(p.muted));
                if ui
                    .small_button(format!("{} ×", tr(category.label())))
                    .on_hover_text(tr("Show all file types"))
                    .clicked()
                {
                    self.category = None;
                    self.reconcile_selection();
                }
            }
        });
    }
    fn content(&mut self, ui: &mut egui::Ui, p: Palette) {
        let small = ui.ctx().content_rect().height() < 700.0;
        ui.add_space(if small { 4.0 } else { 9.0 });
        self.filters(ui, p);
        ui.add_space(if small { 3.0 } else { 7.0 });
        self.notice(ui, p);
        let reset_scroll =
            self.table
                .prepare_list(&self.queue_id, self.filter, self.category, &self.search);
        let jobs: Vec<_> = self
            .jobs
            .iter()
            .filter(|job| self.accepts(job))
            .cloned()
            .collect();
        if jobs.is_empty() {
            ui.add_space(25.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(if self.jobs.is_empty() {
                        tr("Ready for your next file")
                    } else {
                        tr("No downloads match these filters")
                    })
                    .size(19.0)
                    .strong(),
                );
                ui.label(
                    RichText::new(if self.jobs.is_empty() {
                        tr("Add a download or drop a file link from your browser.")
                    } else {
                        tr("Try another status, category, or filename.")
                    })
                    .color(p.muted),
                );
                ui.add_space(10.0);
                if self.jobs.is_empty() {
                    if ui.add(primary(tr("Add download"), p)).clicked() {
                        self.open_add();
                    }
                } else if ui.button(tr("Clear filters")).clicked() {
                    self.filter = Filter::All;
                    self.category = None;
                    self.search.clear();
                    self.reconcile_selection();
                }
            });
            return;
        }
        egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(0)
            .show(ui, |ui| {
                // Keep overflow discoverable, including when the mouse is outside the table.
                ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                let available = (ui.available_width() - SCROLLBAR_STRIP).max(300.0);
                let columns = table::widths(
                    available,
                    self.interface.columns,
                    self.table.preview(self.interface.table_layout),
                );
                let width = columns.iter().map(|(_, w)| w).sum::<f32>().max(available);
                // The strip kept for the scrollbar is painted like the header, so it reads
                // as part of the frame rather than a gap; the scrollbar draws over it.
                // Wider saved columns scroll sideways through it, so leave it bare then.
                if width <= available + 0.5 {
                    let area = ui.available_rect_before_wrap();
                    let strip = Rect::from_min_max(
                        pos2(area.right() - SCROLLBAR_STRIP, area.top()),
                        area.right_bottom(),
                    );
                    ui.painter().rect_filled(strip, 0.0, p.background);
                    ui.painter()
                        .vline(strip.left(), strip.y_range(), Stroke::new(1.0, p.border));
                }
                egui::ScrollArea::horizontal()
                    .id_salt("download-columns-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let (header, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
                        ui.painter().rect_filled(header, 0.0, p.background);
                        if let Some(layout) =
                            self.table
                                .header(ui, header, &columns, self.interface.table_layout, p)
                        {
                            let mut value = self.interface;
                            value.table_layout = layout;
                            self.save_interface(ui.ctx(), value);
                        }
                        let row_height = if self.interface.compact_rows || small {
                            40.0
                        } else {
                            48.0
                        };
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let list_id = egui::Id::new("download-list");
                        let list_rect = ui.available_rect_before_wrap();
                        let list_response =
                            ui.interact(list_rect, list_id, Sense::focusable_noninteractive());
                        if list_response.has_focus() {
                            ui.memory_mut(|m| {
                                m.set_focus_lock_filter(
                                    list_id,
                                    egui::EventFilter {
                                        vertical_arrows: true,
                                        horizontal_arrows: true,
                                        ..Default::default()
                                    },
                                )
                            });
                        }
                        let mut scroll = egui::ScrollArea::vertical()
                            .id_salt("download-table")
                            .auto_shrink([false, false])
                            .min_scrolled_height(0.0);
                        if reset_scroll {
                            scroll = scroll.vertical_scroll_offset(0.0);
                        }
                        if std::mem::take(&mut self.selection.scroll_to_focus)
                            && let Some(index) = jobs
                                .iter()
                                .position(|j| self.selected.as_deref() == Some(&j.id))
                        {
                            let id = ui.make_persistent_id("download-table");
                            let offset = egui::scroll_area::State::load(ui.ctx(), id)
                                .map_or(0.0, |s| s.offset.y);
                            let top = index as f32 * row_height;
                            let visible_height = ui.available_height().max(row_height);
                            let next = if top < offset {
                                top
                            } else if top + row_height > offset + visible_height {
                                top + row_height - visible_height
                            } else {
                                offset
                            };
                            scroll = scroll.vertical_scroll_offset(next);
                        }
                        scroll.show_rows(ui, row_height, jobs.len(), |ui, range| {
                            for index in range {
                                let job = &jobs[index];
                                ui.push_id(&job.id, |ui| {
                                    self.table_row(
                                        ui,
                                        job,
                                        index + 1,
                                        (width, row_height),
                                        &columns,
                                        p,
                                    )
                                });
                            }
                        });
                    });
            });
    }
    pub(super) fn select_download(&mut self, ctx: &egui::Context, job: &Job) {
        self.set_selection(Some(job.id.clone()));
        self.focus_download_list(ctx);
    }
    fn select_download_with_modifiers(&mut self, ctx: &egui::Context, job: &Job) {
        self.choose_download(&job.id, ctx.input(|i| i.modifiers));
        self.focus_download_list(ctx);
    }
    fn focus_download_list(&mut self, ctx: &egui::Context) {
        ctx.memory_mut(|m| {
            let list = egui::Id::new("download-list");
            m.request_focus(list);
            m.set_focus_lock_filter(
                list,
                egui::EventFilter {
                    vertical_arrows: true,
                    horizontal_arrows: true,
                    ..Default::default()
                },
            );
        });
        if !self.interface.show_details {
            let mut value = self.interface;
            value.show_details = true;
            self.save_interface(ctx, value);
        }
    }
    fn table_row(
        &mut self,
        ui: &mut egui::Ui,
        job: &Job,
        index: usize,
        size: (f32, f32),
        columns: &[(Column, f32)],
        p: Palette,
    ) {
        let (width, height) = size;
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
        ui.painter().rect_filled(
            rect,
            0.0,
            if self.is_selected(&job.id) {
                p.selected
            } else {
                p.surface
            },
        );
        ui.painter()
            .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.border));
        if self.is_selected(&job.id) {
            ui.painter().rect_filled(
                Rect::from_min_size(rect.min, vec2(3.0, height)),
                0.0,
                p.blue,
            );
        }
        // Keyboard focus belongs to the active row, not the empty list viewport.
        if ui.memory(|m| m.has_focus(egui::Id::new("download-list")))
            && (self.selected.as_deref() == Some(job.id.as_str())
                || self.selected.is_none() && index == 1)
        {
            ui.painter().rect_stroke(
                rect,
                0.0,
                Stroke::new(1.0, p.blue),
                egui::StrokeKind::Inside,
            );
        }
        if response.clicked() {
            self.select_download_with_modifiers(ui.ctx(), job);
        }
        if response.double_clicked() {
            self.open_download_status(job);
        }
        let mut context_response = response.clone();
        let rate = self.rate(job);
        let compact = self.interface.compact_rows || height < 60.0;
        let inline_progress = !columns
            .iter()
            .any(|(column, _)| *column == Column::Progress)
            && matches!(job.status, Status::Downloading | Status::Paused)
            && progress(job).is_some();
        let small_height = ui.text_style_height(&egui::TextStyle::Small);
        for (column, raw) in table::cells(rect, columns) {
            // Center the actual content block, including two-line status/speed
            // cells, instead of giving each column its own top padding.
            let content_height = match column {
                Column::Name | Column::Actions => 30.0,
                Column::Status if inline_progress => small_height + 3.0 + 3.0,
                Column::Progress => small_height.max(ui.spacing().interact_size.y),
                Column::Speed if self.limits.downloads.contains_key(&job.id) => {
                    small_height * 2.0 + 3.0
                }
                Column::Size | Column::Added if !compact => small_height * 2.0 + 3.0,
                _ => small_height,
            };
            let bounds = raw.shrink2(vec2(8.0, 0.0));
            let rect = Align2::LEFT_CENTER
                .align_size_within_rect(vec2(bounds.width(), content_height), bounds);
            let mut cell = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(column as u8)
                    .max_rect(rect)
                    .layout(Layout::top_down(Align::Min)),
            );
            cell.set_clip_rect(rect.intersect(ui.clip_rect()));
            cell.spacing_mut().item_spacing = vec2(5.0, 3.0);
            match column {
                Column::Number => {
                    cell.label(RichText::new(index.to_string()).small().color(p.muted));
                }
                Column::Name => {
                    cell.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(20.0, 30.0), Sense::hover());
                        icon(
                            ui,
                            r.center(),
                            super::shell::category_icon(job.category()),
                            19.0,
                            match job.category() {
                                Category::Programs => p.blue,
                                Category::Compressed => p.orange,
                                Category::Video | Category::Music => p.accent,
                                _ => p.muted,
                            },
                        );
                        let layout = if compact {
                            Layout::left_to_right(Align::Center)
                        } else {
                            Layout::top_down(Align::Min)
                        };
                        ui.with_layout(layout, |ui| {
                            let name_response = ui
                                .add(egui::Button::new(&job.name).frame(false).truncate())
                                .on_hover_text(format!(
                                    "{}\n{}\n{}",
                                    job.name,
                                    tr(job.category().label()),
                                    job.url
                                ));
                            if name_response.clicked() {
                                self.select_download_with_modifiers(ui.ctx(), job);
                            }
                            if name_response.double_clicked() {
                                self.open_download_status(job);
                            }
                            // The filename is its own button; include its secondary
                            // click so it opens the same menu as the rest of the row.
                            context_response = context_response.union(name_response);
                            if !compact {
                                ui.label(
                                    RichText::new(tr(job.category().label()))
                                        .small()
                                        .color(p.muted),
                                );
                            }
                        });
                    });
                }
                Column::Status => {
                    let label = status_label(
                        job,
                        self.queues
                            .iter()
                            .any(|q| q.id == self.queue_id && q.running)
                            || self.immediate_downloads.contains(&job.id),
                        chrono::Utc::now().timestamp_millis(),
                    );
                    let label = if inline_progress && let Some(fraction) = progress(job) {
                        format!("{label}   {:.1}%", fraction * 100.0)
                    } else {
                        label
                    };
                    cell.add(
                        egui::Label::new(RichText::new(label).small().color(p.status(job.status)))
                            .truncate(),
                    );
                    if inline_progress && let Some(fraction) = progress(job) {
                        let (bar, _) = cell
                            .allocate_exact_size(vec2(cell.available_width(), 3.0), Sense::hover());
                        cell.painter().rect_filled(bar, 2.0, p.track);
                        cell.painter().rect_filled(
                            Rect::from_min_size(
                                bar.min,
                                vec2(bar.width() * fraction, bar.height()),
                            ),
                            2.0,
                            p.status(job.status),
                        );
                    }
                }
                Column::Progress => {
                    cell.horizontal(|ui| {
                        let fraction = progress(job);
                        let bar_width = (ui.available_width() - 40.0).max(20.0);
                        let (bar, _) = ui.allocate_exact_size(vec2(bar_width, 7.0), Sense::hover());
                        ui.painter().rect_filled(bar, 4.0, p.track);
                        if let Some(value) = fraction {
                            ui.painter().rect_filled(
                                Rect::from_min_size(
                                    bar.min,
                                    vec2(bar.width() * value, bar.height()),
                                ),
                                4.0,
                                p.status(job.status),
                            );
                        }
                        ui.label(
                            RichText::new(
                                fraction
                                    .map(|v| format!("{:.0}%", v * 100.0))
                                    .unwrap_or_else(|| "—".into()),
                            )
                            .small(),
                        );
                    });
                }
                Column::Size => {
                    cell.label(
                        RichText::new(job.total.map(bytes).unwrap_or_else(|| tr("Unknown").into()))
                            .small(),
                    );
                    if !compact {
                        cell.label(
                            RichText::new(
                                job.total
                                    .map(|n| format!("of {}", bytes(n)))
                                    .unwrap_or_else(|| tr("Size unknown").into()),
                            )
                            .small()
                            .color(p.muted),
                        );
                    }
                }
                Column::Speed => {
                    cell.label(
                        RichText::new(if job.status == Status::Downloading {
                            format!("{}/s", bytes(rate as u64))
                        } else {
                            "—".into()
                        })
                        .small(),
                    );
                    if let Some(cap) = self.limits.downloads.get(&job.id) {
                        cell.add(
                            egui::Label::new(
                                RichText::new(format!("≤ {}", speed_limit_label(*cap)))
                                    .small()
                                    .color(p.muted),
                            )
                            .truncate(),
                        )
                        .on_hover_text(tr("File speed limit; the total limit also applies."));
                    }
                }
                Column::Eta => {
                    cell.label(RichText::new(eta(job, rate)).small());
                }
                Column::Added => {
                    let date = i64::try_from(job.created)
                        .ok()
                        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                        .map(|t| t.with_timezone(&chrono::Local));
                    if let Some(date) = date {
                        cell.label(RichText::new(date.format("%d %b %Y").to_string()).small());
                        if !compact {
                            cell.label(
                                RichText::new(date.format("%H:%M").to_string())
                                    .small()
                                    .color(p.muted),
                            );
                        }
                    }
                }
                Column::Actions => {
                    cell.spacing_mut().button_padding.y = 4.0;
                    cell.spacing_mut().interact_size.y = 30.0;
                    cell.horizontal(|ui| {
                        let (glyph, label) = match job.status {
                            Status::Complete => (FOLDER, tr("Open folder")),
                            Status::Downloading | Status::Queued => (PAUSE, tr("Pause")),
                            Status::Paused => (PLAY, resume_action(job)),
                            Status::Failed => (HELP, tr("Review")),
                        };
                        let button =
                            action(ui, "", glyph, vec2(30.0, 30.0), p).on_hover_text(label);
                        button.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                ui.is_enabled(),
                                format!("{label}: {}", job.name),
                            )
                        });
                        if button.clicked() {
                            self.select_download(ui.ctx(), job);
                            match job.status {
                                Status::Complete => self.show_folder(job),
                                Status::Downloading | Status::Queued => {
                                    self.command(Command::Pause(job.id.clone()))
                                }
                                Status::Paused => {
                                    self.expect_download_status(&job.id);
                                    self.command(Command::Resume(job.id.clone()));
                                }
                                Status::Failed => {}
                            }
                        }
                        self.file_menu_button(ui, "⋮", job);
                    });
                }
            }
        }
        if context_response.secondary_clicked() && !self.is_selected(&job.id) {
            self.set_selection(Some(job.id.clone()));
        }
        let menu_config = theme::menu_config();
        let popup = egui::Popup::context_menu(&context_response)
            .style(menu_config.style.clone())
            .info(egui::UiStackInfo::new(egui::UiKind::Menu).with_tag_value(
                egui::containers::menu::MenuConfig::MENU_CONFIG_TAG,
                menu_config,
            ));
        #[cfg(feature = "screenshot")]
        let popup = if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
            && std::env::var("IDM_EX_TEST_MENU")
                .is_ok_and(|path| path.starts_with("Download context|"))
            && self.selected.as_deref() == Some(job.id.as_str())
        {
            // Render the real menu at a stable row anchor without an OS right-click.
            popup.open(true).at_position(response.rect.center())
        } else {
            popup
        };
        popup.show(|ui| {
            self.file_menu(ui, job);
            #[cfg(feature = "screenshot")]
            shell::capture_menu_contents(ui, "Download context");
        });
    }
}

impl Desktop {
    fn file_menu_button(&mut self, ui: &mut egui::Ui, label: &str, job: &Job) {
        #[cfg(feature = "screenshot")]
        let capture_label = if label == "⋮" {
            tr("Download actions")
        } else {
            label
        };
        let _response = egui::containers::menu::MenuButton::new(label)
            .config(theme::menu_config())
            .ui(ui, |ui| {
                self.file_menu(ui, job);
                #[cfg(feature = "screenshot")]
                shell::capture_menu_contents(ui, capture_label);
            });
        #[cfg(feature = "screenshot")]
        if self.selected.as_deref() == Some(job.id.as_str()) {
            shell::capture_target(ui, capture_label, &_response.0);
        }
    }

    pub(super) fn duplicate_download(&mut self, ctx: &egui::Context, job: &Job) {
        if job.hls.is_some() {
            self.open_hls(&job.url);
            return;
        }
        if job
            .authentication
            .as_ref()
            .is_some_and(|b| b.kind == AuthKind::Browser)
        {
            self.notice = Some(Notice::info(tr("To duplicate this protected file, send it again using Use this site's login in the browser extension.").into()));
            return;
        }
        // A new job never inherits credentials or partial bytes. Filename collision
        // handling remains the engine's atomic publication path.
        let result = (|| {
            let mut copy = self.store.new_download(
                &job.url,
                Some(&job.name),
                Some(&job.directory),
                Uuid::new_v4().to_string(),
            )?;
            copy.integrity.expected.clone_from(&job.integrity.expected);
            copy.ftp.clone_from(&job.ftp);
            if let Some(ftp) = &mut copy.ftp {
                ftp.reset();
            }
            if job.authentication.is_some() {
                copy.status = Status::Paused;
            }
            self.store
                .enqueue_batch(&[copy], &self.queue_id)
                .map(|mut jobs| jobs.remove(0))
        })();
        match result {
            Ok(copy) => {
                self.filter = Filter::All;
                self.category = None;
                self.search.clear();
                self.view = View::Downloads;
                self.force_refresh();
                self.select_download(ctx, &copy);
                if job.authentication.is_some() {
                    self.open_authentication(&copy);
                }
            }
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot duplicate download: {error}",
                    error = error
                )))
            }
        }
    }
    pub(super) fn open_file(&mut self, job: &Job) {
        match self.store.antivirus_settings() {
            Ok(settings) if settings.enabled => {
                self.review_scan(job, true);
                return;
            }
            Ok(_) => {}
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot open file: {error}",
                    error = error
                )));
                return;
            }
        }
        let result = (|| {
            anyhow::ensure!(
                job.status == Status::Complete,
                tr("Finish the download before opening it")
            );
            anyhow::ensure!(
                job.integrity.matches() != Some(false),
                tr(
                    "The last SHA-256 verification did not match. Check the file checksum before opening it."
                )
            );
            let path = job
                .saved_path
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!(tr("The saved file location is unavailable")))?;
            download_engine::paths::open_download(path)
        })();
        if let Err(error) = result {
            self.notice = Some(Notice::error(tf!(
                "Cannot open file: {error}",
                error = error
            )));
        }
    }
    pub(super) fn open_file_with(&mut self, job: &Job) {
        let result = (|| {
            anyhow::ensure!(
                job.status == Status::Complete,
                tr("Finish the download before opening it")
            );
            anyhow::ensure!(
                job.integrity.matches() != Some(false),
                tr(
                    "The last SHA-256 verification did not match. Check the file checksum before opening it."
                )
            );
            let path = job
                .saved_path
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!(tr("The saved file location is unavailable")))?;
            download_engine::paths::open_download_with(path)
        })();
        if let Err(error) = result {
            self.notice = Some(Notice::error(tf!(
                "Cannot open file: {error}",
                error = error
            )));
        }
    }
    fn move_menu(&mut self, ui: &mut egui::Ui, job: &Job) {
        ui.add_enabled_ui(job.status != Status::Downloading, |ui| {
            ui.menu_button(tr("Move to"), |ui| {
                for queue in self.queues.clone() {
                    if queue.id != self.queue_id
                        && ui
                            .button(display_queue_name(&queue.id, &queue.name))
                            .clicked()
                    {
                        self.command(Command::MoveJob(job.id.clone(), queue.id));
                        ui.close();
                    }
                }
                if self.queues.len() == 1 {
                    ui.label(tr("Create another queue first."));
                }
                ui.separator();
                if ui.button(tr("Move up")).clicked() {
                    self.command(Command::ReorderJob(job.id.clone(), true));
                    ui.close();
                }
                if ui.button(tr("Move down")).clicked() {
                    self.command(Command::ReorderJob(job.id.clone(), false));
                    ui.close();
                }
                #[cfg(feature = "screenshot")]
                shell::capture_menu_contents(ui, tr("Move to"));
            });
        });
    }
    pub(super) fn file_menu(&mut self, ui: &mut egui::Ui, job: &Job) {
        if !self.is_selected(&job.id) {
            self.set_selection(Some(job.id.clone()));
        }
        let selected = self.selected_jobs();
        if selected.len() > 1 {
            ui.label(tf!("{} selected downloads", selected.len()));
            if ui.add_enabled(selected.iter().any(|j| matches!(j.status, Status::Queued | Status::Paused | Status::Failed)), egui::Button::new(tr("Start selected immediately")))
                .on_hover_text(tr("Start only these files ahead of the queue. Other files and schedules stay unchanged; the app's active-download limit and data quotas still apply."))
                .clicked() {
                for job in &selected {
                    if matches!(job.status, Status::Queued | Status::Paused | Status::Failed) {
                        self.command(Command::StartImmediately(job.id.clone()));
                    }
                }
                ui.close();
            }
            if ui
                .add_enabled(
                    selected
                        .iter()
                        .any(|j| matches!(j.status, Status::Paused | Status::Failed)),
                    egui::Button::new(tr("Resume selected")),
                )
                .clicked()
            {
                self.selection_command(false);
                ui.close();
            }
            if ui
                .add_enabled(
                    selected
                        .iter()
                        .any(|j| matches!(j.status, Status::Queued | Status::Downloading)),
                    egui::Button::new(tr("Pause selected")),
                )
                .clicked()
            {
                self.selection_command(true);
                ui.close();
            }
            if ui.add_enabled(
                selected.iter().any(|job| matches!(job.status, Status::Downloading | Status::Queued)),
                egui::Button::new(tr("Stop selected")),
            ).on_hover_text(tr("Stop the selected downloads and close their status windows. Saved progress is kept.")).clicked() {
                self.stop_selected_downloads();
                ui.close();
            }
            if ui
                .add_enabled(
                    selected.iter().all(|j| j.status != Status::Downloading),
                    egui::Button::new(tr("Remove selected")),
                )
                .clicked()
            {
                self.request_destructive(DestructiveAction::Remove, selected.clone());
                ui.close();
            }
            if ui
                .add_enabled(
                    selected.iter().all(|j| j.status == Status::Complete),
                    egui::Button::new(tr("Delete files…")),
                )
                .on_hover_text(tr(
                    "Permanently delete the selected completed files and remove them from history",
                ))
                .clicked()
            {
                self.request_destructive(DestructiveAction::DeleteFile, selected);
                ui.close();
            }
            return;
        }
        if ui.button(tr("Download status")).clicked() {
            self.open_download_status(job);
            ui.close();
        }
        if ui.button(tr("Open folder")).clicked() {
            self.show_folder(job);
            ui.close();
        }
        if matches!(job.status, Status::Queued | Status::Paused | Status::Failed)
            && ui.button(tr("Start immediately"))
                .on_hover_text(tr("Start this file ahead of the queue, even if paused or scheduled. Other files stay unchanged; the app's active-download limit and data quotas still apply."))
                .clicked() {
            self.command(Command::StartImmediately(job.id.clone()));
            self.expect_download_status(&job.id);
            ui.close();
        }
        if matches!(job.status, Status::Paused | Status::Failed)
            && ui.button(resume_action(job)).clicked()
        {
            self.command(Command::Resume(job.id.clone()));
            self.expect_download_status(&job.id);
            ui.close();
        }
        if matches!(job.status, Status::Downloading | Status::Queued)
            && ui.button(tr("Pause")).clicked()
        {
            self.command(Command::Pause(job.id.clone()));
            ui.close();
        }
        if matches!(job.status, Status::Downloading | Status::Queued)
            && ui
                .button(tr("Stop"))
                .on_hover_text(tr(
                    "Stop this download and close its status window. Saved progress is kept.",
                ))
                .clicked()
        {
            self.stop_selected_downloads();
            ui.close();
        }
        if job.status == Status::Complete && ui.button(tr("Redownload")).clicked() {
            self.duplicate_download(ui.ctx(), job);
            ui.close();
        }
        ui.separator();
        if let Some(state) = &job.synchronization {
            ui.label(
                state
                    .checked_at
                    .map(|at| tf!("Last check: {}", download_engine::queue::schedule_label(at)))
                    .unwrap_or_else(|| tr("Update check pending").into()),
            );
            ui.label(tr("Previous copy is kept:"));
            ui.add(egui::Label::new(state.previous.path.to_string_lossy()).truncate())
                .on_hover_text(state.previous.path.to_string_lossy());
            if ui.button(tr("Open previous copy's folder")).clicked() {
                let mut previous = job.clone();
                if let Some(parent) = state.previous.path.parent() {
                    previous.directory = parent.to_path_buf();
                }
                self.show_folder(&previous);
                ui.close();
            }
            ui.separator();
        }
        if job.status == Status::Complete && ui.button(tr("Antivirus scan / result")).clicked() {
            self.review_scan(job, false);
            ui.close();
        }
        if job.status == Status::Complete && ui.button(tr("SHA-256 checksum")).clicked() {
            self.open_checksum(job);
            ui.close();
        }
        if job.status == Status::Complete && ui.button(tr("Open file")).clicked() {
            self.open_file(job);
            ui.close();
        }
        if ui.button(tr("Copy link")).clicked() {
            ui.ctx().copy_text(job.url.clone());
            ui.close();
        }
        if job.ftp.is_none() && job.hls.is_none() && ui.button(tr("Preview ZIP contents")).clicked()
        {
            self.open_zip_job(job);
            ui.close();
        }
        if ui.button(tr("Duplicate")).clicked() {
            self.duplicate_download(ui.ctx(), job);
            ui.close();
        }
        if ui.button(tr("Speed limit")).clicked() {
            self.set_selection(Some(job.id.clone()));
            self.open_speed_limits();
            ui.close();
        }
        if matches!(job.status, Status::Paused | Status::Failed) {
            if job.ftp.is_none()
                && ui
                    .button(if job.hls.is_some() {
                        tr("Preview video again")
                    } else {
                        tr("Update download link")
                    })
                    .clicked()
            {
                self.open_link(job);
                ui.close();
            }
            if supports_authentication(job) && ui.button(tr("Authentication")).clicked() {
                self.open_authentication(job);
                ui.close();
            }
            if (job.plan.is_some() || job.hls.is_some() || job.ftp.is_some())
                && ui
                    .button(tr("Restart"))
                    .on_hover_text(if job.hls.is_some() {
                        tr("Discard saved segments and download the reviewed video again")
                    } else {
                        tr("Discard partial data and fetch the current version")
                    })
                    .clicked()
            {
                self.command(Command::Restart(job.id.clone()));
                ui.close();
            }
        }
        self.move_menu(ui, job);
        ui.separator();
        if job.status == Status::Complete
            && ui
                .button(tr("Delete file…"))
                .on_hover_text(tr(
                    "Permanently delete the saved file and remove it from history",
                ))
                .clicked()
        {
            self.request_destructive(DestructiveAction::DeleteFile, vec![job.clone()]);
            ui.close();
        }
        if ui
            .add_enabled(
                job.status != Status::Downloading,
                egui::Button::new(if job.status == Status::Complete {
                    tr("Clear from history")
                } else {
                    tr("Remove")
                }),
            )
            .on_hover_text(if job.status == Status::Complete {
                tr("Keep the saved file")
            } else {
                tr("Remove this entry and its partial data")
            })
            .clicked()
        {
            self.command(Command::Remove(job.id.clone()));
            ui.close();
        }
    }
    fn details(&mut self, ui: &mut egui::Ui, job: &Job, p: Palette) {
        let has_error = job.error.is_some();
        egui::Panel::bottom("file-details")
            .exact_size(if has_error { 206.0 } else { 162.0 })
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.surface)
                    .inner_margin(egui::Margin::symmetric(18, 12)),
            )
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 7.0;
                // A row as tall as File actions centers the icon, name and × on that button.
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), 28.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(18.0, 22.0), Sense::hover());
                        icon(ui, r.center(), FILE, 17.0, p.muted);
                        let name_width = (ui.available_width() - 165.0).max(100.0);
                        ui.allocate_ui_with_layout(
                            vec2(name_width, 22.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.add(
                                    egui::Label::new(RichText::new(&job.name).strong()).truncate(),
                                )
                            },
                        )
                        .inner
                        .on_hover_text(&job.name);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let close = close_button(ui, 24.0, tr("Hide details"));
                            close.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    close.enabled(),
                                    tr("Hide details"),
                                )
                            });
                            if close.clicked() {
                                let mut value = self.interface;
                                value.show_details = false;
                                self.save_interface(ui.ctx(), value);
                            }
                            self.file_menu_button(ui, tr("File actions"), job);
                        });
                    },
                );
                let width = (ui.available_width() - 26.0) / 2.0;
                let queue_name = self
                    .queues
                    .iter()
                    .find(|q| q.id == self.queue_id)
                    .map(|q| display_queue_name(&q.id, &q.name))
                    .unwrap_or(tr("Main queue"));
                ui.columns(2, |cols| {
                    for (col, values) in [
                        vec![
                            (
                                tr("Save to"),
                                job.saved_path
                                    .as_ref()
                                    .and_then(|p| p.parent())
                                    .unwrap_or(&job.directory)
                                    .to_string_lossy()
                                    .into_owned(),
                            ),
                            (tr("Source"), job.url.clone()),
                            (tr("Resume"), resume_description(job).into()),
                        ],
                        vec![
                            (
                                tr("Downloaded"),
                                job.total
                                    .map(|n| tf!("{} of {}", bytes(job.downloaded), bytes(n)))
                                    .unwrap_or_else(|| bytes(job.downloaded)),
                            ),
                            (tr("Queue"), queue_name.into()),
                            (
                                tr("Status"),
                                status_label(
                                    job,
                                    self.queues
                                        .iter()
                                        .any(|q| q.id == self.queue_id && q.running)
                                        || self.immediate_downloads.contains(&job.id),
                                    chrono::Utc::now().timestamp_millis(),
                                ),
                            ),
                        ],
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        egui::Grid::new(("file-metadata", col))
                            .spacing([12.0, 7.0])
                            .show(&mut cols[col], |ui| {
                                for (label, value) in values {
                                    ui.label(RichText::new(label).small().color(p.muted));
                                    ui.allocate_ui_with_layout(
                                        vec2(width - 85.0, 18.0),
                                        Layout::left_to_right(Align::Center),
                                        |ui| {
                                            ui.add(
                                                egui::Label::new(RichText::new(&value).small())
                                                    .truncate(),
                                            )
                                        },
                                    )
                                    .inner
                                    .on_hover_text(value);
                                    ui.end_row();
                                }
                            });
                    }
                });
                if job.status == Status::Complete {
                    ui.label(
                        RichText::new(self.antivirus_label(job))
                            .small()
                            .color(p.muted),
                    );
                }
                if let Some(error) = &job.error {
                    egui::ScrollArea::vertical()
                        .id_salt("download-error")
                        .max_height(60.0)
                        .show(ui, |ui| {
                            if let Some(description) = retry_description(
                                job,
                                self.queues
                                    .iter()
                                    .any(|q| q.id == self.queue_id && q.running)
                                    || self.immediate_downloads.contains(&job.id),
                            ) {
                                ui.label(RichText::new(description).small().color(p.accent));
                            }
                            ui.label(RichText::new(error).small().color(p.warning));
                        });
                }
            });
    }
}
