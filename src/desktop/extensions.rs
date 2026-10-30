//! The Installed tab of the Extensions page, and the reviews shown before an
//! extension is installed, updated or removed.
use super::*;
use download_engine::{
    extensions::{Changes, Grants, Installed, MAX_PACKAGE_BYTES, Package, Review, Setting},
    script_automation::{Run, Script, Trigger},
};
use egui::{Align, Align2, FontId, Layout, Sense, Stroke, pos2, vec2};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};

const PUZZLE: &str = "\u{ea86}";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Detail {
    #[default]
    Overview,
    Settings,
    Code,
}

struct Job {
    script: Script,
    last: Option<Run>,
}

struct Entry {
    installed: Installed,
    jobs: Vec<Job>,
    conflict: Option<String>,
    saved: BTreeMap<String, serde_json::Value>,
}
impl Entry {
    fn id(&self) -> &str {
        &self.installed.package.id
    }
    fn failed(&self) -> Option<&Run> {
        self.jobs
            .iter()
            .filter_map(|job| job.last.as_ref())
            .find(|run| matches!(run.state.as_str(), "failed" | "timed_out"))
    }
    fn status(&self, p: Palette) -> (String, Color32) {
        if self.installed.enabled && self.conflict.is_some() {
            (tr("Never runs: page conflict").into(), p.orange)
        } else if self.failed().is_some() {
            (tr("Last run failed").into(), p.warning)
        } else if self.installed.enabled {
            (tr("Enabled").into(), p.accent)
        } else {
            (tr("Disabled").into(), p.muted)
        }
    }
}

/// A package waiting for the person to decide.
struct Pending {
    bytes: Vec<u8>,
    review: Review,
    file_access: BTreeSet<String>,
    queues: BTreeMap<String, String>,
    error: Option<String>,
}

#[derive(Default)]
pub(super) struct State {
    loaded: bool,
    entries: Vec<Entry>,
    selected: Option<String>,
    search: String,
    detail: Detail,
    error: Option<String>,
    pending: Option<Pending>,
    removing: Option<String>,
    /// Plain settings being edited, as text, by setting ID.
    values: BTreeMap<String, String>,
    /// New values for protected settings. Saved ones are never read back.
    secrets: BTreeMap<String, String>,
    settings_error: Option<String>,
    file: Option<String>,
    source_url: String,
    file_name: String,
    request: String,
    added: bool,
}

impl State {
    pub(super) fn count(&self) -> usize {
        self.entries.len()
    }
    pub(super) fn loaded(&self) -> bool {
        self.loaded
    }
}

enum Act {
    Select(String),
    Enable(String, bool),
    Remove(String),
    Copy(String, String),
    SaveSettings(String),
    ClearSetting(String, String),
    AddDownload(Script),
    OpenLog(String),
    OpenScripts,
}

fn panel(p: Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.background)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(10)
        .inner_margin(12)
}
fn group(p: Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(12, 10))
}
fn heading(ui: &mut egui::Ui, text: &str, p: Palette) {
    ui.label(RichText::new(text).small().strong().color(p.muted));
}

fn tile(painter: &egui::Painter, rect: egui::Rect, name: &str, p: Palette) {
    painter.rect_filled(rect, 7, p.track);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        name.chars()
            .next()
            .unwrap_or('?')
            .to_uppercase()
            .to_string(),
        FontId::proportional(rect.height() * 0.42),
        p.text,
    );
}

fn row(ui: &mut egui::Ui, entry: &Entry, selected: bool, p: Palette) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 62.0), Sense::click());
    let painter = ui.painter_at(rect);
    let fill = if selected {
        p.selected
    } else if response.hovered() {
        p.surface
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, 6, fill);
    let package = &entry.installed.package;
    tile(
        &painter,
        egui::Rect::from_min_size(rect.min + vec2(8.0, 15.0), vec2(32.0, 32.0)),
        &package.name,
        p,
    );
    let (status, color) = entry.status(p);
    for (y, text, color) in [
        (6.0, package.name.clone(), p.text),
        (
            23.0,
            format!("{} · {}", package.version, job_count(package.jobs.len())),
            p.muted,
        ),
        (40.0, status, color),
    ] {
        painter.text(
            pos2(rect.left() + 50.0, rect.top() + y),
            Align2::LEFT_TOP,
            text,
            FontId::proportional(if y == 6.0 { 13.0 } else { 12.0 }),
            color,
        );
    }
    response
        .on_hover_text(&package.name)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

fn job_count(count: usize) -> String {
    if count == 1 {
        tr("1 job").into()
    } else {
        tf!("{count} jobs", count = count)
    }
}

/// In plain words, what the package's jobs can and cannot do.
fn powers(package: &Package, files_allowed: bool) -> (Vec<String>, Vec<String>) {
    let web = package.jobs.iter().any(|job| !job.is_automation());
    let automation = package.jobs.iter().any(|job| job.is_automation());
    let hosts = package.permissions.network.join(", ");
    let mut can = Vec::new();
    if web && !hosts.is_empty() {
        can.push(tf!("Read {hosts}", hosts = hosts));
    }
    if automation {
        if !hosts.is_empty() {
            can.push(tf!("Add downloads only from {hosts}", hosts = hosts));
        }
        can.push(tr("See download names, folders and status, never their addresses").into());
        can.push(
            tr("Pause, resume and retry downloads, manage queues and show notifications").into(),
        );
    }
    if package
        .settings
        .iter()
        .any(|s| matches!(s, Setting::AesKey { .. }))
    {
        can.push(tr("Use a key you save in Settings").into());
    }
    if files_allowed {
        can.push(tr("Rename, move and delete any downloaded file. You allowed this.").into());
    }
    let mut cannot: Vec<String> = vec![
        tr("Use your browser logins or cookies").into(),
        tr("Run other programs").into(),
    ];
    if !files_allowed {
        cannot.push(tr("Change, move or delete your files").into());
    }
    if hosts.is_empty() {
        cannot.push(tr("Reach the web").into());
    }
    (can, cannot)
}

fn power_list(ui: &mut egui::Ui, items: &[String], allowed: bool, p: Palette) {
    for item in items {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(
                RichText::new(if allowed { "✓" } else { "×" }).color(if allowed {
                    p.accent
                } else {
                    p.muted
                }),
            );
            ui.add(egui::Label::new(item.as_str()).wrap());
        });
    }
}

fn kind_name(job: &Script) -> &'static str {
    match job.trigger {
        Trigger::SiteReader { .. } => tr("Site reader"),
        Trigger::Provider => tr("Download provider"),
        _ => tr("Automation"),
    }
}
fn runs_when(job: &Script) -> &'static str {
    match job.trigger {
        Trigger::SiteReader { .. } => tr("On matching pages"),
        Trigger::Provider => tr("When you add a download"),
        _ => scripts::trigger_name(&job.trigger),
    }
}
fn last_run(job: &Job) -> String {
    let Some(run) = &job.last else {
        return tr("Not run yet").into();
    };
    let state = match run.state.as_str() {
        "succeeded" => tr("Succeeded"),
        "queued" => tr("Queued"),
        "running" => tr("Running"),
        "failed" | "timed_out" => tr("Failed"),
        _ => tr("Stopped"),
    };
    match run.finished_at.or(run.started_at) {
        Some(at) => format!("{state} · {}", scripts::time(at)),
        None => state.into(),
    }
}

fn overview(ui: &mut egui::Ui, state: &mut State, index: usize, p: Palette) -> Option<Act> {
    let entry = &state.entries[index];
    let package = &entry.installed.package;
    let files = entry.jobs.iter().any(|job| job.script.change_files);
    let (can, cannot) = powers(package, files);
    let mut act = None;
    let width = ui.available_width();
    if width >= 560.0 {
        ui.columns(2, |columns| {
            group(p).show(&mut columns[0], |ui| {
                ui.set_width(ui.available_width());
                heading(ui, tr("What it can do"), p);
                power_list(ui, &can, true, p);
            });
            group(p).show(&mut columns[1], |ui| {
                ui.set_width(ui.available_width());
                heading(ui, tr("What it can't do"), p);
                power_list(ui, &cannot, false, p);
            });
        });
    } else {
        for (title, items, allowed) in [
            (tr("What it can do"), &can, true),
            (tr("What it can't do"), &cannot, false),
        ] {
            group(p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                heading(ui, title, p);
                power_list(ui, items, allowed, p);
            });
            ui.add_space(8.0);
        }
    }
    ui.add_space(8.0);
    group(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        heading(ui, tr("Jobs"), p);
        ui.add_space(4.0);
        let titles = [tr("Name"), tr("Type"), tr("Runs"), tr("Last run")];
        let rows: Vec<[String; 4]> = entry
            .jobs
            .iter()
            .map(|job| {
                [
                    job.script.name.clone(),
                    kind_name(&job.script).to_owned(),
                    runs_when(&job.script).to_owned(),
                    last_run(job),
                ]
            })
            .collect();
        let gap = 16.0;
        let font = egui::TextStyle::Body.resolve(ui.style());
        let measure = |text: &str| {
            ui.painter()
                .layout_no_wrap(text.to_owned(), font.clone(), p.accent)
                .size()
                .x
        };
        let table = (0..titles.len())
            .map(|column| {
                rows.iter()
                    .map(|row| measure(&row[column]))
                    .fold(measure(titles[column]), f32::max)
            })
            .sum::<f32>()
            + gap * (titles.len() - 1) as f32;
        if table > ui.available_width() {
            // Long names and translations can't share a row in a small window.
            for row in &rows {
                ui.label(&row[0]);
                ui.label(RichText::new(row[1..].join(" · ")).small().color(p.muted));
                ui.add_space(4.0);
            }
            return;
        }
        egui::Grid::new("extension-jobs")
            .num_columns(titles.len())
            .spacing(vec2(gap, 6.0))
            .min_col_width(60.0)
            .show(ui, |ui| {
                for title in titles {
                    ui.label(RichText::new(title).small().color(p.muted));
                }
                ui.end_row();
                for row in &rows {
                    for cell in row {
                        ui.label(cell);
                    }
                    ui.end_row();
                }
            });
    });
    let provider = entry
        .jobs
        .iter()
        .find(|job| job.script.trigger == Trigger::Provider)
        .map(|job| job.script.clone());
    if let Some(script) = provider {
        ui.add_space(8.0);
        group(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            heading(ui, &tf!("Add a download with {job}", job = script.name), p);
            ui.add_space(4.0);
            let mut changed = ui
                .add(
                    view::line_edit(&mut state.source_url)
                        .hint_text(tr("Source URL"))
                        .desired_width(ui.available_width())
                        .char_limit(16384),
                )
                .changed();
            ui.horizontal(|ui| {
                // The button's width depends on the language.
                let add = tr("Add download");
                let font = egui::TextStyle::Button.resolve(ui.style());
                let button = ui
                    .painter()
                    .layout_no_wrap(add.to_owned(), font, p.accent)
                    .size()
                    .x
                    + 2.0 * ui.spacing().button_padding.x
                    + ui.spacing().item_spacing.x;
                changed |= ui
                    .add(
                        view::line_edit(&mut state.file_name)
                            .hint_text(tr("File name"))
                            .desired_width((ui.available_width() - button).max(120.0))
                            .char_limit(240),
                    )
                    .changed();
                if ui
                    .add_enabled(
                        !state.source_url.trim().is_empty(),
                        egui::Button::new(add),
                    )
                    .clicked()
                {
                    act = Some(Act::AddDownload(script.clone()));
                }
            });
            if changed {
                state.request.clear();
                state.added = false;
            }
            if state.added {
                ui.label(
                    RichText::new(tr(
                        "Added to Main queue. Use the download list to pause, resume, stop or inspect errors.",
                    ))
                    .small()
                    .color(p.accent),
                );
            }
        });
    }
    act
}

fn settings(ui: &mut egui::Ui, state: &mut State, index: usize, p: Palette) -> Option<Act> {
    let id = state.entries[index].id().to_owned();
    let package = state.entries[index].installed.package.clone();
    let saved = state.entries[index].saved.clone();
    let mut act = None;
    if package.settings.is_empty() {
        ui.label(RichText::new(tr("This extension has no settings.")).color(p.muted));
        return None;
    }
    group(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::Grid::new("extension-settings")
            .num_columns(2)
            .spacing(vec2(16.0, 10.0))
            .min_col_width(130.0)
            .show(ui, |ui| {
                for setting in &package.settings {
                    let key = setting.id().to_owned();
                    ui.label(setting.label());
                    ui.vertical(|ui| {
                        let width = (ui.available_width() - 8.0).clamp(160.0, 380.0);
                        match setting {
                            Setting::Choice { options, .. } => {
                                let value = state.values.entry(key.clone()).or_default();
                                egui::ComboBox::from_id_salt(("extension-setting", &key))
                                    .selected_text(value.clone())
                                    .width(width)
                                    .show_ui(ui, |ui| {
                                        for option in options {
                                            ui.selectable_value(
                                                value,
                                                option.clone(),
                                                option.as_str(),
                                            );
                                        }
                                    });
                            }
                            Setting::Boolean { .. } => {
                                let value = state.values.entry(key.clone()).or_default();
                                let mut on = value == "true";
                                if ui.square_checkbox(&mut on, tr("On")).changed() {
                                    *value = on.to_string();
                                }
                            }
                            Setting::String { .. } | Setting::Integer { .. } => {
                                ui.add(
                                    view::line_edit(state.values.entry(key.clone()).or_default())
                                        .desired_width(width),
                                );
                            }
                            Setting::Secret { .. } | Setting::AesKey { .. } => {
                                let has = saved.contains_key(&key);
                                ui.horizontal(|ui| {
                                    ui.add(
                                        view::line_edit(
                                            state.secrets.entry(key.clone()).or_default(),
                                        )
                                        .password(true)
                                        .hint_text(if has {
                                            tr("Saved. Type to replace it")
                                        } else {
                                            tr("Not set")
                                        })
                                        .desired_width(width),
                                    );
                                    if has && ui.button(tr("Clear")).clicked() {
                                        act = Some(Act::ClearSetting(id.clone(), key.clone()));
                                    }
                                });
                                ui.label(
                                    RichText::new(if matches!(setting, Setting::Secret { .. }) {
                                        tr("Protected by Windows. Only this extension's web jobs can read it.")
                                    } else {
                                        tr("Protected by Windows. Only its provider jobs can use it, and it can't be shown again.")
                                    })
                                    .small()
                                    .color(p.muted),
                                );
                            }
                        }
                    });
                    ui.end_row();
                }
            });
    });
    if let Some(error) = &state.settings_error {
        ui.colored_label(p.warning, error);
    }
    ui.add_space(8.0);
    if ui.add(primary(tr("Save settings"), p)).clicked() {
        act = Some(Act::SaveSettings(id));
    }
    act
}

fn code(ui: &mut egui::Ui, state: &mut State, index: usize, p: Palette) -> Option<Act> {
    let package = &state.entries[index].installed.package;
    let mut act = None;
    let selected = state
        .file
        .clone()
        .filter(|file| package.files.contains_key(file))
        .or_else(|| package.jobs.first().map(|job| job.entry.clone()))
        .unwrap_or_default();
    let source = package.files.get(&selected).cloned().unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(tr("Read only. Copy a file to your scripts to change it."))
                .small()
                .color(p.muted),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(tr("Copy to my scripts")).clicked() {
                act = Some(Act::Copy(selected.clone(), source.clone()));
            }
        });
    });
    ui.add_space(6.0);
    let files: Vec<String> = package.files.keys().cloned().collect();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(150.0);
            for file in &files {
                if ui.selectable_label(*file == selected, file).clicked() {
                    state.file = Some(file.clone());
                }
            }
        });
        let mut text = source.as_str();
        egui::ScrollArea::both()
            .id_salt("extension-code")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY),
                );
            });
    });
    act
}

fn detail(ui: &mut egui::Ui, state: &mut State, index: usize, p: Palette) -> Option<Act> {
    let mut act = None;
    let entry = &state.entries[index];
    let id = entry.id().to_owned();
    let package = &entry.installed.package;
    let name = package.name.clone();
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        tile(ui.painter(), rect, &name, p);
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 180.0).max(120.0));
            ui.add(egui::Label::new(RichText::new(&name).size(16.0).strong()).truncate());
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "{} · {} · {}",
                        package.version,
                        package.publisher,
                        tr("Publisher not verified")
                    ))
                    .small()
                    .color(p.muted),
                )
                .truncate(),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(tr("Remove")).clicked() {
                act = Some(Act::Remove(id.clone()));
            }
            let mut enabled = entry.installed.enabled;
            if ui.square_checkbox(&mut enabled, tr("Enabled")).changed() {
                act = Some(Act::Enable(id.clone(), enabled));
            }
        });
    });
    ui.add_space(6.0);
    ui.add(egui::Label::new(&package.description).wrap());
    if entry.installed.enabled
        && let Some(blocker) = &entry.conflict
    {
        ui.add_space(6.0);
        group(p).stroke(Stroke::new(1.0, p.orange)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(tf!(
                    "{blocker} handles the same pages first, so this job never runs.",
                    blocker = blocker
                ));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(tr("Open Scripts")).clicked() {
                        act = Some(Act::OpenScripts);
                    }
                });
            });
        });
    } else if let Some(run) = entry.failed() {
        ui.add_space(6.0);
        let run_id = run.id.clone();
        let error = run.error.clone().unwrap_or_default();
        group(p).stroke(Stroke::new(1.0, p.warning)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(tf!("The last run failed: {error}", error = error)).truncate(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(tr("View log")).clicked() {
                        act = Some(Act::OpenLog(run_id.clone()));
                    }
                });
            });
        });
    }
    ui.add_space(8.0);
    egui::Frame::new()
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (tab, label) in [
                    (Detail::Overview, tr("Overview")),
                    (Detail::Settings, tr("Settings")),
                    (Detail::Code, tr("Code")),
                ] {
                    if automation::subtab_button(ui, state.detail == tab, label, p).clicked() {
                        state.detail = tab;
                    }
                }
            });
        });
    ui.add_space(8.0);
    let next = match state.detail {
        Detail::Overview => {
            egui::ScrollArea::vertical()
                .id_salt("extension-overview")
                .auto_shrink([false, false])
                .show(ui, |ui| overview(ui, state, index, p))
                .inner
        }
        Detail::Settings => {
            egui::ScrollArea::vertical()
                .id_salt("extension-settings-form")
                .auto_shrink([false, false])
                .show(ui, |ui| settings(ui, state, index, p))
                .inner
        }
        Detail::Code => code(ui, state, index, p),
    };
    act.or(next)
}

fn select(state: &mut State, id: String) {
    state.selected = Some(id);
    state.detail = Detail::Overview;
    state.file = None;
    state.source_url.clear();
    state.file_name.clear();
    state.request.clear();
    state.added = false;
    load_values(state);
}

fn load_values(state: &mut State) {
    state.values.clear();
    state.secrets.clear();
    state.settings_error = None;
    let Some(entry) = state
        .entries
        .iter()
        .find(|entry| Some(entry.id()) == state.selected.as_deref())
    else {
        return;
    };
    for setting in &entry.installed.package.settings {
        if setting.protected() {
            continue;
        }
        let value = entry
            .saved
            .get(setting.id())
            .cloned()
            .unwrap_or_else(|| setting.default_value());
        let text = match value {
            serde_json::Value::Null => String::new(),
            serde_json::Value::String(text) => text,
            other => other.to_string(),
        };
        state.values.insert(setting.id().to_owned(), text);
    }
}

fn plain_value(setting: &Setting, text: &str) -> anyhow::Result<serde_json::Value> {
    Ok(match setting {
        Setting::Integer { .. } => text
            .trim()
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!(tr("Enter a whole number")))?
            .into(),
        Setting::Boolean { .. } => (text == "true").into(),
        _ => text.into(),
    })
}

impl Desktop {
    pub(super) fn refresh_extensions(&mut self) {
        let result = (|| -> anyhow::Result<Vec<Entry>> {
            self.store
                .extensions()?
                .into_iter()
                .map(|installed| {
                    let id = installed.package.id.clone();
                    let mut jobs = Vec::new();
                    for job in &installed.package.jobs {
                        let script = self.store.extension_job(&id, &job.id)?;
                        let last = self.store.script_runs(Some(&script.id))?.into_iter().next();
                        jobs.push(Job { script, last });
                    }
                    Ok(Entry {
                        conflict: self.store.extension_conflict(&id)?,
                        saved: self.store.extension_settings(&id)?,
                        installed,
                        jobs,
                    })
                })
                .collect()
        })();
        let state = &mut self.extensions;
        state.loaded = true;
        match result {
            Ok(entries) => state.entries = entries,
            Err(error) => state.error = Some(error.to_string()),
        }
        if state
            .selected
            .as_deref()
            .is_none_or(|id| !state.entries.iter().any(|entry| entry.id() == id))
        {
            state.selected = state.entries.first().map(|e| e.id().to_owned());
            state.detail = Detail::Overview;
            state.file = None;
            load_values(state);
        }
        // This also runs on background updates, so what someone is typing stays.
    }

    #[cfg(feature = "screenshot")]
    pub(super) fn prepare_extensions_capture(&mut self, mode: &str) {
        self.refresh_extensions();
        match mode {
            "automation-installed-settings" => self.extensions.detail = Detail::Settings,
            "automation-installed-code" => self.extensions.detail = Detail::Code,
            "automation-installed-review" => {
                let bytes = std::env::var("IDM_EX_TEST_EXTENSION")
                    .ok()
                    .and_then(|path| std::fs::read(path).ok())
                    .unwrap_or_default();
                if let Ok(review) = self.store.review_extension(&bytes) {
                    self.review_extension(bytes, review);
                }
            }
            _ => {}
        }
    }

    pub(super) fn extension_actions(&mut self, ui: &mut egui::Ui, p: Palette) {
        if ui.add(primary(tr("Install from file…"), p)).clicked() {
            self.pick_extension_file();
        }
    }

    pub(super) fn pick_extension_file(&mut self) {
        let path = match self.folder_picker.open_file(
            tr("Install from file…"),
            &self.store.paths.downloads,
            (tr("Extension (*.de-extension.json)"), "*.de-extension.json"),
        ) {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                self.extensions.error = Some(error.to_string());
                return;
            }
        };
        let result = (|| -> anyhow::Result<_> {
            let mut bytes = Vec::new();
            std::fs::File::open(&path)?
                .take(MAX_PACKAGE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            let review = self.store.review_extension(&bytes)?;
            Ok((bytes, review))
        })();
        match result {
            Ok((bytes, review)) => self.review_extension(bytes, review),
            Err(error) => self.extensions.error = Some(format!("{error:#}")),
        }
    }

    pub(super) fn review_extension(&mut self, bytes: Vec<u8>, review: Review) {
        self.extensions.error = None;
        // An update starts from the queues already chosen.
        let queues = review
            .package
            .jobs
            .iter()
            .filter_map(|job| {
                let script = self.store.extension_job(&review.package.id, &job.id).ok()?;
                match script.trigger {
                    Trigger::QueueCompleted { queue } => Some((job.id.clone(), queue)),
                    _ => None,
                }
            })
            .collect();
        self.extensions.pending = Some(Pending {
            bytes,
            review,
            file_access: BTreeSet::new(),
            queues,
            error: None,
        });
        self.dialog = Some(Dialog::ExtensionReview);
    }

    pub(super) fn installed_tab(&mut self, ui: &mut egui::Ui, p: Palette) {
        if !self.extensions.loaded {
            self.refresh_extensions();
        }
        if let Some(error) = self.extensions.error.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(p.warning, error);
                if ui.button(tr("Dismiss")).clicked() {
                    self.extensions.error = None;
                }
            });
            ui.add_space(6.0);
        }
        if self.extensions.entries.is_empty() {
            let mut open_scripts = false;
            let mut install = false;
            panel(p).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.vertical_centered(|ui| {
                    ui.add_space((ui.available_height() / 2.0 - 70.0).max(16.0));
                    ui.label(RichText::new(tr("No extensions yet")).size(16.0).strong());
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(tr(
                            "Extensions add site support and automation without changing the app. Install one from a file, or write your own script.",
                        ))
                        .color(p.muted),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_space((ui.available_width() - 250.0).max(0.0) / 2.0);
                        install = ui.add(primary(tr("Install from file…"), p)).clicked();
                        open_scripts = ui.button(tr("Open Scripts")).clicked();
                    });
                });
            });
            if install {
                self.pick_extension_file();
            }
            if open_scripts {
                self.automation.show_scripts();
            }
            return;
        }
        let height = ui.available_height();
        let left = (ui.available_width() * 0.3).clamp(210.0, 292.0);
        let state = &mut self.extensions;
        let mut act = None;
        let full = vec2(ui.available_width(), height);
        ui.allocate_ui_with_layout(full, Layout::left_to_right(Align::Min), |ui| {
            panel(p).show(ui, |ui| {
                ui.set_width(left - 24.0);
                ui.set_min_height(height - 26.0);
                // A frame inherits this row's direction; its content stacks.
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.add(
                        view::line_edit(&mut state.search)
                            .id(ui.id().with("extension-search"))
                            .hint_text(tr("Search extensions"))
                            .desired_width(ui.available_width()),
                    );
                    ui.add_space(6.0);
                    let query = state.search.trim().to_lowercase();
                    egui::ScrollArea::vertical()
                        .id_salt("extension-list")
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for entry in &state.entries {
                                let package = &entry.installed.package;
                                if !query.is_empty()
                                    && !format!("{} {}", package.name, package.publisher)
                                        .to_lowercase()
                                        .contains(&query)
                                {
                                    continue;
                                }
                                let selected = state.selected.as_deref() == Some(entry.id());
                                if row(ui, entry, selected, p) {
                                    act = Some(Act::Select(entry.id().to_owned()));
                                }
                            }
                        });
                });
            });
            ui.add_space(4.0);
            let index = state
                .entries
                .iter()
                .position(|entry| Some(entry.id()) == state.selected.as_deref())
                .unwrap_or(0);
            panel(p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(height - 26.0);
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    if let Some(next) = detail(ui, state, index, p) {
                        act = Some(next);
                    }
                });
            });
        });
        if let Some(act) = act {
            self.extension_action(act);
        }
    }

    fn extension_action(&mut self, act: Act) {
        let saving = matches!(act, Act::SaveSettings(_));
        let result = match act {
            Act::Select(id) => {
                select(&mut self.extensions, id);
                Ok(())
            }
            Act::Enable(id, enabled) => self.store.set_extension_enabled(&id, enabled),
            Act::Remove(id) => {
                self.extensions.removing = Some(id);
                self.dialog = Some(Dialog::ExtensionRemove);
                Ok(())
            }
            Act::Copy(name, body) => {
                if self.scripts.untouched() {
                    self.scripts.new_script_from(&name, &body);
                    self.automation.show_scripts();
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(tr(
                        "Save or discard your open script first"
                    )))
                }
            }
            Act::SaveSettings(id) => self.save_extension_settings(&id),
            Act::ClearSetting(id, key) => self.store.clear_extension_setting(&id, &key),
            Act::AddDownload(script) => self.add_extension_download(&script),
            Act::OpenLog(run) => {
                self.scripts.show_run(run);
                self.automation.show_scripts();
                Ok(())
            }
            Act::OpenScripts => {
                self.automation.show_scripts();
                Ok(())
            }
        };
        if let Err(error) = result {
            self.extensions.error = Some(format!("{error:#}"));
        }
        // Keep what was typed so a refused value can be corrected.
        if self.extensions.settings_error.is_none() {
            self.refresh_extensions();
            if saving {
                load_values(&mut self.extensions);
            }
        }
    }

    fn save_extension_settings(&mut self, id: &str) -> anyhow::Result<()> {
        let state = &mut self.extensions;
        state.settings_error = None;
        let package = self.store.extension(id)?.package;
        let result = (|| -> anyhow::Result<()> {
            for setting in &package.settings {
                if setting.protected() {
                    let text = state.secrets.get(setting.id()).map_or("", String::as_str);
                    if !text.is_empty() {
                        self.store
                            .set_extension_secret(id, setting.id(), text)
                            .map_err(|e| anyhow::anyhow!("{}: {e}", setting.label()))?;
                    }
                } else {
                    let text = state.values.get(setting.id()).map_or("", String::as_str);
                    if text.is_empty() {
                        continue;
                    }
                    let value = plain_value(setting, text)
                        .map_err(|e| anyhow::anyhow!("{}: {e}", setting.label()))?;
                    self.store
                        .set_extension_setting(id, setting.id(), &value)
                        .map_err(|e| anyhow::anyhow!("{}: {e}", setting.label()))?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            state.settings_error = Some(error.to_string());
        }
        Ok(())
    }

    fn add_extension_download(&mut self, script: &Script) -> anyhow::Result<()> {
        let state = &mut self.extensions;
        if state.request.is_empty() {
            state.request = uuid::Uuid::new_v4().to_string();
        }
        let name = state.file_name.trim();
        self.store.add_provider_download(
            &script.id,
            script.revision,
            state.source_url.trim(),
            (!name.is_empty()).then_some(name),
            MAIN_QUEUE,
            &state.request,
        )?;
        state.added = true;
        Ok(())
    }

    pub(super) fn extension_review_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(pending) = &mut self.extensions.pending else {
            self.dialog = None;
            return;
        };
        let package = pending.review.package.clone();
        let update = pending.review.update.clone();
        let title = if update.is_some() {
            tf!("Update {name}?", name = package.name)
        } else {
            tf!("Install {name}?", name = package.name)
        };
        let version = match &update {
            Some(changes) => tf!(
                "{from} to {to}",
                from = changes.from_version,
                to = package.version
            ),
            None => package.version.clone(),
        };
        let subtitle = format!(
            "{version} · {} · {}",
            package.publisher,
            tr("Publisher not verified")
        );
        let mut cancel = false;
        let mut install = false;
        let response = form_dialog::modal("extension-review", p).show(ctx, |ui| {
            cancel |= form_dialog::header(
                ui,
                &title,
                &subtitle,
                form_dialog::Icon::Symbol(PUZZLE),
                p,
                true,
            );
            form_dialog::body(ui, "extension-review-body", |ui| {
                ui.add(egui::Label::new(&package.description).wrap());
                ui.add_space(6.0);
                match &update {
                    Some(changes) => update_summary(ui, changes, p),
                    None => {
                        let (can, _) = powers(&package, false);
                        group(p).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            heading(ui, tr("It can"), p);
                            power_list(ui, &can, true, p);
                        });
                    }
                }
                ui.add_space(6.0);
                file_requests(ui, pending, &package, update.as_ref(), p);
                queue_choices(ui, pending, &package, &self.queues, p);
                group(p).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    heading(ui, &tf!("Jobs ({count})", count = package.jobs.len()), p);
                    for job in &package.jobs {
                        ui.horizontal(|ui| {
                            ui.label(&job.name);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(job_kind(job)).color(p.muted));
                            });
                        });
                    }
                });
                ui.add_space(6.0);
                let notes: Vec<&str> = match &update {
                    Some(changes) => vec![
                        tr("Not now keeps the installed version exactly as it is."),
                        tr("Downloads already running keep the code they started with."),
                        if changes.dropped_settings.is_empty() {
                            tr("Your settings, keys and stored values are kept.")
                        } else {
                            tr("Your other settings, keys and stored values are kept.")
                        },
                    ],
                    None => vec![tr(
                        "It installs turned off. Turn it on after you review it.",
                    )],
                };
                for note in notes {
                    ui.label(RichText::new(note).small().color(p.muted));
                }
                if let Some(error) = &pending.error {
                    ui.colored_label(p.warning, error);
                }
            });
            form_dialog::controls(ui, |ui| {
                let action = match &update {
                    Some(changes) if changes.needs_approval() => tr("Approve and update"),
                    Some(_) => tr("Update"),
                    None => tr("Install"),
                };
                install = form_dialog::primary_button(ui, action).clicked();
                cancel |= form_dialog::secondary_button(
                    ui,
                    if update.is_some() {
                        tr("Not now")
                    } else {
                        tr("Cancel")
                    },
                )
                .clicked();
            });
        });
        cancel |= response.should_close();
        if cancel {
            self.extensions.pending = None;
            self.dialog = None;
        } else if install {
            let grants = Grants {
                file_access: pending.file_access.clone(),
                queues: pending.queues.clone(),
                approved: true,
            };
            match self.store.install_extension(&pending.bytes, &grants) {
                Ok(installed) => {
                    self.extensions.pending = None;
                    self.dialog = None;
                    self.refresh_extensions();
                    select(&mut self.extensions, installed.package.id);
                }
                Err(error) => pending.error = Some(format!("{error:#}")),
            }
        }
    }

    pub(super) fn extension_remove_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(id) = self.extensions.removing.clone() else {
            self.dialog = None;
            return;
        };
        let name = self
            .extensions
            .entries
            .iter()
            .find(|entry| entry.id() == id)
            .map_or(id.clone(), |entry| entry.installed.package.name.clone());
        let mut cancel = false;
        let mut remove = false;
        let response = form_dialog::modal("extension-remove", p).show(ctx, |ui| {
            ui.set_width(430.0);
            cancel = form_dialog::title(
                ui,
                &tf!("Remove {name}?", name = name),
                form_dialog::Icon::Symbol("\u{e7ba}"),
                p,
                true,
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(tr(
                    "Its settings, saved keys and stored values are deleted. Run history stays, and files you downloaded are kept.",
                ))
                .color(p.muted),
            );
            form_dialog::controls(ui, |ui| {
                remove = form_dialog::primary_button(ui, tr("Remove")).clicked();
                cancel |= form_dialog::secondary_button(ui, tr("Cancel")).clicked();
            });
        });
        cancel |= response.should_close();
        if remove {
            if let Err(error) = self.store.remove_extension(&id) {
                self.extensions.error = Some(format!("{error:#}"));
            }
            self.refresh_extensions();
        }
        if cancel || remove {
            self.extensions.removing = None;
            self.dialog = None;
        }
    }
}

fn job_kind(job: &download_engine::extensions::Job) -> &'static str {
    match job.trigger {
        Trigger::SiteReader { .. } => tr("Site reader"),
        Trigger::Provider => tr("Download provider"),
        _ => tr("Automation"),
    }
}

fn update_summary(ui: &mut egui::Ui, changes: &Changes, p: Palette) {
    group(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        heading(ui, tr("What changes"), p);
        let mut new = |text: String| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(tr("New access"))
                        .small()
                        .strong()
                        .color(p.orange),
                );
                ui.label(text);
            });
        };
        if let Some(publisher) = &changes.new_publisher {
            new(tf!("Publisher changes to {name}", name = publisher));
        }
        for host in &changes.added_hosts {
            new(tf!("Read {hosts}", hosts = host));
        }
        for job in &changes.new_file_access {
            new(tf!("{job} asks to change downloaded files", job = job));
        }
        for kind in &changes.new_kinds {
            new(match kind.as_str() {
                "automation" => tr("See download names, folders and status").into(),
                "site_reader" => tr("Read pages through a new site reader").into(),
                _ => tr("Process downloads through a new provider").into(),
            });
        }
        for job in &changes.changed_jobs {
            new(tf!("{job} changes when or how it runs", job = job));
        }
        if !changes.changed_files.is_empty() {
            ui.label(tf!(
                "Code files changed: {files}",
                files = changes.changed_files.join(", ")
            ));
        }
        if !changes.dropped_settings.is_empty() {
            ui.label(tf!(
                "Settings reset: {settings}",
                settings = changes.dropped_settings.join(", ")
            ));
        }
        for host in &changes.removed_hosts {
            ui.label(RichText::new(tf!("No longer reads {host}", host = host)).color(p.muted));
        }
        if changes.added_jobs.is_empty() && changes.removed_jobs.is_empty() {
            ui.label(tr("No jobs added or removed"));
        } else {
            for job in &changes.added_jobs {
                ui.label(tf!("New job: {job}", job = job));
            }
            for job in &changes.removed_jobs {
                ui.label(tf!("Removed job: {job}", job = job));
            }
        }
    });
}

fn file_requests(
    ui: &mut egui::Ui,
    pending: &mut Pending,
    package: &Package,
    update: Option<&Changes>,
    p: Palette,
) {
    // An update keeps what was already granted, so it only asks about what is new.
    let requests: Vec<_> = package
        .jobs
        .iter()
        .filter(|job| {
            job.change_files && update.is_none_or(|c| c.new_file_access.contains(&job.name))
        })
        .collect();
    if requests.is_empty() {
        return;
    }
    group(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        heading(ui, tr("File access requested"), p);
        for job in requests {
            let mut allowed = pending.file_access.contains(&job.id);
            if ui
                .square_checkbox(
                    &mut allowed,
                    tf!(
                        "Let {job} rename, move and delete any downloaded file",
                        job = job.name
                    ),
                )
                .changed()
            {
                if allowed {
                    pending.file_access.insert(job.id.clone());
                } else {
                    pending.file_access.remove(&job.id);
                }
            }
        }
    });
    ui.add_space(6.0);
}

fn queue_choices(
    ui: &mut egui::Ui,
    pending: &mut Pending,
    package: &Package,
    queues: &[DownloadQueue],
    p: Palette,
) {
    let jobs: Vec<_> = package
        .jobs
        .iter()
        .filter(|job| matches!(job.trigger, Trigger::QueueCompleted { .. }))
        .collect();
    if jobs.is_empty() {
        return;
    }
    group(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        heading(ui, tr("Choose a queue"), p);
        for job in jobs {
            ui.label(&job.name);
            let id = pending
                .queues
                .entry(job.id.clone())
                .or_insert_with(|| MAIN_QUEUE.into());
            automation::queue_picker(ui, &format!("extension-queue-{}", job.id), queues, id);
        }
    });
    ui.add_space(6.0);
}

#[cfg(test)]
mod tests;
