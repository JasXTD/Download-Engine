//! The extensions folder: packed extension files saved in the data folder's
//! Extensions folder, listed in a dialog and counted on a strip on the Installed
//! tab. Listing runs nothing. Review… opens the same review as Install from
//! file… for the file as it is then, and a new extension installs turned off.
//! It is listed when Download Engine starts, each time the Installed tab opens,
//! when the list opens, and on Scan now. Nothing watches it in the background.
use super::{
    examples::{Offer, package_row},
    *,
};
use anyhow::Context as _;
use download_engine::extensions::{
    folder::{self, Kind, Listing, MAX_LISTED},
    version,
};

/// What the folder held when it was last listed.
#[derive(Default)]
pub(in crate::desktop) struct Folder {
    task: Option<background_io::Task<Listing>>,
    /// List it again when the running listing ends: the folder may have
    /// changed after that one started.
    again: bool,
    listing: Option<Listing>,
    /// Why the folder couldn't be listed. A later listing that works clears it.
    error: Option<String>,
    /// Why Review… or Open folder failed. It stays until the next action, even
    /// though a new listing starts right after.
    problem: Option<String>,
    /// When the last listing finished, as a Unix time.
    looked: Option<i64>,
    /// The frame the Installed tab was last drawn in, to tell when it opens.
    drawn: Option<u64>,
    #[cfg(test)]
    pub(super) opened: Option<std::path::PathBuf>,
}

impl Folder {
    fn finish(&mut self, result: anyhow::Result<Listing>) {
        self.looked = Some(chrono::Utc::now().timestamp());
        match result {
            Ok(listing) => {
                self.listing = Some(listing);
                self.error = None;
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }

    /// Extensions with a file that would install them or update them. Two files
    /// of one extension count once.
    fn to_review(&self, entries: &[Entry]) -> usize {
        self.listing
            .iter()
            .flat_map(|listing| &listing.found)
            .filter_map(|found| match &found.kind {
                Kind::Package(package)
                    if matches!(status(package, entries), Status::New | Status::Update(_)) =>
                {
                    Some(package.id.as_str())
                }
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .len()
    }
}

/// How a file in the folder compares with what is installed.
enum Status<'a> {
    New,
    /// Newer than the installed version, which is given.
    Update(&'a str),
    /// The same package as the installed one.
    Installed,
    /// Not the installed package, and not newer, so it can't update it.
    NotNewer(&'a str),
}

fn status<'a>(package: &Package, entries: &'a [Entry]) -> Status<'a> {
    let Some(installed) = entries
        .iter()
        .map(|entry| &entry.installed.package)
        .find(|installed| installed.id == package.id)
    else {
        return Status::New;
    };
    let newer = matches!(
        (version(&package.version), version(&installed.version)),
        (Ok(new), Ok(old)) if new > old
    );
    if newer {
        Status::Update(&installed.version)
    } else if installed == package {
        Status::Installed
    } else {
        Status::NotNewer(&installed.version)
    }
}

/// A Unix time as the local hour and minute.
fn clock(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .map(|time| time.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

impl Desktop {
    /// Lists the folder in the background. It is created first, so Open folder
    /// and the path in the guide always lead somewhere.
    pub(in crate::desktop) fn scan_extension_folder(&mut self) {
        let state = &mut self.extensions.folder;
        if state.task.is_some() {
            state.again = true;
            return;
        }
        let path = self.store.paths.extensions();
        match background_io::Task::spawn(self.background_io.context.clone(), move || {
            std::fs::create_dir_all(&path)
                .with_context(|| format!("Can't create {}", path.display()))?;
            folder::scan(&path)
        }) {
            Ok(task) => state.task = Some(task),
            Err(error) => state.error = Some(format!("{error:#}")),
        }
    }

    pub(in crate::desktop) fn poll_extension_folder(&mut self) {
        let state = &mut self.extensions.folder;
        let Some(result) = state.task.as_mut().and_then(|task| task.poll()) else {
            return;
        };
        state.task = None;
        state.finish(result);
        if std::mem::take(&mut state.again) {
            self.scan_extension_folder();
        }
    }

    #[cfg(test)]
    pub(in crate::desktop) fn extension_folder_pending(&self) -> bool {
        self.extensions.folder.task.is_some()
    }

    /// Lists the folder now, on this thread, for screenshots.
    #[cfg(feature = "screenshot")]
    pub(super) fn list_extension_folder_now(&mut self) {
        let path = self.store.paths.extensions();
        self.extensions.folder.finish(folder::scan(&path));
    }

    /// The Installed tab calls this each frame. Coming back to it, from another
    /// page or tab, lists the folder again.
    pub(super) fn extension_folder_strip(&mut self, ui: &mut egui::Ui, p: Palette) {
        let frame = ui.ctx().cumulative_frame_nr();
        let state = &mut self.extensions.folder;
        let opened = state.drawn.is_none_or(|drawn| frame > drawn + 1);
        state.drawn = Some(frame);
        if opened {
            self.scan_extension_folder();
        }
        self.poll_extension_folder();
        let count = self.extensions.folder.to_review(&self.extensions.entries);
        if count == 0 {
            return;
        }
        let text = if count == 1 {
            tr("Your extensions folder has 1 extension to review.").to_owned()
        } else {
            tf!(
                "Your extensions folder has {count} extensions to review.",
                count = count
            )
        };
        let action = tr("Review…");
        let mut review = false;
        group(p)
            .stroke(Stroke::new(1.0, p.blue.gamma_multiply(0.6)))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let font = egui::TextStyle::Button.resolve(ui.style());
                let button = ui
                    .painter()
                    .layout_no_wrap(action.to_owned(), font, p.text)
                    .size()
                    .x
                    + 2.0 * ui.spacing().button_padding.x
                    + ui.spacing().item_spacing.x;
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                    view::icon(ui, rect.center(), "folder", 18.0, p.blue);
                    ui.vertical(|ui| {
                        ui.set_width((ui.available_width() - button).max(120.0));
                        ui.add(egui::Label::new(text).wrap());
                    });
                    review = ui.button(action).clicked();
                });
            });
        ui.add_space(6.0);
        if review {
            self.open_extension_folder();
        }
    }

    pub(in crate::desktop) fn open_extension_folder(&mut self) {
        self.extensions.error = None;
        self.extensions.folder.problem = None;
        self.scan_extension_folder();
        self.dialog = Some(Dialog::ExtensionFolder);
    }

    fn show_extension_folder(&mut self, path: std::path::PathBuf) {
        if let Err(error) = std::fs::create_dir_all(&path) {
            self.extensions.folder.problem = Some(format!("{error:#}"));
            return;
        }
        #[cfg(test)]
        {
            self.extensions.folder.opened = Some(path);
        }
        #[cfg(not(test))]
        self.open_folder_async(path);
    }

    pub(in crate::desktop) fn extension_folder_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        self.poll_extension_folder();
        let path = self.store.paths.extensions();
        let state = &self.extensions.folder;
        let entries = &self.extensions.entries;
        let mut close = false;
        let mut chosen = None;
        let (mut show, mut scan, mut guide) = (false, false, false);
        let response = form_dialog::modal("extension-folder", p).show(ctx, |ui| {
            close |= form_dialog::header(
                ui,
                tr("Your extensions folder"),
                tr("Drop .de-extension.json files here to review and install them"),
                form_dialog::Icon::Symbol("folder"),
                p,
                true,
            );
            form_dialog::body(ui, "extension-folder-body", |ui| {
                egui::Frame::new()
                    .fill(p.track)
                    .corner_radius(5)
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(
                            egui::Label::new(RichText::new(path.display().to_string()).monospace())
                                .wrap(),
                        );
                    });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    show = ui.button(tr("Open folder")).clicked();
                    scan = ui
                        .add_enabled(state.task.is_none(), egui::Button::new(tr("Scan now")))
                        .clicked();
                    if state.task.is_some() {
                        ui.spinner();
                        ui.label(RichText::new(tr("Looking in the folder…")).small().color(p.muted));
                    }
                });
                ui.add_space(4.0);
                if let Some(at) = state.looked {
                    let looked = tf!("Looked at {time}. It also looks when Download Engine starts and each time you open Extensions.", time = clock(at));
                    ui.add(egui::Label::new(RichText::new(looked).small().color(p.muted)).wrap());
                }
                let promise = tr("Nothing in this folder runs until you review it, and it installs turned off.");
                ui.add(egui::Label::new(RichText::new(promise).small().color(p.muted)).wrap());
                for error in [&state.problem, &state.error].into_iter().flatten() {
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(RichText::new(error).small().color(p.warning)).wrap());
                }
                if let Some(listing) = &state.listing {
                    if listing.found.is_empty() {
                        ui.add_space(6.0);
                        ui.separator();
                        ui.label(RichText::new(tr("No extension files in this folder yet.")).color(p.muted));
                    }
                    for found in &listing.found {
                        ui.add_space(6.0);
                        ui.separator();
                        match &found.kind {
                            Kind::Package(package) => {
                                let title = format!("{} · {}", package.version, package.publisher);
                                let (notes, offer) = match status(package, entries) {
                                    Status::New => (Vec::new(), Offer::Button(tr("Review…"))),
                                    Status::Update(from) => (
                                        vec![(
                                            tf!("You have {version}. This file is newer.", version = from),
                                            p.muted,
                                        )],
                                        Offer::Button(tr("Review update…")),
                                    ),
                                    Status::Installed => (Vec::new(), Offer::Note(tr("Installed"))),
                                    Status::NotNewer(installed) => {
                                        let same = version(installed).ok() == version(&package.version).ok();
                                        let note = if same {
                                            tf!("Differs from the {version} you installed. Raise its version to update.", version = installed)
                                        } else {
                                            tf!("Older than the {version} you installed.", version = installed)
                                        };
                                        (vec![(note, p.orange)], Offer::Nothing)
                                    }
                                };
                                if package_row(ui, package, Some(&title), &notes, offer, p) {
                                    chosen = Some(found.name.clone());
                                }
                            }
                            Kind::Unreadable(why) => {
                                ui.add(egui::Label::new(RichText::new(&found.name).strong()).wrap());
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(tf!("Can't read this file: {error}", error = why))
                                            .small()
                                            .color(p.orange),
                                    )
                                    .wrap(),
                                );
                            }
                            Kind::Unpacked => {
                                let name = format!("{}{}", found.name, std::path::MAIN_SEPARATOR);
                                ui.add(egui::Label::new(RichText::new(name).strong()).wrap());
                                let hint = tr("Folders aren't read. Pack it into a file with de-cli extensions pack first.");
                                ui.add(egui::Label::new(RichText::new(hint).small().color(p.muted)).wrap());
                            }
                        }
                    }
                    ui.add_space(6.0);
                    ui.separator();
                    if listing.skipped == 1 {
                        ui.label(RichText::new(tr("Skipped 1 other file.")).small().color(p.muted));
                    } else if listing.skipped > 1 {
                        ui.label(
                            RichText::new(tf!("Skipped {count} other files.", count = listing.skipped))
                                .small()
                                .color(p.muted),
                        );
                    }
                    if listing.left_out > 0 {
                        let first = tf!("Only the first {count} extension files are listed.", count = MAX_LISTED);
                        ui.label(RichText::new(first).small().color(p.orange));
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(tr("Deleting a file here doesn't uninstall anything."))
                            .small()
                            .color(p.muted),
                    );
                    guide = ui.button(tr("Extension guide")).clicked();
                });
            });
        });
        close |= response.should_close();
        if chosen.is_some() || show || scan {
            self.extensions.folder.problem = None;
        }
        if let Some(name) = chosen {
            // Read again: what is reviewed, and installed, is the file as it is now.
            let result = folder::read(&path, &name).and_then(|(_, bytes)| {
                let review = self.store.review_extension(&bytes)?;
                Ok((bytes, review))
            });
            match result {
                // The review replaces this list, as if the file had been picked.
                Ok((bytes, review)) => self.review_extension(bytes, review),
                Err(error) => {
                    let error = folder::short(&format!("{name}: {error:#}"));
                    self.extensions.folder.problem =
                        Some(tf!("Can't read this file: {error}", error = error));
                    self.scan_extension_folder();
                }
            }
        } else if show {
            self.show_extension_folder(path);
        } else if scan {
            self.scan_extension_folder();
        } else if guide {
            self.dialog = None;
            self.open_guide("Extensions");
        } else if close {
            self.dialog = None;
        }
    }
}
