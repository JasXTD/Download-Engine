//! Inline Lua/PowerShell editor and persistent, selectable plain-text output.
use super::*;
use download_engine::script_automation::{
    FAILED_EXAMPLE, LUA_EXAMPLE, Language, Log, MAX_SCRIPT_BYTES, READER_EXAMPLE, Reading, Run,
    SampleCandidate, Script, StoredValue, Trigger,
};
mod editor;
mod file;
mod form;
mod sample;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct State {
    items: Vec<Script>,
    draft: Option<Script>,
    baseline: Option<Script>,
    runs: Vec<Run>,
    selected_run: Option<String>,
    pinned_run: bool,
    output: Vec<Log>,
    error: Option<String>,
    load_error: Option<String>,
    stopping: Option<String>,
    all_history: bool,
    pages: form::PageInput,
    /// The page a site reader is tested on, its latest test run and what it found.
    test_url: String,
    test_run: Option<String>,
    test_found: Option<(String, Reading)>,
    /// What the open script keeps between runs.
    values: form::Values,
    /// The Run with sample picker, while it is open.
    sample: Option<sample::Picker>,
    /// A file's code waiting for the person to confirm replacing unsaved code.
    replace: Option<file::Loaded>,
    editor: editor::State,
}
impl State {
    #[cfg(feature = "screenshot")]
    pub(super) fn capture_ready(&self) -> bool {
        self.draft.is_none() || self.editor.capture_ready()
    }
    pub(super) fn guide(&self) -> (&'static str, &'static str) {
        match self.draft.as_ref().map(|script| script.language) {
            Some(Language::PowerShell) => (tr("Script guide"), "Scripting"),
            _ => (tr("Lua guide"), "Lua scripting"),
        }
    }

    pub(super) fn dirty(&self) -> bool {
        self.draft != self.baseline
    }
    pub(super) fn new_script(&mut self) {
        self.draft = Some(Script {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            name: tr("New script").into(),
            language: Language::Lua,
            body: Language::Lua.example().into(),
            enabled: false,
            trigger: Trigger::Manual,
            timeout_seconds: 300,
            change_files: false,
        });
        self.baseline = None;
        self.error = None;
        self.selected_run = None;
        self.pinned_run = false;
        self.output.clear();
        self.all_history = false;
    }
    fn select(&mut self, script: Script) {
        let other = self.draft.as_ref().is_none_or(|d| d.id != script.id);
        self.baseline = Some(script.clone());
        self.draft = Some(script);
        self.selected_run = None;
        self.pinned_run = false;
        self.error = None;
        self.all_history = false;
        if other {
            self.pages = form::PageInput::default();
            self.test_run = None;
            self.test_found = None;
            self.values = form::Values::default();
        }
    }
    /// Show one run's log, whichever script is open; a clicked notification asks for it.
    pub(super) fn show_run(&mut self, run: String) {
        self.all_history = true;
        self.selected_run = Some(run);
        self.pinned_run = true;
    }
    fn reader(&self) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|s| matches!(s.trigger, Trigger::SiteReader { .. }))
    }
    fn visible_runs(&self) -> Vec<Run> {
        self.runs
            .iter()
            .filter(|r| {
                self.all_history || self.draft.as_ref().is_some_and(|s| s.id == r.script_id)
            })
            .cloned()
            .collect()
    }
}
pub(super) fn refresh(store: &Store, state: &mut State) {
    let result = (|| -> anyhow::Result<_> { Ok((store.scripts()?, store.script_runs(None)?)) })();
    match result {
        Ok((items, runs)) => {
            state.items = items;
            state.runs = runs;
            state.load_error = None;
            if state.draft.is_none()
                && let Some(first) = state.items.first().cloned()
            {
                state.select(first);
            }
            // Never replace an unsaved buffer during live polling.
            if !state.dirty()
                && let Some(draft) = &state.draft
                && let Some(latest) = state.items.iter().find(|s| s.id == draft.id)
            {
                state.draft = Some(latest.clone());
                state.baseline = Some(latest.clone());
            }
            let visible = state.visible_runs();
            if !state.pinned_run
                || !visible
                    .iter()
                    .any(|r| Some(&r.id) == state.selected_run.as_ref())
            {
                state.selected_run = visible.first().map(|r| r.id.clone());
            }
            state.values.list = Vec::new();
            if let Some(draft) = state.draft.as_ref().filter(|d| d.revision > 0) {
                match store.script_values(&draft.id) {
                    Ok(values) => state.values.list = values,
                    Err(e) => state.load_error = Some(e.to_string()),
                }
            }
            state.output = Vec::new();
            if let Some(id) = &state.selected_run {
                match store.script_logs(id) {
                    Ok(logs) => state.output = logs,
                    Err(e) => state.load_error = Some(e.to_string()),
                }
            }
            if state
                .stopping
                .as_ref()
                .is_some_and(|id| !state.runs.iter().any(|r| r.id == *id && r.active()))
            {
                state.stopping = None;
            }
            // Load what a finished test found, once.
            if let Some(id) = state.test_run.clone()
                && state
                    .test_found
                    .as_ref()
                    .is_none_or(|(found, _)| *found != id)
                && state
                    .runs
                    .iter()
                    .any(|r| r.id == id && r.state == "succeeded")
            {
                match store.script_result(&id) {
                    Ok(Some(found)) => state.test_found = Some((id, found)),
                    Ok(None) => {}
                    Err(e) => state.load_error = Some(e.to_string()),
                }
            }
        }
        Err(error) => state.load_error = Some(error.to_string()),
    }
}
fn trigger_name(trigger: &Trigger) -> &'static str {
    match trigger {
        Trigger::Manual => tr("Manual only"),
        Trigger::Startup => tr("App startup"),
        Trigger::Interval { .. } => tr("Every interval"),
        Trigger::DownloadCompleted { .. } => tr("Download completed"),
        Trigger::DownloadFailed { .. } => tr("Download failed"),
        Trigger::QueueCompleted { .. } => tr("Queue completed"),
        Trigger::SiteReader { .. } => tr("Site reader"),
    }
}
fn time(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%d %b %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| at.to_string())
}
fn log_text(logs: &[Log]) -> String {
    logs.iter().map(|l| l.text.as_str()).collect()
}
fn output_layout(output: &[Log], dark: bool) -> egui::text::LayoutJob {
    // Pipes split text at arbitrary byte boundaries. Reassemble neighboring
    // chunks so wrapping and warning colours do not invent line breaks.
    let mut blocks: Vec<(&str, String)> = Vec::new();
    for log in output {
        if let Some((level, text)) = blocks.last_mut()
            && *level == log.level
        {
            text.push_str(&log.text);
        } else {
            blocks.push((&log.level, log.text.clone()));
        }
    }
    let mut layout = egui::text::LayoutJob::default();
    for (level, text) in blocks {
        for line in text.split_inclusive('\n') {
            let severity = if level == "error" {
                download_engine::diagnostics::Level::Error
            } else if level == "warn" || line.trim_start().starts_with("WARNING:") {
                download_engine::diagnostics::Level::Warn
            } else {
                download_engine::diagnostics::Level::Info
            };
            // What a sample run would have done stands apart from the script's own output.
            let simulated = severity == download_engine::diagnostics::Level::Info
                && line.trim_end().ends_with("(sample run, nothing changed)");
            layout.append(
                line,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::monospace(12.0),
                    color: if simulated {
                        Palette::new(dark).muted
                    } else {
                        logs::level_color(severity, dark)
                    },
                    ..Default::default()
                },
            );
        }
    }
    layout
}

/// Run the saved site reader on the page in the test field.
fn test_reader(store: &Store, state: &mut State) {
    let Some(draft) = &state.draft else { return };
    match store.run_site_reader(
        &draft.id,
        state.test_url.trim(),
        &uuid::Uuid::new_v4().to_string(),
    ) {
        Ok(run) => {
            state.test_run = Some(run.id.clone());
            state.test_found = None;
            state.selected_run = Some(run.id);
            state.pinned_run = false;
            state.all_history = false;
            state.error = None;
            refresh(store, state);
        }
        Err(e) => state.error = Some(format!("{e:#}")),
    }
}

/// A found file's type, from its name or address: "MP4", "WEBM", or "FILE".
fn file_kind(item: &download_engine::script_automation::ReadingItem) -> String {
    let path = item.filename.clone().unwrap_or_else(|| {
        reqwest::Url::parse(&item.url).map_or_else(|_| item.url.clone(), |u| u.path().to_owned())
    });
    std::path::Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| (1..=5).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map_or_else(|| "FILE".into(), str::to_ascii_uppercase)
}

/// The test field, its Test button and what the latest test found.
fn test_card(ui: &mut egui::Ui, store: &Store, state: &mut State, p: Palette) {
    let dirty = state.dirty();
    let saved = state.draft.as_ref().is_some_and(|d| d.revision > 0);
    let run = state
        .test_run
        .as_ref()
        .and_then(|id| state.runs.iter().find(|r| &r.id == id))
        .cloned();
    let busy = state
        .draft
        .as_ref()
        .is_some_and(|d| state.runs.iter().any(|r| r.script_id == d.id && r.active()));
    let mut test = false;
    egui::Frame::new()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(10)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("Test on URL")).strong());
                ui.label(
                    RichText::new(tr("Runs the saved script on a page. Nothing is downloaded."))
                        .small()
                        .color(p.muted),
                );
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let response = ui.add(
                    view::line_edit(&mut state.test_url)
                        .id(egui::Id::new("site-reader-test-url"))
                        .hint_text("https://www.example.com/watch/…")
                        .font(egui::TextStyle::Monospace)
                        .desired_width((ui.available_width() - 90.0).max(160.0))
                        .char_limit(4096),
                );
                let entered = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let ready = saved && !dirty && !busy && !state.test_url.trim().is_empty();
                let button = ui
                    .add_enabled(ready, primary(tr("Test"), p))
                    .on_disabled_hover_text(if dirty || !saved {
                        tr("Save changes before testing.")
                    } else {
                        tr("Runs the saved script on a page. Nothing is downloaded.")
                    });
                test = ready && (button.clicked() || entered);
            });
            let Some(run) = run else { return };
            ui.add_space(6.0);
            if run.active() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr("Reading the page…"));
                });
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            } else if run.state == "succeeded" {
                let Some((_, found)) = state.test_found.as_ref().filter(|(id, _)| *id == run.id)
                else {
                    return;
                };
                if found.items.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        script_badge(ui, &tr("Nothing found").to_uppercase(), p.orange);
                        ui.label(tr("The script finished without returning any videos."));
                    });
                    return;
                }
                ui.horizontal_wrapped(|ui| {
                    script_badge(
                        ui,
                        &tf!("Found {count}", count = found.items.len()).to_uppercase(),
                        p.accent,
                    );
                    if let Some(title) = &found.title {
                        ui.label(RichText::new(title).strong());
                    }
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("site-reader-test-results")
                    .max_height(180.0)
                    .show(ui, |ui| {
                        egui::Grid::new("site-reader-test-grid")
                            .num_columns(4)
                            .striped(true)
                            .spacing(egui::vec2(14.0, 6.0))
                            .show(ui, |ui| {
                                for header in [tr("Kind"), tr("Label"), tr("Address"), tr("Size")] {
                                    ui.label(RichText::new(header).small().color(p.muted));
                                }
                                ui.end_row();
                                for item in &found.items {
                                    let (kind, color) = match item.kind {
                                        download_engine::browser_video::Kind::Hls => ("HLS".into(), p.accent),
                                        download_engine::browser_video::Kind::Dash => ("DASH".into(), p.accent),
                                        download_engine::browser_video::Kind::File => (file_kind(item), p.blue),
                                    };
                                    ui.label(RichText::new(kind).strong().color(color));
                                    ui.label(item.label.as_deref().unwrap_or("—"));
                                    ui.add_sized(
                                        [(ui.available_width() * 0.6).clamp(160.0, 420.0), 18.0],
                                        egui::Label::new(
                                            RichText::new(&item.url).monospace().small().color(p.muted),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&item.url);
                                    ui.label(item.size.map_or("—".into(), |size| {
                                        format!("{:.1} MB", size as f64 / 1_048_576.0)
                                    }));
                                    ui.end_row();
                                }
                            });
                    });
                ui.label(
                    RichText::new(tr("Opening this page shows these in Video downloads. One stream and no files goes straight to the quality picker."))
                        .small()
                        .color(p.muted),
                );
            } else {
                // The script's own error is in the log; its first line is the message.
                let detail = state
                    .output
                    .iter()
                    .filter(|log| log.level == "error")
                    .map(|log| log.text.as_str())
                    .collect::<String>();
                let message = detail
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .map(str::to_owned)
                    .or(run.error.clone())
                    .unwrap_or_else(|| run.state.clone());
                ui.colored_label(p.warning, tf!("Test failed: {error}", error = message));
            }
        });
    if test {
        test_reader(store, state);
    }
}

fn script_badge(ui: &mut egui::Ui, label: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.14))
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.38)))
        .corner_radius(5)
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(11.0).strong().color(color));
        });
}

fn controls(ui: &mut egui::Ui, store: &Store, state: &mut State, p: Palette) {
    if state.draft.is_none() && state.items.is_empty() {
        return;
    }
    let dirty = state.dirty();
    let mut chosen = None;
    egui::Frame::new()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().interact_size.y = form::CONTROL_HEIGHT;
            if let Some(draft) = state.draft.clone() {
                let active = state
                    .runs
                    .iter()
                    .find(|r| r.script_id == draft.id && r.active())
                    .cloned();
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(dirty, primary(tr("Save script"), p)).clicked() {
                        match store.save_script(&draft) {
                            Ok(saved) => {
                                state.select(saved);
                                refresh(store, state);
                            }
                            Err(e) => state.error = Some(e.to_string()),
                        }
                    }
                    let reader = matches!(draft.trigger, Trigger::SiteReader { .. });
                    if reader {
                        if ui
                            .add_enabled(
                                !dirty
                                    && draft.revision > 0
                                    && active.is_none()
                                    && !state.test_url.trim().is_empty(),
                                egui::Button::new(tr("Test on URL")),
                            )
                            .on_hover_text(tr("Runs the saved script on a page. Nothing is downloaded."))
                            .clicked()
                        {
                            test_reader(store, state);
                        }
                    } else if ui
                        .add_enabled(
                            !dirty && draft.revision > 0 && active.is_none(),
                            egui::Button::new(tr("Run script")),
                        )
                        .on_hover_text(tr("Save changes before running. Manual runs work even with automatic triggers disabled."))
                        .clicked()
                    {
                        match store.run_script(&draft.id, &uuid::Uuid::new_v4().to_string()) {
                            Ok(run) => {
                                state.selected_run = Some(run.id);
                                state.pinned_run = false;
                                state.all_history = false;
                                state.error = None;
                                refresh(store, state);
                            }
                            Err(e) => state.error = Some(e.to_string()),
                        }
                    }
                    if !reader
                        && draft.takes_sample()
                        && ui
                            .add_enabled(
                                !dirty && draft.revision > 0 && active.is_none(),
                                egui::Button::new(tr("Run with sample…")),
                            )
                            .on_hover_text(tr("Runs the saved script with downloads you choose. Changes are only simulated."))
                            .on_disabled_hover_text(tr("Save changes before running."))
                            .clicked()
                    {
                        match sample::Picker::open(store, &draft) {
                            Ok(picker) => {
                                state.sample = Some(picker);
                                state.error = None;
                            }
                            Err(e) => state.error = Some(e.to_string()),
                        }
                    }
                    if ui
                        .add_enabled(
                            active.is_some() && state.stopping.is_none(),
                            egui::Button::new(if state.stopping.is_some() {
                                tr("Stopping…")
                            } else {
                                tr("Stop script")
                            }),
                        )
                        .clicked()
                        && let Some(run) = active.clone()
                    {
                        match store.stop_script(&run.id) {
                            Ok(()) => {
                                state.stopping = Some(run.id);
                                refresh(store, state);
                            }
                            Err(e) => state.error = Some(e.to_string()),
                        }
                    }
                    if ui.button(tr("Reload / discard edits")).clicked() {
                        match store.script(&draft.id) {
                            Ok(saved) => state.select(saved),
                            Err(e) if draft.revision > 0 => state.error = Some(e.to_string()),
                            Err(_) => {
                                state.draft = None;
                                state.baseline = None;
                                state.error = None;
                            }
                        }
                        refresh(store, state);
                    }
                    if ui
                        .add_enabled(
                            !dirty
                                && draft.revision > 0
                                && !state.runs.iter().any(|r| r.script_id == draft.id && r.active()),
                            egui::Button::new(tr("Remove script")),
                        )
                        .on_hover_text(tr("Removes the saved script. Run logs stay in All scripts history."))
                        .clicked()
                    {
                        match store.remove_script(&draft.id) {
                            Ok(()) => {
                                state.draft = None;
                                state.baseline = None;
                                state.error = None;
                                state.all_history = true;
                                refresh(store, state);
                            }
                            Err(e) => state.error = Some(e.to_string()),
                        }
                    }
                    if dirty {
                        script_badge(ui, tr("Unsaved changes"), p.warning);
                    } else if active.is_some() {
                        script_badge(ui, tr("Running"), p.accent);
                    }
                    if reader {
                        script_badge(ui, &tr("Site reader").to_uppercase(), p.blue);
                    }
                });
            }
            if !state.items.is_empty() {
                if state.draft.is_some() {
                    ui.add_space(6.0);
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tr("Saved script")).small().color(p.muted));
                    ui.add_enabled_ui(!dirty, |ui| {
                        ui.set_max_width(240.0);
                        egui::ComboBox::from_id_salt("saved-scripts")
                            .width(240.0)
                            .truncate()
                            .selected_text(
                                state
                                    .draft
                                    .as_ref()
                                    .map_or(tr("Choose a script"), |s| s.name.as_str()),
                            )
                            .show_ui(ui, |ui| {
                                for item in &state.items {
                                    if ui
                                        .selectable_label(
                                            state.draft.as_ref().is_some_and(|s| s.id == item.id),
                                            &item.name,
                                        )
                                        .clicked()
                                    {
                                        chosen = Some(item.clone());
                                    }
                                }
                            })
                            .response
                            .on_hover_text(state.draft.as_ref().map_or(tr("Choose a script"), |s| s.name.as_str()));
                    });
                });
            }
        });
    if let Some(item) = chosen {
        state.select(item);
        refresh(store, state);
    }
    ui.add_space(8.0);
}

impl Desktop {
    pub(super) fn scripts_page(&mut self, ui: &mut egui::Ui, p: Palette) {
        let mut state = std::mem::take(&mut self.scripts);
        if let Some(error) = &state.load_error {
            ui.colored_label(p.warning, error);
        }
        if let Some(error) = &state.error {
            ui.colored_label(p.warning, error);
        }
        controls(ui, &self.store, &mut state, p);
        #[cfg(feature = "screenshot")]
        {
            static SAMPLE_OPENED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            match std::env::var("IDM_EX_TEST_VIEW").as_deref() {
                Ok("automation-scripts-values") => state.values.shown = true,
                Ok("automation-scripts-sample")
                    if !SAMPLE_OPENED.swap(true, std::sync::atomic::Ordering::Relaxed)
                        && let Some(draft) = &state.draft =>
                {
                    state.sample = sample::Picker::open(&self.store, draft).ok();
                }
                Ok("automation-scripts-replace")
                    if !SAMPLE_OPENED.swap(true, std::sync::atomic::Ordering::Relaxed) =>
                {
                    file::screenshot_replace(&mut state);
                    self.dialog = Some(Dialog::ScriptReplace);
                }
                _ => {}
            }
        }
        if state.sample.is_some() && self.dialog.is_none() {
            self.dialog = Some(Dialog::ScriptSample);
        }
        let mut file_request = None;
        egui::ScrollArea::vertical()
            .id_salt("scripts-page")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if state.draft.is_none() {
                    egui::Frame::new()
                        .fill(p.surface)
                        .stroke(egui::Stroke::new(1.0, p.border))
                        .corner_radius(10)
                        .inner_margin(18)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                script_badge(ui, "LUA", p.blue);
                                script_badge(ui, "POWERSHELL", p.accent);
                            });
                            ui.add_space(6.0);
                            ui.label(RichText::new(tr("Your first script")).size(15.0).strong());
                            ui.label(tr("Run custom Lua or PowerShell code manually, on a timer, or when downloads and queues finish."));
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(tr("Choose + New script in the top right to start from a working example."))
                                    .color(p.muted),
                            );
                            ui.label(
                                RichText::new(tr("Choose Site reader as the trigger to teach Download Engine a new video site."))
                                    .color(p.muted),
                            );
                        });
                }
                if let Some(mut draft) = state.draft.take() {
                    egui::Frame::new()
                        .fill(p.surface)
                        .stroke(egui::Stroke::new(1.0, p.border))
                        .corner_radius(10)
                        .inner_margin(14)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            form::show(
                                ui,
                                &mut draft,
                                &mut state.pages,
                                &self.queues,
                                &mut state.values,
                                p,
                            );

                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                script_badge(
                                    ui,
                                    match draft.language {
                                        Language::Lua => "LUA",
                                        Language::PowerShell => "PS1",
                                    },
                                    match draft.language {
                                        Language::Lua => p.blue,
                                        Language::PowerShell => p.accent,
                                    },
                                );
                                ui.label(
                                    RichText::new(tf!("{} editor", draft.language.label()))
                                        .strong(),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "{} / 64 KiB",
                                                draft.body.len().div_ceil(1024)
                                            ))
                                            .small()
                                            .monospace()
                                            .color(p.muted),
                                        );
                                        // Right to left: these read "Open file… Save to file…".
                                        if ui
                                            .button(tr("Save to file…"))
                                            .on_hover_text(tr("Saves the code in the editor to a .lua or .ps1 file."))
                                            .clicked()
                                        {
                                            file_request = Some(file::Request::Save);
                                        }
                                        if ui
                                            .button(tr("Open file…"))
                                            .on_hover_text(tr("Copies a .lua or .ps1 file into the editor. Nothing is saved until you choose Save script."))
                                            .clicked()
                                        {
                                            file_request = Some(file::Request::Open);
                                        }
                                    },
                                );
                            });
                            ui.add_space(4.0);
                            editor::show(ui, &mut draft, &mut state.editor, p);
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(match (&draft.trigger, draft.language) {
                                    (Trigger::SiteReader { .. }, _) => tr("event.url is the page. Use xd.fetch to read it or the site's API, then return the videos you found."),
                                    (Trigger::DownloadFailed { .. }, Language::Lua) => tr("event.downloads lists the downloads that need attention, with their errors. Use xd.retry or xd.restart to try again."),
                                    (Trigger::DownloadFailed { .. }, Language::PowerShell) => tr("$Event.downloads lists the downloads that need attention, with their errors. Use Invoke-DownloadEngine retry or restart to try again."),
                                    (_, Language::Lua) => tr("Use xd.* to inspect or control queues and event.* for trigger details."),
                                    (_, Language::PowerShell) => tr("Use Invoke-DownloadEngine for CLI commands and $Event for trigger details."),
                                })
                                .small()
                                .color(p.muted),
                            );
                        });
                    state.draft = Some(draft.clone());
                    if std::mem::take(&mut state.values.clear) {
                        match self.store.clear_script_values(&draft.id) {
                            Ok(_) => {
                                state.values.shown = false;
                                refresh(&self.store, &mut state);
                            }
                            Err(e) => state.error = Some(e.to_string()),
                        }
                    }
                }
                if state.reader() {
                    ui.add_space(8.0);
                    test_card(ui, &self.store, &mut state, p);
                }

                // Only show Run logs card when a script is open or historical runs exist
                if state.draft.is_some() || !state.runs.is_empty() {
                    ui.add_space(8.0);
                    egui::Frame::new()
                        .fill(p.surface)
                        .stroke(egui::Stroke::new(1.0, p.border))
                        .corner_radius(10)
                        .inner_margin(14)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let visible = state.visible_runs();
                            let current_run = visible
                                .iter()
                                .find(|r| Some(&r.id) == state.selected_run.as_ref())
                                .cloned();

                            // Clean single-row header: Title + Status on left, Actions on right
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(tr("Run logs")).strong());
                                if let Some(run) = &current_run {
                                    let status_color = match run.state.as_str() {
                                        "succeeded" => p.success,
                                        "failed" | "timed_out" => p.warning,
                                        "running" | "queued" => p.accent,
                                        _ => p.orange,
                                    };
                                    script_badge(
                                        ui,
                                        &tr(match run.state.as_str() {
                                            "succeeded" => "Succeeded", "failed" => "Failed",
                                            "timed_out" => "Timed out", "running" => "Running",
                                            "queued" => "Queued", "stopped" => "Stopped",
                                            "interrupted" => "Interrupted", other => other,
                                        }).replace('_', " ").to_uppercase(),
                                        status_color,
                                    );
                                    if sample::is_sample(run) {
                                        script_badge(ui, &tr("Sample").to_uppercase(), p.blue);
                                    }
                                    ui.label(
                                        RichText::new(tf!(
                                            "{} · rev {}{}",
                                            run.language.label(),
                                            run.revision,
                                            run.exit_code
                                                .map_or(String::new(), |n| tf!(" · exit {n}", n=n))
                                        ))
                                        .small()
                                        .color(p.muted),
                                    );
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if !visible.is_empty()
                                            && ui
                                                .add_enabled(
                                                    !state.output.is_empty(),
                                                    egui::Button::new(tr("Copy log")),
                                                )
                                                .clicked()
                                        {
                                            ui.ctx().copy_text(log_text(&state.output));
                                        }
                                        if state.pinned_run && ui.small_button(tr("Latest run")).clicked()
                                        {
                                            state.pinned_run = false;
                                            refresh(&self.store, &mut state);
                                        }
                                        if !state.runs.is_empty()
                                            && ui
                                                .square_checkbox(&mut state.all_history, tr("All scripts"))
                                                .changed()
                                        {
                                            state.selected_run = None;
                                            refresh(&self.store, &mut state);
                                        }
                                    },
                                );
                            });

                            let visible = state.visible_runs();
                            if visible.is_empty() {
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(
                                        tr("No runs yet — save your script and choose Run script to view output here."),
                                    )
                                    .small()
                                    .color(p.muted),
                                );
                            } else {
                                ui.add_space(6.0);
                                if visible.len() > 1 || state.all_history {
                                    let before = state.selected_run.clone();
                                    let label = |r: &Run| {
                                        let mut label = format!(
                                            "{} · {} · {}",
                                            time(r.queued_at),
                                            r.name,
                                            r.trigger.replace('_', " ")
                                        );
                                        if sample::is_sample(r) {
                                            label = format!("{label} · {}", tr("sample"));
                                        }
                                        label
                                    };
                                    egui::ComboBox::from_id_salt("script-run")
                                        .width(ui.available_width())
                                        .selected_text(
                                            visible
                                                .iter()
                                                .find(|r| {
                                                    Some(&r.id) == state.selected_run.as_ref()
                                                })
                                                .map_or_else(|| tr("Choose a run").into(), label),
                                        )
                                        .show_ui(ui, |ui| {
                                            for r in &visible {
                                                ui.selectable_value(
                                                    &mut state.selected_run,
                                                    Some(r.id.clone()),
                                                    label(r),
                                                );
                                            }
                                        });
                                    if before != state.selected_run {
                                        state.pinned_run = true;
                                        refresh(&self.store, &mut state);
                                    }
                                    ui.add_space(4.0);
                                }
                                if let Some(run) = visible
                                    .iter()
                                    .find(|r| Some(&r.id) == state.selected_run.as_ref())
                                {
                                    if let Some(error) = &run.error {
                                        ui.colored_label(
                                            logs::level_color(
                                                download_engine::diagnostics::Level::Error,
                                                ui.visuals().dark_mode,
                                            ),
                                            error,
                                        );
                                    }
                                    egui::Frame::new()
                                        .fill(p.background)
                                        .stroke(egui::Stroke::new(1.0, p.border))
                                        .corner_radius(8)
                                        .inner_margin(10)
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            egui::ScrollArea::vertical()
                                                .id_salt(("script-output", &run.id))
                                                .max_height(180.0)
                                                .min_scrolled_height(64.0)
                                                .stick_to_bottom(true)
                                                .show(ui, |ui| {
                                                    if state.output.is_empty() {
                                                        ui.label(
                                                            RichText::new(if run.active() {
                                                                tr("Waiting for output…")
                                                            } else {
                                                                tr("This run produced no output.")
                                                            })
                                                            .color(p.muted),
                                                        );
                                                    }
                                                    ui.add(
                                                        egui::Label::new(output_layout(
                                                            &state.output,
                                                            ui.visuals().dark_mode,
                                                        ))
                                                        .selectable(true)
                                                        .wrap(),
                                                    );
                                                });
                                        });
                                }
                            }
                        });
                }
                #[cfg(feature = "screenshot")]
                if std::env::var("IDM_EX_TEST_VIEW").as_deref() == Ok("automation-scripts-logs") {
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                }
            });
        match file_request {
            Some(file::Request::Open) => self.open_script_file(&mut state),
            Some(file::Request::Save) => self.save_script_file(&mut state),
            None => {}
        }
        self.scripts = state;
    }
}
