//! Generic controls for explicit plugin web actions. Service code stays in Lua.
use super::*;
use download_engine::{extensions::ActionInput, script_automation::Reading, store::Store};

pub(super) struct Report {
    pub run: Run,
    pub page: String,
    pub logs: String,
    pub reading: Option<Reading>,
}

pub(super) fn reports(
    store: &Store,
    entries: &[Entry],
) -> anyhow::Result<BTreeMap<String, Report>> {
    if !entries
        .iter()
        .any(|e| e.installed.package.jobs.iter().any(|j| j.action.is_some()))
    {
        return Ok(BTreeMap::new());
    }
    let runs = store.script_runs(None)?; // Database insertion order breaks same-second ties.
    entries
        .iter()
        .filter_map(|entry| {
            let ids: BTreeSet<_> = entry
                .jobs
                .iter()
                .filter(|j| {
                    j.script
                        .extension
                        .as_ref()
                        .and_then(|l| entry.installed.package.job(&l.job))
                        .is_some_and(|j| j.action.is_some())
                })
                .map(|j| j.script.id.as_str())
                .collect();
            let run = runs
                .iter()
                .find(|r| ids.contains(r.script_id.as_str()))?
                .clone();
            let logs = match store.script_logs(&run.id) {
                Ok(logs) => logs,
                Err(e) => return Some(Err(e)),
            };
            let logs = logs
                .iter()
                .rev()
                .take(8)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let reading = if run.state == "succeeded" {
                match store.script_result(&run.id) {
                    Ok(result) => result,
                    Err(e) => return Some(Err(e)),
                }
            } else {
                None
            };
            let page = run.context["url"]
                .as_str()
                .unwrap_or("https://example.com/")
                .to_owned();
            Some(Ok((
                entry.id().to_owned(),
                Report {
                    run,
                    page,
                    logs,
                    reading,
                },
            )))
        })
        .collect()
}

pub(super) fn card(ui: &mut egui::Ui, state: &mut State, index: usize, p: Palette) -> Option<Act> {
    let entry = &state.entries[index];
    let package = &entry.installed.package;
    if !package.jobs.iter().any(|j| j.action.is_some()) {
        return None;
    }
    let busy = entry
        .jobs
        .iter()
        .filter_map(|j| j.last.as_ref())
        .find(|r| r.active());
    let mut act = None;
    ui.add_space(8.0);
    group(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        heading(ui, tr("Plugin actions"), p);
        if !entry.installed.enabled {
            ui.label(RichText::new(tr("Turn on this plugin to use its actions.")).color(p.muted));
        }
        ui.horizontal_wrapped(|ui| {
            for job in package
                .jobs
                .iter()
                .filter(|j| j.action == Some(ActionInput::None))
            {
                if ui
                    .add_enabled(
                        entry.installed.enabled && busy.is_none(),
                        egui::Button::new(&job.name),
                    )
                    .clicked()
                    && let Some(script) = entry
                        .jobs
                        .iter()
                        .find(|j| j.script.extension.as_ref().is_some_and(|l| l.job == job.id))
                        .map(|j| j.script.clone())
                {
                    let Trigger::SiteReader { pages } = &job.trigger else {
                        continue;
                    };
                    let host = pages[0]
                        .split('/')
                        .next()
                        .unwrap_or_default()
                        .trim_start_matches("*.");
                    act = Some(Act::RunAction(script, format!("https://{host}/")));
                }
            }
        });
        if let Some(homepage) = &package.homepage {
            ui.hyperlink_to(tr("Open service website"), homepage);
        }
        if package
            .jobs
            .iter()
            .any(|j| j.action == Some(ActionInput::Url))
        {
            ui.add_space(8.0);
            ui.add(
                view::line_edit(&mut state.source_url)
                    .hint_text(tr("Source URL"))
                    .desired_width(ui.available_width())
                    .char_limit(16384),
            );
            ui.horizontal_wrapped(|ui| {
                for job in package
                    .jobs
                    .iter()
                    .filter(|j| j.action == Some(ActionInput::Url))
                {
                    if ui
                        .add_enabled(
                            entry.installed.enabled
                                && busy.is_none()
                                && !state.source_url.trim().is_empty(),
                            egui::Button::new(&job.name),
                        )
                        .clicked()
                        && let Some(script) = entry
                            .jobs
                            .iter()
                            .find(|j| j.script.extension.as_ref().is_some_and(|l| l.job == job.id))
                    {
                        act = Some(Act::RunAction(
                            script.script.clone(),
                            state.source_url.trim().to_owned(),
                        ));
                    }
                }
            });
        }
        if let Some(run) = busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(&run.name);
                if ui.button(tr("Stop")).clicked() {
                    act = Some(Act::StopAction(run.id.clone()));
                }
            });
        }
        if let Some(report) = state.action_reports.get(entry.id()) {
            if !report.logs.is_empty() {
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(RichText::new(&report.logs).small())
                        .selectable(true)
                        .wrap(),
                );
            }
            if let Some(reading) = &report.reading {
                if let Some(title) = &reading.title {
                    ui.add(egui::Label::new(RichText::new(title).color(p.accent)).wrap());
                }
                if !reading.items.is_empty() && ui.button(tr("Review generated links")).clicked() {
                    act = Some(Act::ReviewAction(entry.id().to_owned()));
                }
            } else if let Some(error) = &report.run.error {
                ui.add(egui::Label::new(RichText::new(error).color(p.warning)).wrap());
            } else if !report.run.active() {
                ui.label(&report.run.state);
            }
        }
    });
    act
}
