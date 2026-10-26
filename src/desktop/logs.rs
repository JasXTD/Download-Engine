//! Read-only text viewer. Bounded disk snapshots are read off the UI thread.
use super::*;
use download_engine::diagnostics::{self, Component, Event, Level, Record};
use egui::{Align, Layout, Stroke};
use std::{sync::mpsc, thread};

#[derive(Clone, Copy)]
enum SpanRole {
    Timestamp,
    Badge(Level),
    Separator,
    Component(Component),
    Event(Level),
    Details(Level),
    JobId,
}

struct Entry {
    record: Record,
    text: String,
    spans: Vec<(std::ops::Range<usize>, SpanRole)>,
    search: String,
}
impl Entry {
    fn new(record: Record) -> Self {
        let time = chrono::DateTime::parse_from_rfc3339(&record.time)
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%d %b %Y %H:%M:%S%.3f")
                    .to_string()
            })
            .unwrap_or_else(|_| record.time.clone());
        let mut text = String::new();
        let mut spans = Vec::new();
        let mut push = |part: &str, role: SpanRole| {
            if !part.is_empty() {
                let start = text.len();
                text.push_str(part);
                spans.push((start..text.len(), role));
            }
        };
        push(&format!("[{time}] "), SpanRole::Timestamp);
        push(
            &level_label(record.level).to_uppercase(),
            SpanRole::Badge(record.level),
        );
        push(" | ", SpanRole::Separator);
        push(
            &format!("{}:", source_label(record.component)),
            SpanRole::Component(record.component),
        );
        push(
            &format!(" {}", event_label(record.event)),
            SpanRole::Event(record.level),
        );
        let details = details_label(&record.details);
        if !details.is_empty() {
            push(&format!("\n  {details}"), SpanRole::Details(record.level));
        }
        if let Some(job) = record.details.job {
            push(&format!("\n  Download ID: {job}"), SpanRole::JobId);
        }
        let search = text.to_lowercase();
        Self {
            record,
            text,
            spans,
            search,
        }
    }
    fn matches(&self, state: &State, query: &str) -> bool {
        state.level.is_none_or(|level| self.record.level == level)
            && state
                .source
                .is_none_or(|source| self.record.component == source)
            && self.search.contains(query)
    }
}

#[derive(Default)]
struct Loaded {
    language: download_engine::i18n::Language,
    entries: Vec<Entry>,
    skipped: usize,
    older: usize,
}
impl From<diagnostics::Snapshot> for Loaded {
    fn from(snapshot: diagnostics::Snapshot) -> Self {
        Self {
            language: download_engine::i18n::current(),
            entries: snapshot.records.into_iter().map(Entry::new).collect(),
            skipped: snapshot.skipped,
            older: snapshot.older,
        }
    }
}

impl Loaded {
    fn relocalize(&mut self) -> bool {
        let language = download_engine::i18n::current();
        if self.language == language {
            return false;
        }
        self.entries = self
            .entries
            .drain(..)
            .map(|entry| Entry::new(entry.record))
            .collect();
        self.language = language;
        true
    }
}

pub(super) struct State {
    pub search: String,
    level: Option<Level>,
    source: Option<Component>,
    auto_refresh: bool,
    loaded: bool,
    data: Loaded,
    display: String,
    spans: Vec<(std::ops::Range<usize>, SpanRole)>,
    shown: usize,
    filter: Option<(String, Option<Level>, Option<Component>)>,
    scrolled_away: bool,
    error: Option<String>,
    checked: Option<Instant>,
    refresh_requested: bool,
    forced_pending: bool,
    pending: Option<mpsc::Receiver<anyhow::Result<Loaded>>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            search: String::new(),
            level: None,
            source: None,
            auto_refresh: true,
            loaded: false,
            data: Loaded::default(),
            display: String::new(),
            spans: Vec::new(),
            shown: 0,
            filter: None,
            scrolled_away: false,
            error: None,
            checked: None,
            refresh_requested: false,
            forced_pending: false,
            pending: None,
            worker: None,
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        // Reads have bounded sizes and lock waits. Release handles before the
        // store and any isolated test directory are dropped.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl State {
    #[cfg(feature = "screenshot")]
    pub fn loaded(&self) -> bool {
        self.loaded
    }

    pub fn request_refresh(&mut self) {
        // Keep explicit requests through an older snapshot that is finishing.
        self.refresh_requested = true;
    }
    fn reading(&self, ctx: &egui::Context) -> bool {
        self.scrolled_away || ctx.memory(|m| m.has_focus(egui::Id::new("logs-text")))
    }
    fn update_text(&mut self) {
        // A pending worker may have started before a language change. Rebuild
        // presentation from the original records, never translate saved logs.
        if self.data.relocalize() {
            self.filter = None;
        }
        let filter = (self.search.trim().to_lowercase(), self.level, self.source);
        if self.filter.as_ref() == Some(&filter) {
            return;
        }
        let entries: Vec<&Entry> = self
            .data
            .entries
            .iter()
            .filter(|entry| entry.matches(self, &filter.0))
            .collect();
        self.shown = entries.len();
        let mut display = String::new();
        let mut spans = Vec::new();
        for entry in entries {
            if !display.is_empty() {
                let sep = display.len();
                display.push_str("\n\n");
                spans.push((sep..display.len(), SpanRole::Separator));
            }
            let start = display.len();
            display.push_str(&entry.text);
            for (range, role) in &entry.spans {
                spans.push(((start + range.start)..(start + range.end), *role));
            }
        }
        self.display = display;
        self.spans = spans;
        self.filter = Some(filter);
    }
    pub fn keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        let search = egui::Id::new("logs-search");
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            ctx.memory_mut(|m| m.request_focus(search));
        }
        if ctx.memory(|m| m.has_focus(search) || m.had_focus_last_frame(search))
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            if self.search.is_empty() {
                ctx.memory_mut(|m| m.surrender_focus(search));
            } else {
                self.search.clear();
                ctx.memory_mut(|m| m.request_focus(search));
            }
        }
    }
    fn poll(&mut self) {
        let Some(receiver) = &self.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(anyhow::anyhow!(tr("The log reader stopped unexpectedly")))
            }
        };
        self.pending = None;
        self.forced_pending = false;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.loaded = true;
        self.checked = Some(Instant::now());
        match result {
            Ok(data) => {
                self.data = data;
                self.filter = None;
                self.error = None;
            }
            // Keep the last good snapshot and mark it stale. Logging every read
            // error here would feed the viewer with another event every refresh.
            Err(error) => {
                self.error = Some(tf!(
                    "Cannot read logs: {error}. Try Refresh.",
                    error = error
                ))
            }
        }
    }
    fn refresh(&mut self, paths: &download_engine::paths::AppPaths, ctx: &egui::Context) {
        // Never replace text under an active selection or while reading older
        // lines. Refresh is an explicit override; returning to the top resumes.
        let reading = self.reading(ctx);
        if reading && !self.refresh_requested && !self.forced_pending {
            return;
        }
        self.poll();
        if reading && !self.refresh_requested {
            return;
        }
        if self.pending.is_some()
            || (!self.refresh_requested
                && self.checked.is_some_and(|checked| {
                    !self.auto_refresh || checked.elapsed() < Duration::from_secs(2)
                }))
        {
            return;
        }
        self.checked = Some(Instant::now());
        self.forced_pending = self.refresh_requested;
        self.refresh_requested = false;
        let paths = paths.clone();
        let ctx = ctx.clone();
        let language = download_engine::i18n::current();
        let (sender, receiver) = mpsc::channel();
        match thread::Builder::new()
            .name("log-reader".into())
            .spawn(move || {
                let _language = download_engine::i18n::scope(language);
                let result = diagnostics::read_recent(&paths).map(Loaded::from);
                let _ = sender.send(result);
                ctx.request_repaint();
            }) {
            Ok(worker) => {
                self.worker = Some(worker);
                self.pending = Some(receiver);
            }
            Err(error) => {
                self.loaded = true;
                self.error = Some(tf!("Cannot start log reader: {error}", error = error));
            }
        }
    }
}

fn status_pill(ui: &mut egui::Ui, label: impl Into<String>, color: Color32, p: Palette) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.38)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(label.into()).size(11.5).strong().color(color));
        });
    let _ = p;
}

fn alert_banner(ui: &mut egui::Ui, message: &str, p: Palette) {
    egui::Frame::new()
        .fill(p.warning.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0, p.warning.gamma_multiply(0.42)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new("•").strong().color(p.warning));
                ui.label(RichText::new(message).small().color(p.warning));
            });
        });
}

impl Desktop {
    pub(super) fn logs_page(&mut self, ui: &mut egui::Ui, p: Palette) {
        self.logs.refresh(&self.store.paths, ui.ctx());
        self.poll_diagnostic_export();
        self.logs.update_text();
        let dark = ui.visuals().dark_mode;
        ui.horizontal(|ui| {
            ui.heading(RichText::new(tr("Logs")).size(18.5).strong());
            let total = self.logs.data.entries.len();
            let count = if self.logs.shown == total {
                tf!("{total} events", total = total)
            } else {
                tf!("{} / {total} events", self.logs.shown, total = total)
            };
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(8, 3))
                .show(ui, |ui| {
                    ui.label(RichText::new(count).size(11.5).strong().color(p.muted));
                });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !self.diagnostics.busy(),
                        if self.diagnostics.busy() {
                            egui::Button::new(tr("Exporting…"))
                        } else {
                            primary(tr("Export diagnostics…"), p)
                        },
                    )
                    .on_hover_text(tr(
                        "Save all retained desktop and connector events to a local report",
                    ))
                    .clicked()
                {
                    self.export_diagnostics();
                }
                if ui
                    .button(tr("Open log folder"))
                    .on_hover_text(tr("Open the log files in File Explorer"))
                    .clicked()
                {
                    self.open_diagnostic_logs();
                }
                if ui
                    .add_enabled(
                        self.logs.pending.is_none(),
                        egui::Button::new(tr("Refresh")),
                    )
                    .clicked()
                {
                    self.logs.request_refresh();
                }
                if ui
                    .add_enabled(
                        !self.logs.display.is_empty(),
                        egui::Button::new(tr("Copy logs")),
                    )
                    .on_hover_text(tr("Copy the displayed log text"))
                    .clicked()
                {
                    ui.ctx().copy_text(self.logs.display.clone());
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(tr("Local desktop and browser connector diagnostics."))
                    .small()
                    .color(p.muted),
            );
            if !self.logs.data.entries.is_empty() {
                let errors = self
                    .logs
                    .data
                    .entries
                    .iter()
                    .filter(|e| e.record.level == Level::Error)
                    .count();
                let warnings = self
                    .logs
                    .data
                    .entries
                    .iter()
                    .filter(|e| e.record.level == Level::Warn)
                    .count();
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if errors > 0 {
                        status_pill(
                            ui,
                            format!(
                                "{errors} {}",
                                tr(if errors == 1 { "error" } else { "errors" })
                            ),
                            level_color(Level::Error, dark),
                            p,
                        );
                    }
                    if warnings > 0 {
                        status_pill(
                            ui,
                            format!(
                                "{warnings} {}",
                                tr(if warnings == 1 { "warning" } else { "warnings" })
                            ),
                            level_color(Level::Warn, dark),
                            p,
                        );
                    }
                    if errors == 0 && warnings == 0 {
                        status_pill(ui, tr("All healthy"), p.success, p);
                    }
                });
            }
        });
        ui.add_space(8.0);
        self.notice(ui, p);
        if !diagnostics::available() {
            alert_banner(
                ui,
                tr("Logging unavailable. Existing events are still shown."),
                p,
            );
            ui.add_space(6.0);
        }
        if let Some(error) = &self.logs.error {
            alert_banner(ui, error, p);
            ui.add_space(6.0);
        }
        egui::Frame::new()
            .fill(p.background)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(8)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                self.logs.controls(ui, p);
                egui::Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        self.logs.console(ui, p);
                    });
            });
    }
}

impl State {
    fn controls(&mut self, ui: &mut egui::Ui, p: Palette) {
        ui.set_width(ui.available_width());
        let height = view::CONTROL_HEIGHT;
        ui.spacing_mut().interact_size.y = height;
        ui.horizontal(|ui| {
            let auto_width = ui.fonts_mut(|fonts| {
                fonts.layout_no_wrap(tr("Auto-refresh").into(), egui::TextStyle::Body.resolve(ui.style()), p.text).size().x
            }) + ui.spacing().icon_width + ui.spacing().icon_spacing;
            let width = (ui.available_width() - 110.0 - 160.0 - auto_width - 16.0
                - 4.0 * ui.spacing().item_spacing.x).max(100.0);
            ui.add_sized(
                [width, height],
                view::line_edit(&mut self.search)
                    .id(egui::Id::new("logs-search"))
                    .margin(egui::vec2(10.0, 4.0))
                    .vertical_align(Align::Center)
                    .hint_text(tr("Search logs…")),
            );
            egui::ComboBox::from_id_salt("logs-level")
                .width(110.0)
                .selected_text(self.level.map(level_label).unwrap_or(tr("All levels")))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.level, None, tr("All levels"));
                    for level in [Level::Error, Level::Warn, Level::Info] {
                        ui.selectable_value(&mut self.level, Some(level), level_label(level));
                    }
                })
                .response
                .on_hover_text(tr("Filter by severity"));
            egui::ComboBox::from_id_salt("logs-source")
                .width(160.0)
                .selected_text(self.source.map(source_label).unwrap_or(tr("All sources")))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.source, None, tr("All sources"));
                    for source in [Component::Desktop, Component::NativeHost] {
                        ui.selectable_value(&mut self.source, Some(source), source_label(source));
                    }
                })
                .response
                .on_hover_text(tr("Filter by source"));
            ui.square_checkbox(&mut self.auto_refresh, tr("Auto-refresh"))
                .on_hover_text(tr("Pauses while text is focused or scrolled, so you can read and copy without changes"));
            // Reserve the status slot so background reads never shift the console.
            let (rect, response) = ui.allocate_exact_size(egui::vec2(16.0, height), egui::Sense::hover());
            if self.reading(ui.ctx()) && self.auto_refresh {
                view::icon(ui, rect.center(), "\u{e769}", 14.0, p.muted);
                response.on_hover_text(tr("Auto-refresh paused for reading"));
            } else if self.pending.is_some() {
                egui::Spinner::new().size(14.0).paint_at(ui, rect);
                response.on_hover_text(tr("Refreshing logs…"));
            }
        });
        self.update_text();
        if self.data.older > 0 || self.data.skipped > 0 {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if self.data.older > 0 {
                    egui::Frame::new()
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, p.border))
                        .corner_radius(5)
                        .inner_margin(egui::Margin::symmetric(8, 2))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(tf!(
                                    "{} older events available in export.",
                                    self.data.older
                                ))
                                .small()
                                .color(p.muted),
                            );
                        });
                }
                if self.data.skipped > 0 {
                    egui::Frame::new()
                        .fill(p.warning.gamma_multiply(0.10))
                        .stroke(Stroke::new(1.0, p.warning.gamma_multiply(0.38)))
                        .corner_radius(5)
                        .inner_margin(egui::Margin::symmetric(8, 2))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(tf!(
                                    "{} invalid entries skipped.",
                                    self.data.skipped
                                ))
                                .small()
                                .color(p.warning),
                            );
                        });
                }
            });
        }
    }

    fn console(&mut self, ui: &mut egui::Ui, p: Palette) {
        let height = ui.available_height().max(80.0);
        let output = egui::ScrollArea::vertical()
            .id_salt("log-text-scroll")
            .auto_shrink([false, false])
            .max_height(height)
            .min_scrolled_height(height)
            .show(ui, |ui| {
                if self.display.is_empty() {
                    ui.add_space((height * 0.22).clamp(24.0, 72.0));
                    ui.vertical_centered(|ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        let (badge, _) =
                            ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
                        ui.painter().rect_filled(badge, 18, p.track);
                        view::icon(
                            ui,
                            badge.center(),
                            if self.error.is_some() {
                                "\u{e783}"
                            } else {
                                "\u{e7c3}"
                            },
                            16.0,
                            if self.error.is_some() {
                                p.warning
                            } else {
                                p.blue
                            },
                        );
                        ui.label(
                            RichText::new(if !self.loaded {
                                tr("Loading logs…")
                            } else if self.error.is_some() {
                                tr("Logs could not be loaded")
                            } else if self.data.entries.is_empty() {
                                tr("No log entries yet")
                            } else {
                                tr("No events match your filters")
                            })
                            .strong(),
                        );
                        if self.level.is_some()
                            || self.source.is_some()
                            || !self.search.trim().is_empty()
                        {
                            if ui.button(tr("Clear filters")).clicked() {
                                self.search.clear();
                                self.level = None;
                                self.source = None;
                            }
                        } else if self.error.is_none() {
                            ui.label(
                                RichText::new(if self.auto_refresh {
                                    tr("New activity will appear here automatically.")
                                } else {
                                    tr("Use Refresh to load new activity.")
                                })
                                .small()
                                .color(p.muted),
                            );
                        }
                    });
                } else {
                    // An immutable TextBuffer supports mouse/keyboard selection
                    // and copying while rejecting all edits to the displayed log.
                    let mut text = self.display.as_str();
                    let spans = &self.spans;
                    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, width: f32| {
                        let dark = ui.visuals().dark_mode;
                        let mut job = egui::text::LayoutJob {
                            text: text.as_str().to_owned(),
                            ..Default::default()
                        };
                        job.wrap.max_width = width;
                        job.sections = spans
                            .iter()
                            .map(|(range, role)| egui::text::LayoutSection {
                                leading_space: 0.0,
                                byte_range: egui::text::ByteIndex(range.start)
                                    ..egui::text::ByteIndex(range.end),
                                format: span_format(*role, dark),
                            })
                            .collect();
                        ui.fonts_mut(|fonts| fonts.layout_job(job))
                    };
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .id(egui::Id::new("logs-text"))
                            .font(egui::FontId::monospace(12.0))
                            .layouter(&mut layouter)
                            .desired_width(ui.available_width())
                            .desired_rows(1)
                            .frame(egui::Frame::new()),
                    );
                }
            });
        self.scrolled_away = output.state.offset.y > 0.5;
    }
}

fn span_format(role: SpanRole, dark: bool) -> egui::TextFormat {
    let p = Palette::new(dark);
    let font_id = egui::FontId::monospace(12.0);
    match role {
        SpanRole::Timestamp => egui::TextFormat {
            font_id,
            color: p.muted,
            ..Default::default()
        },
        SpanRole::Badge(level) => {
            let color = level_color(level, dark);
            egui::TextFormat {
                font_id,
                color,
                background: color.gamma_multiply(if dark { 0.16 } else { 0.12 }),
                ..Default::default()
            }
        }
        SpanRole::Separator => egui::TextFormat {
            font_id,
            color: p.muted.gamma_multiply(0.65),
            ..Default::default()
        },
        SpanRole::Component(component) => egui::TextFormat {
            font_id,
            color: match component {
                Component::Desktop => p.blue,
                Component::NativeHost => p.accent,
            },
            ..Default::default()
        },
        SpanRole::Event(level) => egui::TextFormat {
            font_id,
            color: match level {
                Level::Info => p.text,
                Level::Warn | Level::Error => level_color(level, dark),
            },
            ..Default::default()
        },
        SpanRole::Details(level) => egui::TextFormat {
            font_id,
            color: match level {
                Level::Error => level_color(Level::Error, dark).gamma_multiply(0.9),
                Level::Warn => level_color(Level::Warn, dark).gamma_multiply(0.9),
                Level::Info => p.muted,
            },
            ..Default::default()
        },
        SpanRole::JobId => egui::TextFormat {
            font_id,
            color: p.muted,
            ..Default::default()
        },
    }
}

pub(super) fn level_color(level: Level, dark: bool) -> Color32 {
    match (level, dark) {
        (Level::Error, true) => Color32::from_rgb(255, 107, 107),
        (Level::Warn, true) => Palette::new(true).orange,
        (Level::Error, false) => Color32::from_rgb(180, 40, 45),
        (Level::Warn, false) => Palette::new(false).orange,
        (Level::Info, _) => Palette::new(dark).blue,
    }
}
fn level_label(level: Level) -> &'static str {
    match level {
        Level::Info => tr("Info"),
        Level::Warn => tr("Warning"),
        Level::Error => tr("Error"),
    }
}
fn source_label(source: Component) -> &'static str {
    match source {
        Component::Desktop => tr("Desktop"),
        Component::NativeHost => tr("Browser connector"),
    }
}
fn event_label(event: Event) -> &'static str {
    match event {
        Event::Started => tr("Application started"),
        Event::Ready => tr("Ready"),
        Event::Stopped => tr("Application stopped"),
        Event::StartupFailed => tr("Startup failed"),
        Event::Panic => tr("Unexpected application error"),
        Event::DownloadStarted => tr("Download started"),
        Event::DownloadCompleted => tr("Download completed"),
        Event::DownloadPaused => tr("Download paused"),
        Event::DownloadRetryScheduled => tr("Download retry scheduled"),
        Event::DownloadQuotaHeld => tr("Download waiting for quota"),
        Event::DownloadFailed => tr("Download failed"),
        Event::DownloadStateSaveFailed => tr("Could not save download state"),
        Event::WorkerFailed => tr("Download worker failed"),
        Event::SchedulerFailed => tr("Scheduler failed"),
        Event::UiError => tr("Interface error"),
        Event::McpStarted => tr("MCP server started"),
        Event::McpStopped => tr("MCP server stopped"),
        Event::McpFailed => tr("MCP server failed"),
        Event::VideoLookupStarted => tr("Video lookup started"),
        Event::VideoLookupSucceeded => tr("Video lookup completed"),
        Event::VideoLookupFailed => tr("Video lookup failed"),
        Event::BrowserRegistered => tr("Browser connected"),
        Event::BrowserUnregistered => tr("Browser disconnected"),
        Event::BrowserSetupFailed => tr("Browser setup failed"),
        Event::NativeRequestReceived => tr("Browser request received"),
        Event::NativeRequestSucceeded => tr("Browser request completed"),
        Event::NativeRequestFailed => tr("Browser request failed"),
        Event::NativeOperationFailed => tr("Browser operation failed"),
        Event::NativeHostFailed => tr("Browser connector failed"),
        Event::NativeMessageInvalid => tr("Invalid browser message"),
        Event::DesktopLaunchFailed => tr("Could not open Download Engine"),
        Event::DiagnosticsExported => tr("Diagnostics exported"),
        Event::EntriesDropped => tr("Log buffer full; events skipped"),
    }
}
fn details_label(details: &diagnostics::Details) -> String {
    let mut values = Vec::new();
    if let Some(failure) = &details.failure {
        use diagnostics::FailureKind::*;
        values.push(
            match failure.kind {
                Application => tr("Application error"),
                Io => tr("File access error"),
                NotFound => tr("File not found"),
                PermissionDenied => tr("Permission denied"),
                DiskFull => tr("Disk full"),
                Database => tr("Database error"),
                Network => tr("Network error"),
                Timeout => tr("Timed out"),
                Authentication => tr("Authentication required"),
                Quota => tr("Quota reached"),
                Cancelled => tr("Cancelled"),
                Unsupported => tr("Unsupported"),
                Validation => tr("Invalid input"),
            }
            .to_owned(),
        );
        if let Some(code) = failure.code {
            values.push(tf!("Error code: {code}", code = code));
        }
        if let Some(status) = failure.http_status {
            values.push(format!("HTTP {status}"));
        }
    }
    if let Some(action) = details.action {
        values.push(tf!("Action: {action:?}", action = action));
    }
    if let Some(request) = details.request {
        values.push(tf!("Request: {request}", request = request));
    }
    if let Some(elapsed) = details.elapsed_ms {
        values.push(tf!("Duration: {elapsed} ms", elapsed = elapsed));
    }
    if let Some(size) = details.bytes {
        values.push(tf!("Transferred: {}", bytes(size)));
    }
    if let Some(attempt) = details.attempt {
        values.push(tf!("Attempt: {attempt}", attempt = attempt));
    }
    if let Some(dropped) = details.dropped {
        values.push(tf!("Skipped: {dropped}", dropped = dropped));
    }
    values.join("  ·  ")
}

#[cfg(test)]
mod tests;
