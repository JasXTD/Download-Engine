//! Declarative automation editor and plain-text run history.
use super::*;
use download_engine::automation::{Rule, Run, Workflow};
use egui::{Align, Layout, Stroke};

/// The tabs of the Extensions page.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Installed,
    Scripts,
    Rules,
    History,
}

#[derive(Default)]
pub(super) struct State {
    loaded: bool,
    rules: Vec<Rule>,
    runs: Vec<Run>,
    /// Script and extension runs, each with where it came from.
    script_runs: Vec<(download_engine::script_automation::Run, String)>,
    tab: Tab,
    error: Option<String>,
    load_error: Option<String>,
    draft: Option<Draft>,
}
impl State {
    /// Open the Scripts tab.
    pub(super) fn show_scripts(&mut self) {
        self.tab = Tab::Scripts;
    }
}

struct Draft {
    id: String,
    name: String,
    enabled: bool,
    chain: bool,
    host: String,
    extension: String,
    source: String,
    queue: String,
    limited: bool,
    speed: String,
    error: Option<String>,
}
impl Draft {
    fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            enabled: true,
            chain: false,
            host: String::new(),
            extension: String::new(),
            source: MAIN_QUEUE.into(),
            queue: MAIN_QUEUE.into(),
            limited: false,
            speed: "1024".into(),
            error: None,
        }
    }
    fn from_rule(rule: &Rule) -> Self {
        let mut form = Self {
            id: rule.id.clone(),
            name: rule.name.clone(),
            enabled: rule.enabled,
            ..Self::new()
        };
        match &rule.workflow {
            Workflow::DownloadAdded {
                host,
                extension,
                queue,
                bytes_per_second,
            } => {
                form.host = host.clone();
                form.extension = extension.clone();
                form.queue = queue.clone();
                form.limited = bytes_per_second.is_some();
                if let Some(speed) = bytes_per_second {
                    form.speed = (speed / 1024).to_string();
                }
            }
            Workflow::QueueCompleted { source, start } => {
                form.chain = true;
                form.source = source.clone();
                form.queue = start.clone();
            }
        }
        form
    }
    fn rule(&self) -> anyhow::Result<Rule> {
        let workflow = if self.chain {
            Workflow::QueueCompleted {
                source: self.source.clone(),
                start: self.queue.clone(),
            }
        } else {
            Workflow::DownloadAdded {
                host: self.host.trim().to_ascii_lowercase(),
                extension: self
                    .extension
                    .trim()
                    .trim_start_matches('.')
                    .to_ascii_lowercase(),
                queue: self.queue.clone(),
                bytes_per_second: if self.limited {
                    Some(
                        self.speed
                            .trim()
                            .parse::<u64>()
                            .map_err(|_| {
                                anyhow::anyhow!(tr("Enter a whole-number speed in KiB/s"))
                            })?
                            .checked_mul(1024)
                            .ok_or_else(|| anyhow::anyhow!(tr("Speed is too large")))?,
                    )
                } else {
                    None
                },
            }
        };
        Ok(Rule {
            id: self.id.clone(),
            name: self.name.trim().into(),
            enabled: self.enabled,
            workflow,
        })
    }
}

pub(super) fn queue_label(queues: &[DownloadQueue], id: &str) -> String {
    queues
        .iter()
        .find(|q| q.id == id)
        .map(|q| display_queue_name(&q.id, &q.name).to_owned())
        .unwrap_or_else(|| tr("Queue removed — choose another").into())
}
pub(super) fn queue_picker(
    ui: &mut egui::Ui,
    key: &str,
    queues: &[DownloadQueue],
    id: &mut String,
) {
    egui::ComboBox::from_id_salt(key)
        .selected_text(queue_label(queues, id))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for q in queues {
                ui.selectable_value(id, q.id.clone(), display_queue_name(&q.id, &q.name));
            }
        });
}

fn badge_pill(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    fill: Color32,
    stroke: Color32,
    ink: Color32,
) {
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).size(11.5).strong().color(ink));
        });
}

fn flow_pill(ui: &mut egui::Ui, prefix: &str, value: &str, accent: Option<Color32>, p: Palette) {
    let border = accent.map_or(p.border, |c| c.gamma_multiply(0.45));
    let fill = accent.map_or(p.background, |c| c.gamma_multiply(0.10));
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if !prefix.is_empty() {
                    ui.label(RichText::new(prefix).size(11.5).color(p.muted));
                }
                ui.label(
                    RichText::new(value)
                        .size(11.5)
                        .strong()
                        .color(accent.unwrap_or(p.text)),
                );
            });
        });
}

fn render_rule_flow(ui: &mut egui::Ui, rule: &Rule, queues: &[DownloadQueue], p: Palette) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 5.0);
        badge_pill(ui, "WHEN", p.background, p.border, p.muted);
        match &rule.workflow {
            Workflow::DownloadAdded {
                host,
                extension,
                queue,
                bytes_per_second,
            } => {
                let host_label = if host.is_empty() {
                    tr("Any site")
                } else {
                    host.as_str()
                };
                let ext_label = if extension.is_empty() {
                    tr("Any file type").into()
                } else {
                    format!(".{extension}")
                };
                flow_pill(
                    ui,
                    tr("Host:"),
                    host_label,
                    (!host.is_empty()).then_some(p.accent),
                    p,
                );
                flow_pill(
                    ui,
                    tr("Type:"),
                    &ext_label,
                    (!extension.is_empty()).then_some(p.accent),
                    p,
                );
                ui.label(RichText::new("→").strong().color(p.accent));
                badge_pill(ui, "THEN", p.background, p.border, p.muted);
                flow_pill(
                    ui,
                    tr("Queue:"),
                    &queue_label(queues, queue),
                    Some(p.accent),
                    p,
                );
                if let Some(speed) = bytes_per_second {
                    flow_pill(
                        ui,
                        tr("Limit:"),
                        &format!("{} KiB/s", speed / 1024),
                        Some(p.orange),
                        p,
                    );
                }
            }
            Workflow::QueueCompleted { source, start } => {
                flow_pill(ui, tr("Finishes:"), &queue_label(queues, source), None, p);
                ui.label(RichText::new("→").strong().color(p.blue));
                badge_pill(ui, "THEN", p.background, p.border, p.muted);
                flow_pill(
                    ui,
                    tr("Start queue:"),
                    &queue_label(queues, start),
                    Some(p.blue),
                    p,
                );
            }
        }
    });
}

pub(super) fn subtab_button(
    ui: &mut egui::Ui,
    selected: bool,
    label: impl Into<String>,
    p: Palette,
) -> egui::Response {
    let text = label.into();
    let rich = RichText::new(text)
        .size(12.5)
        .color(if selected { p.text } else { p.muted });
    let rich = if selected { rich.strong() } else { rich };
    ui.add(
        egui::Button::new(rich)
            .fill(if selected {
                p.selected
            } else {
                Color32::TRANSPARENT
            })
            .stroke(if selected {
                Stroke::new(1.0, p.accent.gamma_multiply(0.45))
            } else {
                Stroke::NONE
            })
            .corner_radius(6)
            .min_size(egui::vec2(76.0, 26.0)),
    )
}

impl Desktop {
    #[cfg(feature = "screenshot")]
    pub(super) fn prepare_automation_capture(&mut self, mode: &str) {
        self.refresh_automation();
        self.automation.tab = if mode == "automation-history" {
            Tab::History
        } else if mode.starts_with("automation-scripts") {
            Tab::Scripts
        } else if mode.starts_with("automation-installed") {
            self.prepare_extensions_capture(mode);
            Tab::Installed
        } else {
            Tab::Rules
        };
        if matches!(mode, "automation-edit" | "automation-error") {
            let mut draft = self
                .automation
                .rules
                .first()
                .map(Draft::from_rule)
                .unwrap_or_else(Draft::new);
            if mode == "automation-error" {
                draft.error = Some(tr("Choose a different queue to start").into());
                draft.chain = true;
                draft.queue = draft.source.clone();
            }
            self.automation.draft = Some(draft);
            self.dialog = Some(Dialog::Automation);
        }
    }

    pub(super) fn refresh_automation(&mut self) {
        scripts::refresh(&self.store, &mut self.scripts);
        // The count on the tab needs a first load, whichever tab opens.
        if self.automation.tab == Tab::Installed || !self.extensions.loaded() {
            self.refresh_extensions();
        }
        let result = (|| -> anyhow::Result<_> {
            let origins = self.store.script_origins()?;
            let script_runs = self
                .store
                .script_runs(None)?
                .into_iter()
                .map(|run| {
                    let origin = match origins.get(&run.script_id) {
                        Some(Some(name)) => tf!("{name} extension", name = name),
                        Some(None) => tr("Your script").into(),
                        None => tr("Removed").into(),
                    };
                    (run, origin)
                })
                .collect();
            Ok((
                self.store.automation_rules()?,
                self.store.automation_history()?,
                script_runs,
            ))
        })();
        match result {
            Ok((rules, runs, script_runs)) => {
                self.automation.rules = rules;
                self.automation.runs = runs;
                self.automation.script_runs = script_runs;
                self.automation.load_error = None;
            }
            Err(error) => self.automation.load_error = Some(error.to_string()),
        }
        self.automation.loaded = true;
    }

    pub(super) fn automation_page(&mut self, ui: &mut egui::Ui, p: Palette) {
        if !self.automation.loaded {
            self.refresh_automation();
        }
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - 128.0).max(260.0));
                ui.label(RichText::new(tr("Extensions")).size(18.5).strong());
                ui.label(
                    RichText::new(tr("Install extensions, write your own scripts and route downloads with rules."))
                        .small()
                        .color(p.muted),
                );
            });
            match self.automation.tab {
                Tab::Rules => {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(primary(tr("+ New rule"), p)
                            .min_size(egui::vec2(112.0, view::PRIMARY_HEIGHT))).clicked() {
                            self.automation.draft = Some(Draft::new());
                            self.dialog = Some(Dialog::Automation);
                        }
                    });
                }
                Tab::Installed => {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        self.extension_actions(ui, p);
                    });
                }
                Tab::Scripts | Tab::History => {}
            }
        });
        ui.add_space(10.0);

        // Unified Segmented Sub-Navigation + Contextual Guide Bar
        ui.horizontal(|ui| {
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(8)
                .inner_margin(egui::Margin::same(3))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let tab = self.automation.tab;
                    if subtab_button(
                        ui,
                        tab == Tab::Installed,
                        tf!("Installed ({})", self.extensions.count()),
                        p,
                    )
                    .clicked()
                    {
                        self.automation.tab = Tab::Installed;
                        self.refresh_extensions();
                    }
                    if subtab_button(ui, tab == Tab::Scripts, tr("Scripts"), p).clicked() {
                        self.automation.tab = Tab::Scripts;
                    }
                    if subtab_button(
                        ui,
                        tab == Tab::Rules,
                        tf!("Rules ({})", self.automation.rules.len()),
                        p,
                    )
                    .clicked()
                    {
                        self.automation.tab = Tab::Rules;
                    }
                    if subtab_button(ui, tab == Tab::History, tr("Run history"), p).clicked() {
                        self.automation.tab = Tab::History;
                    }
                });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (guide_label, guide_topic) = match self.automation.tab {
                    Tab::Scripts => self.scripts.guide(),
                    Tab::Installed => (tr("Extension guide"), "Extensions"),
                    Tab::Rules | Tab::History => (tr("How rules work"), "Scripting"),
                };
                if ui
                    .add(
                        egui::Button::new(RichText::new(guide_label).size(12.5))
                            .fill(p.surface)
                            .stroke(Stroke::new(1.0, p.border))
                            .corner_radius(7)
                            .min_size(egui::vec2(104.0, view::CONTROL_HEIGHT)),
                    )
                    .clicked()
                {
                    self.open_guide(guide_topic);
                }
                if self.automation.tab == Tab::Scripts {
                    if ui
                        .add_enabled(
                            self.scripts.can_export(),
                            egui::Button::new(RichText::new(tr("Export script…")).size(12.5)),
                        )
                        .clicked()
                    {
                        self.scripts.package_action = Some(false);
                    }
                    if ui
                        .button(RichText::new(tr("Import script…")).size(12.5))
                        .on_hover_text(tr("Lua automation and site readers"))
                        .clicked()
                    {
                        self.scripts.package_action = Some(true);
                    }
                }
            });
        });
        ui.add_space(8.0);
        // Results of actions taken here, such as opening or saving a script file.
        self.notice(ui, p);
        match self.automation.tab {
            Tab::Scripts => {
                self.scripts_page(ui, p);
                return;
            }
            Tab::Installed => {
                self.installed_tab(ui, p);
                return;
            }
            Tab::Rules | Tab::History => {}
        }
        let history = self.automation.tab == Tab::History;
        let mut open_log = None;
        if let Some(error) = self.automation.error.clone() {
            egui::Frame::new()
                .fill(p.warning.gamma_multiply(0.12))
                .stroke(Stroke::new(1.0, p.warning.gamma_multiply(0.45)))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(p.warning, error);
                        if ui.button(tr("Dismiss")).clicked() {
                            self.automation.error = None;
                        }
                    });
                });
            ui.add_space(6.0);
        }
        if let Some(error) = &self.automation.load_error {
            ui.colored_label(p.warning, error);
        }
        ui.label(
            RichText::new(if history {
                tr("Latest 500 runs. Completed actions are immediate and cannot be cancelled.")
            } else {
                tr("Rules run while the app is open, including in the tray. The first matching routing rule wins.")
            })
            .small()
            .color(p.muted),
        );
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("automation-page")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if history {
                    if self.automation.runs.is_empty() && self.automation.script_runs.is_empty() {
                        egui::Frame::new()
                            .fill(p.surface)
                            .stroke(Stroke::new(1.0, p.border))
                            .corner_radius(10)
                            .inner_margin(18)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(tr("No runs yet. Matching downloads and completed queues will appear here.")).color(p.muted));
                            });
                    }
                    for run in &self.automation.runs {
                        let date = chrono::DateTime::from_timestamp(run.at, 0)
                            .map(|d| {
                                d.with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M:%S")
                                    .to_string()
                            })
                            .unwrap_or_else(|| run.at.to_string());
                        let level = if run.success {
                            download_engine::diagnostics::Level::Info
                        } else {
                            download_engine::diagnostics::Level::Error
                        };
                        let color = logs::level_color(level, ui.visuals().dark_mode);
                        egui::Frame::new()
                            .fill(p.surface)
                            .stroke(Stroke::new(
                                1.0,
                                if run.success {
                                    p.border
                                } else {
                                    p.warning.gamma_multiply(0.5)
                                },
                            ))
                            .corner_radius(9)
                            .inner_margin(egui::Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    badge_pill(
                                        ui,
                                        if run.success { tr("INFO · OK") } else { "ERROR" },
                                        color.gamma_multiply(0.14),
                                        color.gamma_multiply(0.40),
                                        color,
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(
                                            RichText::new(&date)
                                                .small()
                                                .monospace()
                                                .color(p.muted),
                                        );
                                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                            ui.add(egui::Label::new(RichText::new(&run.rule_name).strong()).truncate())
                                                .on_hover_text(&run.rule_name);
                                        });
                                    });
                                });
                                ui.add_space(4.0);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&run.message)
                                            .monospace()
                                            .size(12.0)
                                            .color(if run.success { p.text } else { color }),
                                    )
                                    .selectable(true)
                                    .wrap(),
                                );
                            });
                        ui.add_space(8.0);
                    }
                    for (run, origin) in &self.automation.script_runs {
                        egui::Frame::new()
                            .fill(p.surface)
                            .stroke(Stroke::new(1.0, p.border))
                            .corner_radius(9)
                            .inner_margin(egui::Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.add(egui::Label::new(RichText::new(&run.name).strong()).truncate())
                                        .on_hover_text(&run.name);
                                    ui.label(RichText::new(origin).small().color(p.muted));
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if ui.button(tr("View log")).clicked() {
                                            open_log = Some(run.id.clone());
                                        }
                                        ui.label(RichText::new(&run.state).small().monospace().color(p.muted));
                                    });
                                });
                            });
                        ui.add_space(8.0);
                    }
                } else {
                    if self.automation.rules.is_empty() {
                        egui::Frame::new()
                            .fill(p.surface)
                            .stroke(Stroke::new(1.0, p.border))
                            .corner_radius(10)
                            .inner_margin(18)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    badge_pill(
                                        ui,
                                        "ROUTING",
                                        p.accent.gamma_multiply(0.14),
                                        p.accent.gamma_multiply(0.35),
                                        p.accent,
                                    );
                                    badge_pill(
                                        ui,
                                        tr("QUEUE CHAINING"),
                                        p.blue.gamma_multiply(0.14),
                                        p.blue.gamma_multiply(0.35),
                                        p.blue,
                                    );
                                });
                                ui.add_space(6.0);
                                ui.label(RichText::new(tr("Your first rule")).size(15.0).strong());
                                ui.label(tr("Send .zip downloads to a work queue, apply a speed limit, or start another queue when one finishes."));
                                ui.add_space(6.0);
                                ui.label(RichText::new(tr("Choose New rule to get started.")).color(p.muted));
                            });
                    }
                    for (index, rule) in self.automation.rules.clone().into_iter().enumerate() {
                        let is_chain = matches!(rule.workflow, Workflow::QueueCompleted { .. });
                        ui.push_id(&rule.id, |ui| {
                            egui::Frame::new()
                                .fill(p.surface)
                                .stroke(Stroke::new(
                                    1.0,
                                    if rule.enabled {
                                        p.border
                                    } else {
                                        p.border.gamma_multiply(0.65)
                                    },
                                ))
                                .corner_radius(10)
                                .inner_margin(14)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        badge_pill(
                                            ui,
                                            format!("#{}", index + 1),
                                            p.background,
                                            p.border,
                                            p.muted,
                                        );
                                        if is_chain {
                                            badge_pill(
                                                ui,
                                                tr("QUEUE CHAIN"),
                                                p.blue.gamma_multiply(0.14),
                                                p.blue.gamma_multiply(0.35),
                                                p.blue,
                                            );
                                        } else {
                                            badge_pill(
                                                ui,
                                                "ROUTING",
                                                p.accent.gamma_multiply(0.14),
                                                p.accent.gamma_multiply(0.35),
                                                p.accent,
                                            );
                                        }
                                        let mut enabled = rule.enabled;
                                        let check_width = (ui.available_width() - 178.0).max(90.0);
                                        let check = ui
                                            .allocate_ui(egui::vec2(check_width, 28.0), |ui| {
                                                ui.style_mut().wrap_mode =
                                                    Some(egui::TextWrapMode::Truncate);
                                                ui.square_checkbox(
                                                    &mut enabled,
                                                    RichText::new(&rule.name).size(13.5).strong(),
                                                )
                                            })
                                            .inner;
                                        if check.on_hover_text(&rule.name).changed() {
                                            let mut updated = rule.clone();
                                            updated.enabled = enabled;
                                            if let Err(error) =
                                                self.store.save_automation_rule(&updated)
                                            {
                                                self.automation.error = Some(error.to_string());
                                            } else {
                                                self.refresh_automation();
                                            }
                                        }
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            if ui.button(tr("Remove rule")).clicked() {
                                                if let Err(error) =
                                                    self.store.remove_automation_rule(&rule.id)
                                                {
                                                    self.automation.error = Some(error.to_string());
                                                } else {
                                                    self.refresh_automation();
                                                }
                                            }
                                            if ui.button(tr("Edit")).clicked() {
                                                self.automation.draft =
                                                    Some(Draft::from_rule(&rule));
                                                self.dialog = Some(Dialog::Automation);
                                            }
                                        });
                                    });
                                    ui.add_space(6.0);
                                    render_rule_flow(ui, &rule, &self.queues, p);
                                });
                        });
                        ui.add_space(8.0);
                    }
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, p.border))
                        .corner_radius(10)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.collapsing(tr("Use the CLI from PowerShell"), |ui| {
                                ui.label(tr("Run de-cli.exe from the application folder. Results are JSON for scripts."));
                                ui.monospace(".\\de-cli.exe --help\n.\\de-cli.exe list\n.\\de-cli.exe queue start main");
                                ui.label(
                                    RichText::new(tr("See docs/automation.md for batch downloads and automation examples."))
                                        .small()
                                        .color(p.muted),
                                );
                            });
                        });
                }
            });
        if let Some(run) = open_log {
            self.scripts.show_run(run);
            self.automation.show_scripts();
        }
    }

    pub(super) fn automation_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(draft) = &mut self.automation.draft else {
            self.dialog = None;
            return;
        };
        let mut cancel = false;
        let mut save = false;
        let response = form_dialog::modal("automation-rule", p).show(ctx, |ui| {
            cancel |= form_dialog::header(
                ui,
                tr("Automation rule"),
                tr("Choose when it runs and what happens next."),
                form_dialog::Icon::Queue,
                p,
                true,
            );
            form_dialog::body(ui, "automation-form", |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Rule name")).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.square_checkbox(&mut draft.enabled, tr("Enabled"));
                    });
                });
                ui.add(
                    view::line_edit(&mut draft.name)
                        .desired_width(f32::INFINITY)
                        .hint_text(tr("e.g. Work downloads")),
                );
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(10)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("1. When (Trigger)").small().strong().color(p.accent));
                        egui::ComboBox::from_id_salt("trigger")
                            .selected_text(if draft.chain {
                                tr("A queue completes")
                            } else {
                                tr("A download is added")
                            })
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut draft.chain, false, tr("A download is added"));
                                ui.selectable_value(&mut draft.chain, true, tr("A queue completes"));
                            });
                        ui.add_space(4.0);
                        if draft.chain {
                            ui.label(tr("Source queue"));
                            queue_picker(ui, "rule-source", &self.queues, &mut draft.source);
                        } else {
                            ui.label(tr("Site hostname · optional"));
                            ui.add(
                                view::line_edit(&mut draft.host)
                                    .desired_width(f32::INFINITY)
                                    .hint_text(tr("example.com — exact hostname")),
                            );
                            ui.label(tr("File extension · optional"));
                            ui.add(
                                view::line_edit(&mut draft.extension)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("zip"),
                            );
                            ui.label(
                                RichText::new(tr("Both conditions must match. Blank conditions match all downloads."))
                                    .small()
                                    .color(p.muted),
                            );
                        }
                    });
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(10)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("2. Then (Action)").small().strong().color(p.blue));
                        if draft.chain {
                            ui.label(tr("Then start this queue"));
                            queue_picker(ui, "rule-target", &self.queues, &mut draft.queue);
                            ui.label(
                                RichText::new(tr("Runs after every file completes successfully. Starting the destination suspends its schedule, just like Start queue."))
                                    .small()
                                    .color(p.muted),
                            );
                        } else {
                            ui.label(tr("Then send to queue"));
                            queue_picker(ui, "rule-target", &self.queues, &mut draft.queue);
                            ui.square_checkbox(&mut draft.limited, tr("Also limit this download's speed"));
                            if draft.limited {
                                ui.horizontal(|ui| {
                                    ui.add(
                                        view::line_edit(&mut draft.speed)
                                            .desired_width(100.0),
                                    );
                                    ui.label("KiB/s");
                                });
                            }
                        }
                    });
                ui.add_space(6.0);
                if let Some(error) = &draft.error {
                    ui.colored_label(p.warning, error);
                }
            });
            save = form_dialog::footer(ui, tr("Save rule"), None, p);
        });
        cancel |= response.should_close();
        if cancel {
            self.automation.draft = None;
            self.dialog = None;
        } else if save {
            match draft
                .rule()
                .and_then(|r| self.store.save_automation_rule(&r))
            {
                Ok(()) => {
                    self.automation.draft = None;
                    self.dialog = None;
                    self.refresh_automation();
                }
                Err(error) => draft.error = Some(error.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::{click, fixture, frame, text_rect};
    use super::*;

    #[test]
    fn long_history_names_leave_the_timestamp_visible() {
        let (_temp, mut app, ctx) = fixture();
        app.view = View::Automation;
        app.refresh_automation();
        app.automation.tab = Tab::History;
        let name = "A long automation rule name ".repeat(4).trim().to_owned();
        app.automation.runs = vec![Run {
            id: 1,
            at: 1_792_100_000,
            rule_name: name.clone(),
            success: false,
            message: "The destination queue was removed. Edit this rule.".into(),
        }];
        let date = chrono::DateTime::from_timestamp(app.automation.runs[0].at, 0)
            .unwrap()
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
            frame(&mut app, &ctx, size, vec![]);
            let out = frame(&mut app, &ctx, size, vec![]);
            let date_rect = text_rect(&out, &date);
            let name_rect = text_rect(&out, &name);
            assert!(
                !date_rect.intersects(name_rect),
                "History name overlaps timestamp"
            );
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(date_rect),
                "Timestamp outside window"
            );
            let shape = out.shapes.iter().find(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(t) if t.galley.job.text == date)).unwrap();
            assert!(
                shape.clip_rect.expand(1.0).contains_rect(date_rect),
                "Timestamp clipped by history card"
            );
        }
    }

    #[test]
    fn automation_editor_validates_saves_toggles_and_discards_cancelled_edits() {
        let (_temp, mut app, ctx) = fixture();
        app.view = View::Automation;
        app.automation.tab = Tab::Rules;
        click(&mut app, &ctx, "+ New rule");
        click(&mut app, &ctx, "Save rule");
        assert!(
            app.automation
                .draft
                .as_ref()
                .unwrap()
                .error
                .as_deref()
                .unwrap()
                .contains("name")
        );
        assert!(app.store.automation_rules().unwrap().is_empty());
        let draft = app.automation.draft.as_mut().unwrap();
        draft.name = "My route".into();
        draft.host = "EXAMPLE.COM".into();
        draft.extension = ".ZIP".into();
        click(&mut app, &ctx, "Save rule");
        assert!(app.dialog.is_none());
        let saved = app.store.automation_rules().unwrap();
        assert_eq!(saved.len(), 1);
        assert!(saved[0].enabled);
        assert!(
            matches!(&saved[0].workflow,Workflow::DownloadAdded {host,extension,..} if host=="example.com" && extension=="zip")
        );
        click(&mut app, &ctx, "My route");
        assert!(!app.store.automation_rules().unwrap()[0].enabled);
        click(&mut app, &ctx, "Edit");
        app.automation.draft.as_mut().unwrap().name = "Discarded".into();
        click(&mut app, &ctx, "×");
        assert!(app.dialog.is_none());
        assert_eq!(app.store.automation_rules().unwrap()[0].name, "My route");
        click(&mut app, &ctx, "Edit");
        for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
            frame(&mut app, &ctx, size, vec![]);
            let out = frame(&mut app, &ctx, size, vec![]);
            for label in ["Save rule", "×", "Enabled"] {
                assert!(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                        .contains_rect(text_rect(&out, label)),
                    "{label}"
                );
            }
        }
        frame(
            &mut app,
            &ctx,
            egui::vec2(1080.0, 800.0),
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.dialog.is_none());
        click(&mut app, &ctx, "Remove rule");
        assert!(app.store.automation_rules().unwrap().is_empty());
    }
}
