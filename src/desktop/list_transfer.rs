//! Tasks → Export list… and Import list…. File I/O and the import transaction
//! run on a worker, so a slow or removable drive never blocks the window.
use super::*;
use download_engine::list_transfer::{
    self as lists, Format, ImportOptions, Imported, ListError, MAX_ENTRIES, Problem, Review, Scope,
};
use std::{sync::mpsc, thread};

const EXPORT: &str = "\u{ede1}";
const IMPORT: &str = "\u{e8b5}";
/// Rows shown in the details list; the rest are summarised.
const DETAIL_ROWS: usize = 500;
const FOLDER_ROWS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Range {
    All,
    Queue,
    Selection,
}

/// What each range would export when the dialog opened.
struct Item {
    id: String,
    complete: bool,
    stream: bool,
}

pub(super) struct ExportDraft {
    pub range: Range,
    pub skip_complete: bool,
    pub format: Format,
    all: Vec<Item>,
    queue: Vec<Item>,
    selection: Vec<Item>,
    queue_id: String,
    queue_name: String,
}
impl ExportDraft {
    fn items(&self, range: Range) -> &[Item] {
        match range {
            Range::All => &self.all,
            Range::Queue => &self.queue,
            Range::Selection => &self.selection,
        }
    }
    /// (downloads in the list, streams left out)
    fn counts(&self, range: Range) -> (usize, usize) {
        let items = self
            .items(range)
            .iter()
            .filter(|item| !(self.skip_complete && item.complete));
        items.fold((0, 0), |(files, streams), item| {
            if item.stream {
                (files, streams + 1)
            } else {
                (files + 1, streams)
            }
        })
    }
    fn scope(&self) -> Scope {
        match self.range {
            Range::All => Scope::All,
            Range::Queue => Scope::Queue(self.queue_id.clone()),
            Range::Selection => {
                Scope::Jobs(self.selection.iter().map(|item| item.id.clone()).collect())
            }
        }
    }
}

pub(super) struct ImportDraft {
    pub path: PathBuf,
    pub review: Review,
    pub queue: String,
    pub match_queues: bool,
    pub saved_folders: bool,
    pub paused: bool,
    pub details: bool,
}

pub(super) enum Stage {
    Export(ExportDraft),
    Import(Box<ImportDraft>),
}

enum Outcome {
    Exported(anyhow::Result<(PathBuf, usize)>),
    Reviewed(anyhow::Result<(PathBuf, Review)>),
    Imported(anyhow::Result<Imported>),
}

#[derive(Default)]
pub(super) struct State {
    pub stage: Option<Stage>,
    pub error: Option<String>,
    pending: Option<mpsc::Receiver<Outcome>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl State {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
}
impl Drop for State {
    fn drop(&mut self) {
        // An accepted export or import finishes before shutdown; never abandon
        // a writer halfway through its file or transaction.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn items<'a>(jobs: impl IntoIterator<Item = &'a Job>) -> Vec<Item> {
    jobs.into_iter()
        .map(|job| Item {
            id: job.id.clone(),
            complete: job.status == Status::Complete,
            stream: lists::is_stream(job),
        })
        .collect()
}

fn list_error(error: &anyhow::Error) -> String {
    match error.downcast_ref::<ListError>() {
        Some(ListError::TooLarge) => {
            tr("This file is larger than 16 MiB. Split the list into smaller files.").into()
        }
        Some(ListError::TooManyEntries) => {
            tr("This list has more than 10,000 entries. Split it into smaller files.").into()
        }
        Some(ListError::NotUtf8) => tr("Save the list as UTF-8 text, then import it again.").into(),
        Some(ListError::NotAList) => {
            tr("This file is not a Download Engine list or a text file of links.").into()
        }
        Some(ListError::NewerVersion) => tr(
            "This list was made by a newer version of Download Engine. Update Download Engine to import it.",
        )
        .into(),
        Some(ListError::Empty) => tr("This list has no downloads.").into(),
        Some(ListError::Exists) => {
            tr("A file with this name already exists. Choose another name.").into()
        }
        None => format!("{error:#}"),
    }
}

fn problem_text(problem: &Problem) -> &str {
    match problem {
        Problem::Invalid(error) => error,
        Problem::Stream => tr("Video stream. Use Preview video to add it."),
        Problem::Checksum => tr("Invalid SHA-256 checksum"),
        Problem::Format => tr("Not in the Download Engine list format"),
        Problem::Repeated => tr("Repeated in this list"),
        Problem::Existing => tr("Already in your list"),
    }
}

impl Desktop {
    fn start_list_work(&mut self, work: impl FnOnce() -> Outcome + Send + 'static) -> bool {
        let (sender, receiver) = mpsc::channel();
        match thread::Builder::new()
            .name("list-transfer".into())
            .spawn(move || {
                let _ = sender.send(work());
            }) {
            Ok(worker) => {
                self.list_transfer.worker = Some(worker);
                self.list_transfer.pending = Some(receiver);
                true
            }
            Err(error) => {
                self.list_transfer.error = Some(tf!(
                    "Cannot start the list transfer: {error}",
                    error = error
                ));
                false
            }
        }
    }

    pub(super) fn open_list_export(&mut self) {
        if self.list_transfer.busy() || self.dialog.is_some() {
            return;
        }
        let all = match self.store.list() {
            Ok(jobs) => items(&jobs),
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot read the download list: {error}",
                    error = error
                )));
                return;
            }
        };
        let selection = items(&self.selected_jobs());
        self.list_transfer.stage = Some(Stage::Export(ExportDraft {
            range: Range::All,
            skip_complete: false,
            format: Format::Json,
            queue: items(&self.jobs),
            all,
            selection,
            queue_id: self.queue_id.clone(),
            queue_name: self.current_queue_name().to_owned(),
        }));
        self.list_transfer.error = None;
        self.dialog = Some(Dialog::ListTransfer);
    }

    pub(super) fn open_list_import(&mut self) {
        if self.list_transfer.busy() || self.dialog.is_some() {
            return;
        }
        match self.folder_picker.open_file(
            tr("Import download list"),
            &self.store.paths.downloads,
            (tr("Download lists (*.json, *.txt)"), "*.json;*.txt"),
        ) {
            Ok(Some(path)) => {
                let store = self.store.clone();
                self.list_transfer.error = None;
                if self.start_list_work(move || {
                    Outcome::Reviewed(
                        lists::read_file(&path)
                            .and_then(|parsed| store.review_import(parsed))
                            .map(|review| (path, review)),
                    )
                }) {
                    self.notice = Some(Notice::info(tr("Reading the list…").into()));
                } else if let Some(error) = self.list_transfer.error.take() {
                    self.notice = Some(Notice::error(error));
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot choose a list file: {error}",
                    error = error
                )))
            }
        }
    }

    fn export_list_now(&mut self) {
        let Some(Stage::Export(draft)) = &self.list_transfer.stage else {
            return;
        };
        if self.list_transfer.busy() {
            return;
        }
        let format = draft.format;
        let (scope, skip_complete) = (draft.scope(), draft.skip_complete);
        let name = format!(
            "DownloadEngine-list-{}.{}",
            chrono::Local::now().format("%Y-%m-%d"),
            format.extension()
        );
        let filter = match format {
            Format::Json => (tr("Download Engine list (*.json)"), "*.json"),
            Format::Text => (tr("Text file of links (*.txt)"), "*.txt"),
        };
        match self.folder_picker.save_file(
            tr("Export download list"),
            &self.store.paths.downloads,
            &name,
            filter,
            format.extension(),
        ) {
            Ok(Some(path)) => {
                // The save dialog asked before choosing an existing file. A file
                // that appears after it closed is never replaced.
                let replace = path.exists();
                let store = self.store.clone();
                self.list_transfer.error = None;
                self.start_list_work(move || {
                    Outcome::Exported((|| {
                        let collected = store.export_list(&scope, skip_complete)?;
                        let bytes = lists::render(&collected.entries, format)?;
                        lists::write_file(&path, &bytes, replace)?;
                        Ok((path, collected.entries.len()))
                    })())
                });
            }
            Ok(None) => {}
            Err(error) => {
                self.list_transfer.error = Some(tf!(
                    "Cannot choose where to save the list: {error}",
                    error = error
                ))
            }
        }
    }

    fn import_list_now(&mut self) {
        let Some(Stage::Import(draft)) = &self.list_transfer.stage else {
            return;
        };
        if self.list_transfer.busy() || draft.review.candidates.is_empty() {
            return;
        }
        let candidates = draft.review.candidates.clone();
        let options = ImportOptions {
            queue: draft.queue.clone(),
            match_queues: draft.match_queues && draft.review.matched_queues > 0,
            saved_folders: draft.saved_folders && !draft.review.folders.is_empty(),
            paused: draft.paused,
        };
        let store = self.store.clone();
        self.list_transfer.error = None;
        self.start_list_work(move || Outcome::Imported(store.import_list(&candidates, &options)));
    }

    pub(super) fn poll_list_transfer(&mut self) {
        let Some(receiver) = &self.list_transfer.pending else {
            return;
        };
        let outcome = match receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.list_transfer.pending = None;
                if let Some(worker) = self.list_transfer.worker.take() {
                    let _ = worker.join();
                }
                let message = tr("The list transfer stopped unexpectedly").to_owned();
                if self.dialog == Some(Dialog::ListTransfer) {
                    self.list_transfer.error = Some(message);
                } else {
                    self.notice = Some(Notice::error(message));
                }
                return;
            }
        };
        self.list_transfer.pending = None;
        if let Some(worker) = self.list_transfer.worker.take() {
            let _ = worker.join();
        }
        match outcome {
            Outcome::Exported(Ok((path, count))) => {
                self.close_list_transfer();
                self.notice = Some(Notice::success(tf!(
                    "Exported {} downloads to {}",
                    count,
                    path.display()
                )));
            }
            Outcome::Exported(Err(error)) => {
                self.list_transfer.error = Some(tf!(
                    "Cannot export the list: {error}",
                    error = list_error(&error)
                ))
            }
            Outcome::Reviewed(Ok((path, review))) => {
                if self
                    .notice
                    .as_ref()
                    .is_some_and(|notice| notice.message == tr("Reading the list…"))
                {
                    self.notice = None;
                }
                if self.dialog.is_some() {
                    self.notice = Some(Notice::info(
                        tr("Finish the current dialog, then import the list again.").into(),
                    ));
                    return;
                }
                let queue = if self.queues.iter().any(|q| q.id == self.queue_id) {
                    self.queue_id.clone()
                } else {
                    MAIN_QUEUE.into()
                };
                self.list_transfer.stage = Some(Stage::Import(Box::new(ImportDraft {
                    match_queues: review.matched_queues > 0,
                    saved_folders: false,
                    paused: true,
                    details: false,
                    queue,
                    path,
                    review,
                })));
                self.list_transfer.error = None;
                self.dialog = Some(Dialog::ListTransfer);
            }
            Outcome::Reviewed(Err(error)) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot import the list: {error}",
                    error = list_error(&error)
                )))
            }
            Outcome::Imported(Ok(imported)) => {
                let skipped = match &self.list_transfer.stage {
                    Some(Stage::Import(draft)) => (
                        imported.existing + draft.review.existing(),
                        draft.review.invalid(),
                    ),
                    _ => (imported.existing, 0),
                };
                self.close_list_transfer();
                if let Some(first) = imported.added.first()
                    && let Ok(Some(queue)) = self.store.job_queue(&first.id)
                {
                    self.queue_id = queue;
                    self.filter = Filter::All;
                    self.search.clear();
                    self.view = View::Downloads;
                    self.category = None;
                    self.set_selection(Some(first.id.clone()));
                }
                self.force_refresh();
                self.notice = Some(Notice::success(if skipped == (0, 0) {
                    tf!("Added {} downloads.", imported.added.len())
                } else {
                    tf!(
                        "Added {} downloads. Skipped {} already in your list and {} invalid entries.",
                        imported.added.len(),
                        skipped.0,
                        skipped.1
                    )
                }));
            }
            Outcome::Imported(Err(error)) => {
                self.list_transfer.error = Some(tf!(
                    "Cannot add the downloads: {error}",
                    error = list_error(&error)
                ))
            }
        }
    }

    /// Screenshot fixtures: the export form, or the review of a sample list with
    /// new, existing, invalid and stream entries. Nothing is written or added.
    #[cfg(feature = "screenshot")]
    pub(super) fn list_transfer_sample(&mut self, mode: &str) {
        if mode == "list-export" {
            self.open_list_export();
            return;
        }
        let downloads = self.store.paths.downloads.display().to_string();
        let document = serde_json::json!({"format": lists::FORMAT, "version": lists::VERSION, "downloads": [
            {"url": "https://example.invalid/Project-assets.zip"},
            {"url": "https://example.invalid/Studio-installer-2.4.exe", "folder": downloads,
             "queue": "Main queue", "sha256": "ab".repeat(32)},
            {"url": "https://example.invalid/Field-recordings.flac", "folder": downloads, "queue": "Main queue"},
            {"url": "https://example.invalid/Lecture-03.mp4", "folder": "C:\\Download Engine sample\\Video",
             "queue": "Evening classes"},
            {"url": "https://example.invalid/Handbook.pdf", "queue": "Evening classes"},
            {"url": "https://example.invalid/Product-reference.pdf"},
            {"url": "file:///C:/Users/Public/notes.txt"},
            {"url": "https://example.invalid/live/master.m3u8"},
        ]});
        match lists::parse(document.to_string().as_bytes())
            .and_then(|parsed| self.store.review_import(parsed))
        {
            Ok(review) => {
                self.list_transfer.stage = Some(Stage::Import(Box::new(ImportDraft {
                    path: PathBuf::from("DownloadEngine-list-2026-10-20.json"),
                    match_queues: review.matched_queues > 0,
                    saved_folders: mode == "list-import-details",
                    paused: true,
                    details: mode == "list-import-details",
                    queue: MAIN_QUEUE.into(),
                    review,
                })));
                self.dialog = Some(Dialog::ListTransfer);
            }
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
    }

    fn close_list_transfer(&mut self) {
        self.list_transfer.stage = None;
        self.list_transfer.error = None;
        if self.dialog == Some(Dialog::ListTransfer) {
            self.dialog = None;
        }
    }

    pub(super) fn list_transfer_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let busy = self.list_transfer.busy();
        let mut submit = false;
        let mut cancel = false;
        let export = matches!(self.list_transfer.stage, Some(Stage::Export(_)));
        let response = form_dialog::modal("list-transfer-dialog", p).show(ctx, |ui| {
            match &mut self.list_transfer.stage {
                Some(Stage::Export(draft)) => {
                    cancel = form_dialog::header(
                        ui,
                        tr("Export list"),
                        tr("Save your downloads to back them up or move them to another PC."),
                        form_dialog::Icon::Symbol(EXPORT),
                        p,
                        !busy,
                    );
                    scroll(ui, "list-export-body", |ui| {
                        ui.add_enabled_ui(!busy, |ui| export_form(ui, draft, p));
                    });
                }
                Some(Stage::Import(draft)) => {
                    let file = draft
                        .path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    cancel = form_dialog::header(
                        ui,
                        tr("Import list"),
                        &file,
                        form_dialog::Icon::Symbol(IMPORT),
                        p,
                        !busy,
                    );
                    scroll(ui, "list-import-body", |ui| {
                        import_summary(ui, draft, p);
                        if !draft.review.candidates.is_empty() {
                            ui.add_enabled_ui(!busy, |ui| import_form(ui, draft, &self.queues, p));
                        }
                    });
                }
                None => cancel = true,
            }
            if let Some(error) = &self.list_transfer.error {
                ui.add_space(4.0);
                ui.label(RichText::new(error).small().color(p.warning));
            }
            submit = form_dialog::controls(ui, |ui| match &self.list_transfer.stage {
                Some(Stage::Export(draft)) => {
                    let (count, _) = draft.counts(draft.range);
                    let clicked = form_dialog::primary_enabled(
                        ui,
                        !busy && (1..=MAX_ENTRIES).contains(&count),
                        tr("Export…"),
                    )
                    .clicked();
                    if busy {
                        ui.label(RichText::new(tr("Exporting…")).small().color(p.muted));
                    }
                    clicked
                }
                Some(Stage::Import(draft)) => {
                    let count = draft.review.candidates.len();
                    let clicked = form_dialog::primary_enabled(
                        ui,
                        !busy && count > 0,
                        &tf!("Add {} downloads", count),
                    )
                    .clicked();
                    if busy {
                        ui.label(RichText::new(tr("Adding…")).small().color(p.muted));
                    }
                    clicked
                }
                None => false,
            });
        });
        if (cancel || response.should_close()) && !busy {
            self.close_list_transfer();
        } else if submit {
            if export {
                self.export_list_now();
            } else {
                self.import_list_now();
            }
        }
    }
}

/// Keep the footer visible; long details scroll inside the dialog.
fn scroll(ui: &mut egui::Ui, id: &'static str, content: impl FnOnce(&mut egui::Ui)) {
    let height = (ui.ctx().content_rect().height() - 220.0).max(150.0);
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(height)
        .show(ui, content);
}

fn section(ui: &mut egui::Ui, p: Palette, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(p.background)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            // Compact rows keep both forms within the minimum window height.
            ui.spacing_mut().interact_size.y = 22.0;
            ui.spacing_mut().item_spacing.y = 3.0;
            content(ui);
        });
}

/// A control with its explanation on the same line, wrapping when needed.
/// The explanation never takes clicks meant for the control.
fn hinted(
    ui: &mut egui::Ui,
    hint: &str,
    p: Palette,
    control: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> egui::Response {
    ui.horizontal_wrapped(|ui| {
        let response = control(ui);
        if !hint.is_empty() {
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(hint).small().color(p.muted)).selectable(false));
        }
        response
    })
    .inner
}

fn choice(
    ui: &mut egui::Ui,
    enabled: bool,
    selected: bool,
    label: &str,
    hint: &str,
    p: Palette,
) -> bool {
    hinted(ui, hint, p, |ui| {
        ui.add_enabled(enabled, egui::RadioButton::new(selected, label))
    })
    .clicked()
}

fn export_form(ui: &mut egui::Ui, draft: &mut ExportDraft, p: Palette) {
    section(ui, p, |ui| {
        ui.label(RichText::new(tr("Downloads")).strong());
        let (all, _) = draft.counts(Range::All);
        let (queue, _) = draft.counts(Range::Queue);
        let (selection, _) = draft.counts(Range::Selection);
        if ui
            .radio(draft.range == Range::All, tf!("All downloads ({})", all))
            .clicked()
        {
            draft.range = Range::All;
        }
        if ui
            .radio(
                draft.range == Range::Queue,
                tf!("Current queue: {} ({})", draft.queue_name, queue),
            )
            .clicked()
        {
            draft.range = Range::Queue;
        }
        if ui
            .add_enabled(
                !draft.selection.is_empty(),
                egui::RadioButton::new(
                    draft.range == Range::Selection,
                    tf!("Selected downloads ({})", selection),
                ),
            )
            .clicked()
        {
            draft.range = Range::Selection;
        }
        // A gap keeps the checkbox apart from the range choices above it.
        ui.add_space(4.0);
        ui.square_checkbox(
            &mut draft.skip_complete,
            tr("Leave out completed downloads"),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(tr("Format")).strong());
        if choice(
            ui,
            true,
            draft.format == Format::Json,
            tr("Download Engine list (.json)"),
            tr("Names, links, folders, queues, file type and expected SHA-256."),
            p,
        ) {
            draft.format = Format::Json;
        }
        if choice(
            ui,
            true,
            draft.format == Format::Text,
            tr("Links only (.txt)"),
            tr("One link per line. Works with Add batch."),
            p,
        ) {
            draft.format = Format::Text;
        }
    });
    ui.add_space(6.0);
    let note = |ui: &mut egui::Ui, text: &str, color| {
        ui.add(egui::Label::new(RichText::new(text).small().color(color)).wrap());
    };
    note(
        ui,
        tr(
            "Passwords, cookies, browser logins, proxy settings and partial download data are never exported. Links are saved as they are; some sites put temporary access keys in links.",
        ),
        p.muted,
    );
    let (count, streams) = draft.counts(draft.range);
    if streams > 0 {
        note(
            ui,
            &tf!(
                "Video streams left out: {}. Use Preview video to add them again.",
                streams
            ),
            p.muted,
        );
    }
    if count == 0 {
        note(ui, tr("There are no downloads to export here."), p.warning);
    } else if count > MAX_ENTRIES {
        note(
            ui,
            tr("Export at most 10,000 downloads at a time. Choose a queue or a selection."),
            p.warning,
        );
    }
}

fn import_summary(ui: &mut egui::Ui, draft: &mut ImportDraft, p: Palette) {
    let review = &draft.review;
    ui.horizontal_wrapped(|ui| {
        let chip = |ui: &mut egui::Ui, text: String, color: Color32| {
            egui::Frame::new()
                .fill(color.gamma_multiply(0.14))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(8, 3))
                .show(ui, |ui| ui.label(RichText::new(text).small().color(color)));
        };
        chip(ui, tf!("{} new", review.candidates.len()), p.success);
        if review.existing() > 0 {
            chip(
                ui,
                tf!("{} already in your list", review.existing()),
                p.muted,
            );
        }
        if review.invalid() > 0 {
            chip(ui, tf!("{} invalid", review.invalid()), p.warning);
        }
    });
    // On its own line, so long translations never wrap the link across rows.
    if !review.rejected.is_empty() {
        let label = if draft.details {
            tr("Hide details")
        } else {
            tr("Show details")
        };
        if ui.link(label).clicked() {
            draft.details = !draft.details;
        }
    }
    if draft.details {
        ui.add_space(4.0);
        section(ui, p, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("list-import-details")
                .max_height(140.0)
                .show(ui, |ui| {
                    for rejected in review.rejected.iter().take(DETAIL_ROWS) {
                        let place = if review.format == Format::Json {
                            tf!("Entry {}", rejected.number)
                        } else {
                            tf!("Line {}", rejected.number)
                        };
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!(
                                    "{place} · {} · {}",
                                    problem_text(&rejected.problem),
                                    rejected.excerpt
                                ))
                                .small()
                                .color(p.muted),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&rejected.excerpt);
                    }
                    if review.rejected.len() > DETAIL_ROWS {
                        ui.label(
                            RichText::new(tf!("and {} more", review.rejected.len() - DETAIL_ROWS))
                                .small()
                                .color(p.muted),
                        );
                    }
                });
        });
    }
    if review.candidates.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new(tr("Nothing new to add.")).color(p.muted));
    }
}

fn import_form(ui: &mut egui::Ui, draft: &mut ImportDraft, queues: &[DownloadQueue], p: Palette) {
    ui.add_space(6.0);
    section(ui, p, |ui| {
        ui.label(RichText::new(tr("Queue")).strong());
        ui.horizontal(|ui| {
            ui.label(tr("Add to"));
            let selected = queues
                .iter()
                .find(|queue| queue.id == draft.queue)
                .map(|queue| display_queue_name(&queue.id, &queue.name))
                .unwrap_or(tr("Main queue"))
                .to_owned();
            egui::ComboBox::from_id_salt("list-import-queue")
                .width(220.0)
                .selected_text(short_queue_name(&selected))
                .show_ui(ui, |ui| {
                    for queue in queues {
                        ui.selectable_value(
                            &mut draft.queue,
                            queue.id.clone(),
                            display_queue_name(&queue.id, &queue.name),
                        );
                    }
                })
                .response
                .on_hover_text(selected);
        });
        let review = &draft.review;
        if review.queue_names > 0 {
            let label = tf!(
                "Use matching queue names from the list ({} of {} found)",
                review.matched_queues,
                review.queue_names
            );
            hinted(ui, tr("Other downloads go to the queue above."), p, |ui| {
                ui.add_enabled(
                    review.matched_queues > 0,
                    view::Checkbox::new(&mut draft.match_queues, label),
                )
            });
        }
        if review.format == Format::Json
            && (!review.folders.is_empty() || review.missing_folders > 0)
        {
            ui.add_space(6.0);
            ui.label(RichText::new(tr("Save files to")).strong());
            if choice(
                ui,
                true,
                !draft.saved_folders,
                tr("My download folders"),
                tr("Sorted by file type, as in Preferences."),
                p,
            ) {
                draft.saved_folders = false;
            }
            let label = tf!(
                "Folders from the list that exist on this PC ({})",
                review.folders.len()
            );
            if choice(
                ui,
                !review.folders.is_empty(),
                draft.saved_folders,
                &label,
                "",
                p,
            ) {
                draft.saved_folders = true;
            }
            if draft.saved_folders {
                for folder in review.folders.iter().take(FOLDER_ROWS) {
                    let path = folder.to_string_lossy();
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.add(
                            egui::Label::new(RichText::new(path.as_ref()).small().color(p.muted))
                                .truncate(),
                        )
                        .on_hover_text(path.as_ref());
                    });
                }
                if review.folders.len() > FOLDER_ROWS {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.label(
                            RichText::new(tf!("and {} more", review.folders.len() - FOLDER_ROWS))
                                .small()
                                .color(p.muted),
                        );
                    });
                }
            }
            if review.missing_folders > 0 {
                ui.add(
                    egui::Label::new(
                        RichText::new(tf!(
                            "Folders not on this PC: {}. Their files go to your download folders.",
                            review.missing_folders
                        ))
                        .small()
                        .color(p.muted),
                    )
                    .wrap(),
                );
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new(tr("After adding")).strong());
        if choice(
            ui,
            true,
            draft.paused,
            tr("Keep paused"),
            tr("Nothing starts until you resume it."),
            p,
        ) {
            draft.paused = true;
        }
        if choice(
            ui,
            true,
            !draft.paused,
            tr("Queue them"),
            tr("They start when their queue runs, like Add batch."),
            p,
        ) {
            draft.paused = false;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::{click, fixture, frame};
    use super::*;
    use std::time::{Duration, Instant};

    /// Replace the in-memory fixture rows with real rows in the isolated store.
    fn stored(app: &mut Desktop, names: &[&str]) {
        let jobs: Vec<_> = names
            .iter()
            .map(|name| {
                app.store
                    .new_download(
                        &format!("https://example.com/{name}"),
                        None,
                        None,
                        Uuid::new_v4().to_string(),
                    )
                    .unwrap()
            })
            .collect();
        app.store.enqueue_batch(&jobs, MAIN_QUEUE).unwrap();
        app.force_refresh();
    }
    fn finish(app: &mut Desktop) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.list_transfer.busy() {
            app.poll_list_transfer();
            assert!(
                Instant::now() < deadline,
                "the list transfer did not finish"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn message(app: &Desktop) -> String {
        app.notice
            .as_ref()
            .map(|notice| notice.message.clone())
            .unwrap_or_default()
    }
    fn review(app: &Desktop) -> &Review {
        match &app.list_transfer.stage {
            Some(Stage::Import(draft)) => &draft.review,
            _ => panic!("the import review is open"),
        }
    }

    #[test]
    fn tasks_menu_exports_and_imports_lists_in_an_isolated_store() {
        let (temp, mut app, ctx) = fixture();
        stored(&mut app, &["first.zip", "guide.pdf", "notes.txt"]);
        click(&mut app, &ctx, "Tasks");
        click(&mut app, &ctx, "Export list…");
        assert_eq!(app.dialog, Some(Dialog::ListTransfer));
        click(&mut app, &ctx, "Links only (.txt)");
        let links = temp.path().join("links.txt");
        app.folder_picker.response = Some(Ok(Some(links.display().to_string())));
        click(&mut app, &ctx, "Export…");
        finish(&mut app);
        assert_eq!(app.dialog, None);
        assert!(
            message(&app).contains("Exported 3 downloads"),
            "{}",
            message(&app)
        );
        let text = std::fs::read_to_string(&links).unwrap();
        assert_eq!(text.lines().count(), 3);
        assert!(
            text.lines()
                .all(|line| line.starts_with("https://example.com/"))
        );

        // Cancelling the save dialog keeps the export form open and writes nothing.
        click(&mut app, &ctx, "Tasks");
        click(&mut app, &ctx, "Export list…");
        app.folder_picker.response = Some(Ok(None));
        click(&mut app, &ctx, "Export…");
        assert!(!app.list_transfer.busy());
        assert_eq!(app.dialog, Some(Dialog::ListTransfer));
        let list = temp.path().join("list.json");
        app.folder_picker.response = Some(Ok(Some(list.display().to_string())));
        click(&mut app, &ctx, "Export…");
        finish(&mut app);
        let document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&list).unwrap()).unwrap();
        assert_eq!(document["downloads"].as_array().unwrap().len(), 3);

        // The same list adds nothing; Escape closes the review.
        app.folder_picker.response = Some(Ok(Some(list.display().to_string())));
        click(&mut app, &ctx, "Tasks");
        click(&mut app, &ctx, "Import list…");
        finish(&mut app);
        assert_eq!(app.dialog, Some(Dialog::ListTransfer));
        assert!(review(&app).candidates.is_empty());
        assert_eq!(review(&app).existing(), 3);
        let size = egui::vec2(1080.0, 800.0);
        frame(&mut app, &ctx, size, vec![]);
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(app.dialog, None);
        assert!(app.list_transfer.stage.is_none());

        // New, repeated and invalid links: review the details, then add paused.
        let incoming = temp.path().join("incoming.txt");
        std::fs::write(
            &incoming,
            "https://example.com/new-1.zip\nhttps://example.com/first.zip\nnot a link\nhttps://example.com/new-2.pdf\n",
        )
        .unwrap();
        app.folder_picker.response = Some(Ok(Some(incoming.display().to_string())));
        click(&mut app, &ctx, "Tasks");
        click(&mut app, &ctx, "Import list…");
        finish(&mut app);
        click(&mut app, &ctx, "Show details");
        assert!(matches!(&app.list_transfer.stage, Some(Stage::Import(draft)) if draft.details));
        click(&mut app, &ctx, "Add 2 downloads");
        finish(&mut app);
        assert_eq!(app.dialog, None);
        assert_eq!(
            message(&app),
            "Added 2 downloads. Skipped 1 already in your list and 1 invalid entries."
        );
        let jobs = app.store.list().unwrap();
        assert_eq!(jobs.len(), 5);
        for name in ["new-1.zip", "new-2.pdf"] {
            let job = jobs.iter().find(|job| job.name == name).unwrap();
            assert_eq!(job.status, Status::Paused);
            assert_eq!(
                app.store.job_queue(&job.id).unwrap().as_deref(),
                Some(MAIN_QUEUE)
            );
        }
        assert_eq!(app.view, View::Downloads);
    }

    #[test]
    fn unreadable_lists_explain_the_problem_and_busy_work_blocks_new_transfers() {
        let (temp, mut app, ctx) = fixture();
        let broken = temp.path().join("broken.json");
        std::fs::write(&broken, "{\"format\": \"something else\"}").unwrap();
        app.folder_picker.response = Some(Ok(Some(broken.display().to_string())));
        click(&mut app, &ctx, "Tasks");
        click(&mut app, &ctx, "Import list…");
        finish(&mut app);
        assert_eq!(app.dialog, None);
        assert_eq!(
            message(&app),
            "Cannot import the list: This file is not a Download Engine list or a text file of links."
        );

        stored(&mut app, &["first.zip"]);
        app.open_list_export();
        let list = temp.path().join("list.json");
        std::fs::write(&list, "keep").unwrap();
        // Choosing an existing file means Windows already asked to replace it.
        app.folder_picker.response = Some(Ok(Some(list.display().to_string())));
        app.export_list_now();
        assert!(app.list_transfer.busy());
        app.folder_picker.response = Some(Ok(Some(list.display().to_string())));
        app.open_list_import();
        assert!(
            app.folder_picker.response.take().is_some(),
            "no file dialog opens while a list is being written"
        );
        finish(&mut app);
        assert!(
            std::fs::read_to_string(&list)
                .unwrap()
                .contains(lists::FORMAT)
        );

        // An export error stays in the dialog so the user can choose again.
        app.open_list_export();
        app.folder_picker.response = Some(Ok(Some(
            temp.path()
                .join("absent")
                .join("list.json")
                .display()
                .to_string(),
        )));
        app.export_list_now();
        finish(&mut app);
        assert_eq!(app.dialog, Some(Dialog::ListTransfer));
        assert!(
            app.list_transfer
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("Cannot export the list:"))
        );
    }
}
