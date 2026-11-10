//! Plain words for what an extension's jobs do and what they can reach, shared
//! by the Overview tab and the install review, and the dialog that grants file
//! access after the install.
use super::*;
use download_engine::extensions::Job as PackageJob;

/// One job as the page describes it.
pub(super) struct JobLine<'a> {
    pub(super) action: bool,
    pub(super) name: &'a str,
    pub(super) trigger: &'a Trigger,
    /// The last run, on the Overview tab only.
    pub(super) last: Option<String>,
}

/// When a job runs, as a sentence. A queue completed job before its install has
/// no queue yet: the review asks for one below.
pub(super) fn runs(trigger: &Trigger, queues: &[DownloadQueue]) -> String {
    match trigger {
        Trigger::SiteReader { .. } => tr("Runs when you open one of these pages:").into(),
        Trigger::Provider => tr("Runs when you add a download with it.").into(),
        Trigger::DownloadCompleted { queue: None } => tr("Runs when any download finishes.").into(),
        Trigger::DownloadFailed { queue: None } => tr("Runs when any download fails.").into(),
        Trigger::QueueCompleted { queue } if queue.is_empty() => {
            tr("Runs when the queue you choose finishes.").into()
        }
        Trigger::QueueCompleted { queue } => tf!(
            "Runs when {queue} finishes.",
            queue = automation::queue_label(queues, queue)
        ),
        Trigger::Startup => tr("Runs each time Download Engine starts.").into(),
        Trigger::Interval { minutes: 1 } => tr("Runs every minute.").into(),
        Trigger::Interval { minutes } => tf!("Runs every {minutes} minutes.", minutes = minutes),
        Trigger::Manual => tr("Runs only when you start it.").into(),
        // Packages can't name a queue for these, so only scripts reach here.
        other => scripts::trigger_name(other).into(),
    }
}

/// A one-line summary for the list of installed extensions.
pub(super) fn summary(jobs: &[Job], queues: &[DownloadQueue]) -> String {
    match jobs {
        [job] => match &job.script.trigger {
            Trigger::SiteReader { pages } => {
                let site = pages
                    .first()
                    .and_then(|page| page.split('/').next())
                    .unwrap_or_default()
                    .trim_start_matches("*.");
                tf!("Reads {site} pages", site = site)
            }
            trigger => runs(trigger, queues),
        },
        jobs => job_count(jobs.len()),
    }
}

/// The kind of job, and what that kind may do, for its hover text.
fn kind(trigger: &Trigger) -> (&'static str, &'static str) {
    match trigger {
        Trigger::SiteReader { .. } => (
            tr("Site reader"),
            tr(
                "Reads a page you open and lists the files on it. It reaches only the sites it names, and can't see or change your downloads.",
            ),
        ),
        Trigger::Provider => (
            tr("Download provider"),
            tr("Handles downloads you add with it. It reaches only the sites it names."),
        ),
        _ => (
            tr("Automation"),
            tr(
                "Runs after something happens in Download Engine, such as a download finishing. It sees and controls downloads, but can't read web pages.",
            ),
        ),
    }
}

/// A small label in a tinted box. Callers add hover text where it explains more.
/// It is measured before it is placed, so in a wrapping row it moves to the next
/// line when it doesn't fit, instead of squeezing or pushing past the edge.
fn chip(ui: &mut egui::Ui, text: &str, ink: Color32) -> egui::Response {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, ink);
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(12.0, 4.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, 5, ink.gamma_multiply(0.14));
        ui.painter().galley(rect.min + vec2(6.0, 2.0), galley, ink);
    }
    response
}

pub(super) fn what_it_does(
    ui: &mut egui::Ui,
    jobs: &[JobLine],
    queues: &[DownloadQueue],
    p: Palette,
) {
    group(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        heading(ui, tr("What it does"), p);
        for (index, job) in jobs.iter().enumerate() {
            if index > 0 {
                ui.add_space(6.0);
            }
            let (kind, help) = kind(job.trigger);
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::Label::new(RichText::new(job.name).strong()).wrap());
                chip(ui, kind, p.muted).on_hover_text(help);
                if let Some(last) = &job.last {
                    ui.label(RichText::new(last).small().color(p.muted));
                }
            });
            let explanation = if job.action {
                tr("Runs only when you start it.").to_owned()
            } else {
                runs(job.trigger, queues)
            };
            ui.add(egui::Label::new(RichText::new(explanation).color(p.muted)).wrap());
            if !job.action
                && let Trigger::SiteReader { pages } = job.trigger
            {
                for page in pages {
                    ui.add(
                        egui::Label::new(RichText::new(page).monospace().small().color(p.muted))
                            .wrap(),
                    );
                }
            }
        }
    });
}

/// The width of the titles column, so every language's longest title fits.
fn title_width(ui: &egui::Ui) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    [tr("The web"), tr("Your downloads"), tr("Your files")]
        .into_iter()
        .map(|title| {
            ui.painter()
                .layout_no_wrap(title.to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0, f32::max)
        + 12.0
}

/// One row of the Access group: an icon, a title and what the content says.
fn row(
    ui: &mut egui::Ui,
    glyph: &str,
    title: &str,
    width: f32,
    p: Palette,
    content: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        view::icon(ui, rect.center(), glyph, 15.0, p.muted);
        ui.allocate_ui_with_layout(vec2(width, 18.0), Layout::left_to_right(Align::Min), |ui| {
            ui.set_width(width);
            ui.label(RichText::new(title).strong());
        });
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
    });
}

/// What the web row says: the hosts the package declares, read or added from.
pub(super) fn web(package: &Package) -> Vec<String> {
    let hosts = package.permissions.network.join(", ");
    if hosts.is_empty() {
        return vec![tr("None. It can't read pages or reach the internet.").into()];
    }
    let mut lines = Vec::new();
    if package.permissions.http_sessions && package.jobs.iter().any(|job| !job.is_automation()) {
        lines.push(tr("Use login cookies and headers you supply, on the declared hosts").into());
    }
    if package.jobs.iter().any(|job| !job.is_automation()) {
        lines.push(tf!("Reads {hosts}", hosts = hosts));
    }
    if package.jobs.iter().any(PackageJob::is_automation) {
        lines.push(tf!("Adds downloads only from {hosts}", hosts = hosts));
    }
    lines
}

/// What the downloads row says. A provider handles the downloads added with it,
/// down to their bytes, so it is never described as unable to touch them.
fn downloads(package: &Package) -> Vec<&'static str> {
    let mut lines = Vec::new();
    if package.jobs.iter().any(PackageJob::is_automation) {
        lines.push(tr("Sees their names, folders and status, never their addresses. Can pause, resume and retry them, manage queues and show notifications."));
    }
    if package
        .jobs
        .iter()
        .any(|job| job.trigger == Trigger::Provider)
    {
        lines.push(tr(
            "Handles only the downloads you add with it, including their contents.",
        ));
    }
    if lines.is_empty() {
        lines.push(tr("Can't see or change them."));
    }
    lines
}

/// The Access group. `files` draws the files row, which differs between the
/// review (boxes to tick) and the Overview tab (what was allowed).
pub(super) fn access(
    ui: &mut egui::Ui,
    package: &Package,
    p: Palette,
    files: impl FnOnce(&mut egui::Ui),
) {
    group(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        heading(ui, tr("Access"), p);
        ui.add_space(2.0);
        let width = title_width(ui);
        row(ui, "\u{e774}", tr("The web"), width, p, |ui| {
            for line in web(package) {
                ui.add(egui::Label::new(line).wrap());
            }
        });
        ui.add_space(4.0);
        row(ui, "\u{e896}", tr("Your downloads"), width, p, |ui| {
            for line in downloads(package) {
                ui.add(egui::Label::new(line).wrap());
            }
        });
        ui.add_space(4.0);
        row(ui, "folder", tr("Your files"), width, p, files);
        ui.add_space(6.0);
        let writes: BTreeSet<_> = package.jobs.iter().flat_map(|j| &j.secret_writes).collect();
        for key in writes {
            if let Some(setting) = package.setting(key) {
                ui.label(format!(
                    "{}: {}",
                    tr("May save protected account settings"),
                    setting.label()
                ));
            }
        }
        if package
            .settings
            .iter()
            .any(|s| matches!(s, Setting::AesKey { .. }))
        {
            ui.add(
                egui::Label::new(
                    RichText::new(tr("Use a key you save in Settings")).color(p.muted),
                )
                .wrap(),
            );
        }
        ui.add(
            egui::Label::new(
                RichText::new(tr(
                    "Never: your browser logins or cookies, or other programs.",
                ))
                .small()
                .color(p.muted),
            )
            .wrap(),
        );
    });
}

/// The jobs that ask to change files.
pub(super) fn asking(package: &Package) -> Vec<&PackageJob> {
    package.jobs.iter().filter(|job| job.change_files).collect()
}

fn asks(job: &PackageJob, alone: bool) -> String {
    if alone {
        tr("It asks to rename, move and delete downloaded files.").into()
    } else {
        tf!(
            "{job} asks to rename, move and delete downloaded files.",
            job = job.name
        )
    }
}

/// The files row on the Overview tab: what was allowed, with a button to change it.
pub(super) fn overview_files(ui: &mut egui::Ui, entry: &Entry, p: Palette) -> Option<Act> {
    let package = &entry.installed.package;
    let jobs = asking(package);
    if jobs.is_empty() {
        ui.add(
            egui::Label::new(
                RichText::new(tr("Can't rename, move or delete them.")).color(p.muted),
            )
            .wrap(),
        );
        return None;
    }
    let mut act = None;
    for job in &jobs {
        let allowed = entry
            .jobs
            .iter()
            .find(|line| {
                line.script
                    .extension
                    .as_ref()
                    .is_some_and(|l| l.job == job.id)
            })
            .is_some_and(|line| line.script.change_files);
        let action = if allowed {
            tr("Take back")
        } else {
            tr("Allow…")
        };
        let font = egui::TextStyle::Button.resolve(ui.style());
        let button = ui
            .painter()
            .layout_no_wrap(action.to_owned(), font, p.text)
            .size()
            .x
            + 2.0 * ui.spacing().button_padding.x
            + ui.spacing().item_spacing.x;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - button).max(120.0));
                if allowed && jobs.len() == 1 {
                    ui.add(egui::Label::new(tr("Rename, move and delete any downloaded file. You allowed this.")).wrap());
                } else if allowed {
                    // Several jobs ask: each row says which one it is about.
                    ui.add(egui::Label::new(tf!("{job} can rename, move and delete any downloaded file. You allowed this.", job = job.name)).wrap());
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::Label::new(asks(job, jobs.len() == 1)).wrap());
                        chip(ui, tr("Not allowed"), p.orange);
                    });
                    ui.add(
                        egui::Label::new(
                            RichText::new(tr("Without this, each move fails and the run log says why."))
                                .small()
                                .color(p.muted),
                        )
                        .wrap(),
                    );
                }
            });
            if ui.button(action).clicked() {
                let id = package.id.clone();
                act = Some(if allowed {
                    Act::TakeBackFiles(id, job.id.clone())
                } else {
                    Act::AllowFiles(id, job.id.clone())
                });
            }
        });
    }
    act
}

/// The files row in an install review: a box per job that asks.
pub(super) fn review_files(
    ui: &mut egui::Ui,
    pending: &mut Pending,
    package: &Package,
    p: Palette,
) {
    let jobs = asking(package);
    if jobs.is_empty() {
        ui.add(
            egui::Label::new(
                RichText::new(tr("Can't rename, move or delete them.")).color(p.muted),
            )
            .wrap(),
        );
        return;
    }
    for job in &jobs {
        let mut allowed = pending.file_access.contains(&job.id);
        let label = if jobs.len() == 1 {
            tr("Let it rename, move and delete any downloaded file").into()
        } else {
            tf!(
                "Let {job} rename, move and delete any downloaded file",
                job = job.name
            )
        };
        if ui.square_checkbox(&mut allowed, label).changed() {
            if allowed {
                pending.file_access.insert(job.id.clone());
            } else {
                pending.file_access.remove(&job.id);
            }
        }
    }
    ui.add(
        egui::Label::new(
            RichText::new(tr(
                "It asks for this. Leave it clear and its moves fail. You can allow it later on the Overview tab.",
            ))
            .small()
            .color(p.muted),
        )
        .wrap(),
    );
}

impl Desktop {
    /// Asks before letting a job change files after its extension is installed.
    pub(in crate::desktop) fn extension_file_access_dialog(
        &mut self,
        ctx: &egui::Context,
        p: Palette,
    ) {
        let Some((id, job)) = self.extensions.granting.clone() else {
            self.dialog = None;
            return;
        };
        let names = self.store.extension(&id).ok().and_then(|installed| {
            let job = installed.package.job(&job)?.name.clone();
            Some((installed.package.name, job))
        });
        let Some((name, job_name)) = names else {
            self.extensions.granting = None;
            self.dialog = None;
            return;
        };
        let mut close = false;
        let mut allow = false;
        let response = form_dialog::modal("extension-file-access", p).show(ctx, |ui| {
            ui.set_width(460.0);
            close = form_dialog::title(
                ui,
                &tf!("Allow {name} to change files?", name = name),
                form_dialog::Icon::Symbol("folder"),
                p,
                true,
            );
            ui.add_space(8.0);
            ui.add(
                egui::Label::new(tf!(
                    "{job} will be able to rename, move and delete any downloaded file, not only the ones it added.",
                    job = job_name
                ))
                .wrap(),
            );
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(tr(
                        "You can take this back on the Overview tab at any time. An update that asks for more access still needs your approval.",
                    ))
                    .color(p.muted),
                )
                .wrap(),
            );
            form_dialog::controls(ui, |ui| {
                allow = form_dialog::primary_button(ui, tr("Allow")).clicked();
            });
        });
        close |= response.should_close();
        if allow {
            if let Err(error) = self.store.set_extension_file_access(&id, &job, true) {
                self.extensions.error = Some(format!("{error:#}"));
            }
            self.refresh_extensions();
        }
        if allow || close {
            self.extensions.granting = None;
            self.dialog = None;
        }
    }
}
