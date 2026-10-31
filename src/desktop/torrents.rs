//! Torrents in the desktop window: the question before the first one, and choosing files.
use super::*;
use download_engine::torrent::{self, Details, File, PROXY_MESSAGE, Settings, Stage};
use egui::{Align, Layout, RichText, vec2};
use std::{collections::HashSet, ops::Range, path::Path};
use tokio_util::sync::CancellationToken;

const ROW_HEIGHT: f32 = 24.0;
const LIST_HEIGHT: f32 = 168.0;
const MIN_LIST_HEIGHT: f32 = 96.0;
/// What the rest of the Add download window needs, so that its buttons always stay on screen.
const ADD_WINDOW_HEIGHT: f32 = 432.0;
const WARNING_HEIGHT: f32 = 64.0;
const NOTE_HEIGHT: f32 = 20.0;
const SIZE_WIDTH: f32 = 72.0;
const FOLDER_INDENT: f32 = 16.0;

/// A magnet link, or a .torrent file on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Source {
    Magnet(String),
    File(PathBuf),
}
impl Source {
    pub(super) fn detect(text: &str) -> Option<Self> {
        let text = text.trim().trim_matches('"');
        if torrent::is_magnet(text) {
            return Some(Self::Magnet(text.to_owned()));
        }
        let path = Path::new(text);
        (path.is_absolute()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent")))
        .then(|| Self::File(path.to_owned()))
    }
}

struct Row {
    depth: usize,
    name: String,
    files: Range<usize>,
    size: u64,
}

/// A torrent's files as a tree of ticks. Every row covers a run of `files`, because sorting
/// by path keeps each folder's files together.
pub(super) struct Picker {
    files: Vec<File>,
    rows: Vec<Row>,
    chosen: Vec<bool>,
}
impl Picker {
    pub(super) fn new(mut files: Vec<File>, chosen: Option<&[usize]>) -> Self {
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let mut rows: Vec<Row> = Vec::new();
        let mut open: Vec<(String, usize)> = Vec::new();
        for (position, file) in files.iter().enumerate() {
            let parts: Vec<&str> = file.path.split('/').collect();
            let (name, folders) = parts.split_last().expect("split gives at least one part");
            let shared = open
                .iter()
                .zip(folders)
                .take_while(|((name, _), folder)| name == *folder)
                .count();
            open.truncate(shared);
            for folder in &folders[shared..] {
                rows.push(Row {
                    depth: open.len(),
                    name: (*folder).into(),
                    files: position..position,
                    size: 0,
                });
                open.push(((*folder).into(), rows.len() - 1));
            }
            for (_, row) in &open {
                rows[*row].files.end = position + 1;
                rows[*row].size += file.len;
            }
            rows.push(Row {
                depth: open.len(),
                name: (*name).into(),
                files: position..position + 1,
                size: file.len,
            });
        }
        let wanted: Option<HashSet<usize>> = chosen.map(|list| list.iter().copied().collect());
        let chosen = files
            .iter()
            .map(|file| {
                wanted
                    .as_ref()
                    .is_none_or(|list| list.contains(&file.index))
            })
            .collect();
        Self {
            files,
            rows,
            chosen,
        }
    }

    fn chosen_files(&self) -> impl Iterator<Item = &File> {
        self.files
            .iter()
            .zip(&self.chosen)
            .filter(|(_, chosen)| **chosen)
            .map(|(file, _)| file)
    }

    pub(super) fn chosen_indices(&self) -> Vec<usize> {
        self.chosen_files().map(|file| file.index).collect()
    }

    pub(super) fn chosen_size(&self) -> u64 {
        self.chosen_files().map(|file| file.len).sum()
    }

    fn set_all(&mut self, value: bool) {
        self.chosen.fill(value);
    }

    fn renamed(&self) -> usize {
        self.files
            .iter()
            .filter(|file| {
                let saved: Vec<_> = file
                    .saved_as
                    .iter()
                    .map(|part| part.to_string_lossy())
                    .collect();
                saved.join("/") != file.path
            })
            .count()
    }

    fn row(&mut self, ui: &mut egui::Ui, p: Palette, index: usize) {
        let row = &self.rows[index];
        let files = row.files.clone();
        let picked = self.chosen[files.clone()]
            .iter()
            .filter(|chosen| **chosen)
            .count();
        let mut all = picked == files.len();
        let mut clicked = false;
        ui.horizontal(|ui| {
            ui.set_height(ROW_HEIGHT);
            ui.add_space(row.depth as f32 * FOLDER_INDENT);
            clicked = ui
                .add(
                    view::Checkbox::new(&mut all, "")
                        .indeterminate(picked > 0 && picked < files.len()),
                )
                .clicked();
            let width = (ui.available_width() - SIZE_WIDTH).max(40.0);
            ui.allocate_ui_with_layout(
                vec2(width, ROW_HEIGHT),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.add(
                        egui::Label::new(row.name.as_str())
                            .truncate()
                            .selectable(false),
                    )
                    .on_hover_text(row.name.as_str());
                },
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(bytes(row.size)).small().color(p.muted));
            });
        });
        if clicked {
            self.chosen[files].fill(all);
        }
    }

    fn list(&mut self, ui: &mut egui::Ui, p: Palette, height: f32) {
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, p.border))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(8, 2))
            .show(ui, |ui| {
                let spacing = ui.spacing().item_spacing.y;
                egui::ScrollArea::vertical()
                    .id_salt("torrent-files")
                    .max_height(height)
                    .auto_shrink([false, true])
                    .show_rows(
                        ui,
                        ROW_HEIGHT + spacing - spacing,
                        self.rows.len(),
                        |ui, range| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for index in range {
                                self.row(ui, p, index);
                            }
                        },
                    );
            });
    }

    fn select_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.small_button(tr("Select all")).clicked() {
                self.set_all(true);
            }
            if ui.small_button(tr("Select none")).clicked() {
                self.set_all(false);
            }
        });
    }

    fn summary(&self) -> String {
        tf!(
            "{chosen} of {total} files · {size} selected",
            chosen = self.chosen_files().count(),
            total = self.files.len(),
            size = bytes(self.chosen_size())
        )
    }
}

enum Fetched {
    Details(Details),
    /// This torrent is already in the list, as this download.
    Existing(String),
    Cancelled,
}

struct Draft {
    source: Source,
    asked: bool,
    cancel: CancellationToken,
    fetch: Option<background_io::Task<Fetched>>,
    started: Instant,
    details: Option<Details>,
    picker: Option<Picker>,
    existing: Option<String>,
    error: Option<String>,
    free: Option<u64>,
    free_for: Option<PathBuf>,
    free_task: Option<background_io::Task<(PathBuf, Option<u64>)>>,
}
impl Draft {
    fn new(source: Source) -> Self {
        Self {
            source,
            asked: false,
            cancel: CancellationToken::new(),
            fetch: None,
            started: Instant::now(),
            details: None,
            picker: None,
            existing: None,
            error: None,
            free: None,
            free_for: None,
            free_task: None,
        }
    }
    fn is_waiting(&self) -> bool {
        self.fetch.is_some()
    }
    fn is_settled(&self) -> bool {
        self.details.is_some() || self.existing.is_some() || self.error.is_some()
    }
}
/// Dropping the fetch waits for its thread, so the search has to end first. Otherwise a
/// file list that never arrives would hold up closing the app.
impl Drop for Draft {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

enum Question {
    AddWindow,
    Resume(String),
}

struct Choice {
    id: String,
    name: String,
    picker: Picker,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) settings: Settings,
    question: Option<Question>,
    draft: Option<Draft>,
    /// Cancelled fetches. Dropping one waits for its thread, which must not happen in a frame.
    leaving: Vec<background_io::Task<Fetched>>,
    saving: Option<background_io::Task<Job>>,
    choice: Option<Choice>,
}

impl State {
    pub(super) fn new(settings: Settings) -> Self {
        Self {
            settings,
            ..Default::default()
        }
    }
}

impl Desktop {
    /// Stops a torrent that can't start yet from reaching the scheduler. True if it did.
    pub(super) fn torrent_needs_attention(&mut self, job: &Job) -> bool {
        let Some(state) = &job.torrent else {
            return false;
        };
        if !self.torrents.settings.enabled {
            self.torrents.question = Some(Question::Resume(job.id.clone()));
            return true;
        }
        if state.stage == Stage::Choosing {
            self.open_torrent_choice(job);
            return true;
        }
        false
    }

    pub(super) fn open_torrent_file(&mut self) {
        if self.dialog.is_some() {
            return;
        }
        match self.folder_picker.open_file(
            tr("Open torrent file…"),
            &self.store.paths.downloads,
            (tr("Torrent files (*.torrent)"), "*.torrent"),
        ) {
            Ok(Some(path)) => self.open_torrent_path(&path),
            Ok(None) => {}
            Err(error) => self.notice = Some(Notice::error(format!("{error:#}"))),
        }
    }

    /// Opens Add download on a .torrent file that is already on this computer.
    pub(super) fn open_torrent_path(&mut self, path: &Path) {
        self.url = path.to_string_lossy().into_owned();
        self.open_add();
    }

    pub(super) fn open_torrent_choice(&mut self, job: &Job) {
        let files = match self.store.torrent_file_list(&job.id) {
            Ok(files) => files,
            Err(error) => {
                self.notice = Some(Notice::error(format!("{error:#}")));
                return;
            }
        };
        let chosen = (!files.chosen.is_empty()).then_some(files.chosen.as_slice());
        let picker = Picker::new(files.list.clone(), chosen);
        self.torrents.choice = Some(Choice {
            id: job.id.clone(),
            name: job.name.clone(),
            picker,
        });
        self.dialog = Some(Dialog::TorrentChoice);
    }

    fn turn_on_torrents(&mut self) -> bool {
        let settings = Settings {
            enabled: true,
            ..self.torrents.settings
        };
        match self.store.save_torrent_settings(settings) {
            Ok(()) => {
                self.torrents.settings = settings;
                self.apply_torrent_settings();
                true
            }
            Err(error) => {
                self.notice = Some(Notice::error(format!("{error:#}")));
                false
            }
        }
    }

    pub(super) fn apply_torrent_settings(&mut self) {
        if let Err(error) = self.scheduler.send(Command::TorrentSettings) {
            self.notice = Some(Notice::error(error.to_string()));
        }
    }

    /// Asks before torrents go on. Nothing here has read a file or connected to anyone yet.
    pub(super) fn torrent_question_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        if self.torrents.question.is_none() {
            return;
        }
        let (mut yes, mut no) = (false, false);
        let response = form_dialog::modal("torrent-question", p).show(ctx, |ui| {
            ui.set_width(440.0);
            no = form_dialog::title(
                ui,
                tr("Turn on torrents?"),
                form_dialog::Icon::Symbol("\u{e716}"),
                p,
                true,
            );
            ui.add_space(4.0);
            for line in [
                tr("Your computer connects directly to other people. They can see your IP address."),
                tr("While it downloads, your computer also uploads pieces to them."),
                tr("Security apps may show blocked addresses while a torrent runs."),
                tr("Only download files you have the right to use."),
            ] {
                ui.label(format!("• {line}"));
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(tr("You can turn torrents off again in Preferences."))
                    .small()
                    .color(p.muted),
            );
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                yes = form_dialog::primary_enabled(ui, true, tr("Turn on torrents")).clicked();
                no |= form_dialog::secondary_button(ui, tr("Not now")).clicked();
            });
        });
        if yes {
            let question = self.torrents.question.take();
            if self.turn_on_torrents()
                && let Some(Question::Resume(id)) = question
            {
                self.command(Command::Resume(id));
            }
        } else if no || response.should_close() {
            self.torrents.question = None;
        }
    }

    pub(super) fn forget_torrent_draft(&mut self) {
        if let Some(mut draft) = self.torrents.draft.take() {
            draft.cancel.cancel();
            self.torrents.leaving.extend(draft.fetch.take());
        }
    }

    /// Lets go of cancelled fetches once their threads have finished.
    pub(super) fn poll_torrent_leftovers(&mut self) {
        self.torrents
            .leaving
            .retain_mut(|task| task.poll().is_none());
    }

    fn start_torrent_fetch(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.torrents.draft else {
            return;
        };
        let store = self.store.clone();
        let source = draft.source.clone();
        let cancel = draft.cancel.clone();
        draft.started = Instant::now();
        let task = background_io::Task::spawn(Some(ctx.clone()), move || {
            let details = match &source {
                Source::Magnet(link) => {
                    let hash = torrent::info_hash(link)?;
                    if let Some(job) = store.torrent_job(&hash)? {
                        return Ok(Fetched::Existing(job.id));
                    }
                    store.fetch_torrent_details(link, &cancel)?
                }
                Source::File(path) => Some(store.read_torrent_file(path)?),
            };
            let Some(details) = details else {
                return Ok(Fetched::Cancelled);
            };
            Ok(match store.torrent_job(&details.info_hash)? {
                Some(job) => Fetched::Existing(job.id),
                None => Fetched::Details(details),
            })
        });
        match task {
            Ok(task) => draft.fetch = Some(task),
            Err(error) => draft.error = Some(format!("{error:#}")),
        }
    }

    fn sync_torrent_draft(&mut self, ctx: &egui::Context, source: &Source) {
        if self
            .torrents
            .draft
            .as_ref()
            .is_none_or(|draft| draft.source != *source)
        {
            self.forget_torrent_draft();
            self.torrents.draft = Some(Draft::new(source.clone()));
        }
        let enabled = self.torrents.settings.enabled;
        let saving = self.torrents.saving.is_some();
        let Some(draft) = &mut self.torrents.draft else {
            return;
        };
        if let Some(result) = draft.fetch.as_mut().and_then(background_io::Task::poll) {
            draft.fetch = None;
            match result {
                Ok(Fetched::Details(details)) => {
                    draft.picker = Some(Picker::new(details.files.clone(), None));
                    draft.details = Some(details);
                }
                Ok(Fetched::Existing(id)) => draft.existing = Some(id),
                Ok(Fetched::Cancelled) => {}
                Err(error) => draft.error = Some(format!("{error:#}")),
            }
        }
        if !enabled {
            if !draft.asked {
                draft.asked = true;
                self.torrents.question = Some(Question::AddWindow);
            }
        } else if !saving && !draft.is_waiting() && !draft.is_settled() {
            self.start_torrent_fetch(ctx);
        }
    }

    fn sync_free_space(&mut self, ctx: &egui::Context) {
        let folder = self
            .folder_override()
            .unwrap_or_else(|| self.store.paths.downloads.clone());
        let Some(draft) = &mut self.torrents.draft else {
            return;
        };
        if let Some(result) = draft.free_task.as_mut().and_then(background_io::Task::poll) {
            draft.free_task = None;
            if let Ok((path, free)) = result
                && path == folder
            {
                draft.free = free;
            }
        }
        if draft.free_task.is_none() && draft.free_for.as_ref() != Some(&folder) {
            draft.free_for = Some(folder.clone());
            draft.free = None;
            // A network folder can take seconds to answer, which a frame must not wait for.
            draft.free_task = background_io::Task::spawn(Some(ctx.clone()), move || {
                let free = torrent::free_space(&folder);
                Ok((folder, free))
            })
            .ok();
        }
    }

    fn submit_torrent(&mut self, ctx: &egui::Context) {
        let store = self.store.clone();
        let queue = self.add_queue_id().to_owned();
        let folder = self.folder_override();
        let request = Uuid::new_v4().to_string();
        let Some(draft) = &mut self.torrents.draft else {
            return;
        };
        let link = match &draft.source {
            Source::Magnet(link) => Some(link.clone()),
            Source::File(_) => None,
        };
        let task = match (&draft.details, &draft.picker, link) {
            (Some(details), Some(picker), _) => {
                let details = details.clone();
                let chosen = picker.chosen_indices();
                background_io::Task::spawn(Some(ctx.clone()), move || {
                    store.add_torrent(&details, chosen, folder.as_deref(), &queue, request)
                })
            }
            (_, _, Some(link)) if draft.error.is_none() && draft.existing.is_none() => {
                // The search must end first, or the new download could start while the
                // helper still holds this torrent under the search's name.
                draft.cancel.cancel();
                draft.cancel = CancellationToken::new();
                let search = draft.fetch.take();
                background_io::Task::spawn(Some(ctx.clone()), move || {
                    drop(search);
                    store.add_magnet_for_later(&link, folder.as_deref(), &queue, request)
                })
            }
            _ => return,
        };
        match task {
            Ok(task) => self.torrents.saving = Some(task),
            Err(error) => self.form_error = Some(format!("{error:#}")),
        }
    }

    fn poll_torrent_save(&mut self) {
        let Some(result) = self
            .torrents
            .saving
            .as_mut()
            .and_then(background_io::Task::poll)
        else {
            return;
        };
        self.torrents.saving = None;
        match result {
            Ok(job) => {
                self.forget_torrent_draft();
                self.finish_add_download(job);
                self.force_refresh();
            }
            Err(error) => self.form_error = Some(format!("{error:#}")),
        }
    }

    fn warning(ui: &mut egui::Ui, p: Palette, title: &str, text: &str) {
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, p.warning))
            .corner_radius(6)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(title).strong());
                if !text.is_empty() {
                    ui.label(RichText::new(text).small().color(p.muted));
                }
            });
    }

    /// Add download once the link is a torrent. It's the same window, showing files instead of
    /// the file name and login fields.
    pub(super) fn add_torrent_dialog(&mut self, ctx: &egui::Context, p: Palette, source: Source) {
        self.poll_torrent_save();
        if self.dialog != Some(Dialog::Add) {
            return;
        }
        self.sync_torrent_draft(ctx, &source);
        self.sync_free_space(ctx);
        let saving = self.torrents.saving.is_some();
        let focused = ctx.memory(|memory| memory.focused());
        let enabled = self.torrents.settings.enabled;
        let mut editing = false;
        let (mut cancel, mut submit, mut turn_on, mut proxy) = (false, false, false, false);
        let mut show = None;
        let wide = self
            .torrents
            .draft
            .as_ref()
            .is_some_and(|draft| draft.picker.is_some());
        let response = form_dialog::modal("add-download", p).show(ctx, |ui| {
            ui.set_width(if wide { 600.0 } else { 460.0 });
            cancel = form_dialog::header(
                ui,
                tr("Add download"),
                tr("Paste a file link, magnet link or torrent file."),
                form_dialog::Icon::Download,
                p,
                !saving,
            );
            ui.add_enabled_ui(!saving, |ui| {
                ui.spacing_mut().interact_size.y = view::CONTROL_HEIGHT;
                ui.label(
                    RichText::new(match source {
                        Source::Magnet(_) => tr("Magnet link"),
                        Source::File(_) => tr("Torrent file"),
                    })
                    .small()
                    .color(p.muted),
                );
                let input = ui.add(
                    view::line_edit(&mut self.url)
                        .id(egui::Id::new("add-file-url"))
                        .desired_width(f32::INFINITY)
                        .vertical_align(Align::Center),
                );
                editing |= focused == Some(input.id);
                ui.add_space(8.0);
                let Some(draft) = &mut self.torrents.draft else {
                    return;
                };
                if !enabled {
                    Self::warning(
                        ui,
                        p,
                        tr("Torrents are off"),
                        tr("Turn them on to add this. Nothing has connected yet."),
                    );
                } else if let Some(id) = &draft.existing {
                    Self::warning(ui, p, tr("Already in your list"), "");
                    if ui.button(tr("Show it")).clicked() {
                        show = Some(id.clone());
                    }
                } else if let Some(error) = &draft.error {
                    if error.contains(PROXY_MESSAGE) {
                        Self::warning(ui, p, tr("Torrents can't start while a proxy is on"), tr(PROXY_MESSAGE));
                        proxy = ui.button(tr("Proxy settings")).clicked();
                    } else {
                        Self::warning(ui, p, tr("This torrent can't be added"), error);
                    }
                } else if let (Some(details), Some(picker)) = (&draft.details, &mut draft.picker) {
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(RichText::new(&details.name).strong()).truncate())
                            .on_hover_text(&details.name);
                    });
                    picker.select_buttons(ui);
                    let needed = picker.chosen_size();
                    let short_of_room = draft.free.is_some_and(|free| needed > free);
                    let warned = short_of_room || picker.chosen_indices().is_empty();
                    let renamed = picker.renamed();
                    let height = ctx.content_rect().height()
                        - ADD_WINDOW_HEIGHT
                        - if warned { WARNING_HEIGHT } else { 0.0 }
                        - if renamed > 0 { NOTE_HEIGHT } else { 0.0 };
                    picker.list(ui, p, height.clamp(MIN_LIST_HEIGHT, LIST_HEIGHT));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(picker.summary()).small().color(p.muted));
                        if let Some(free) = draft.free {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let free_text = tf!("{size} free", size = bytes(free));
                                let color = if needed > free { p.warning } else { p.muted };
                                ui.label(RichText::new(free_text).small().color(color));
                            });
                        }
                    });
                    if picker.chosen_indices().is_empty() {
                        Self::warning(ui, p, tr("Select at least one file."), "");
                    } else if short_of_room {
                        Self::warning(
                            ui,
                            p,
                            &tf!(
                                "This needs {needed}, but the folder has {free} free.",
                                needed = bytes(needed),
                                free = bytes(draft.free.unwrap_or(0))
                            ),
                            tr("Change folder or select fewer files."),
                        );
                    }
                    if renamed > 0 {
                        ui.label(
                            RichText::new(tf!(
                                "{count} file names were changed so Windows can save them.",
                                count = renamed
                            ))
                            .small()
                            .color(p.muted),
                        );
                    }
                } else {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        let seconds = draft.started.elapsed().as_secs();
                        ui.label(format!(
                            "{} {}:{:02}",
                            tr("Getting file list…"),
                            seconds / 60,
                            seconds % 60
                        ));
                    });
                    ui.label(
                        RichText::new(tr("The file list comes from other people, so it can take a while. You can leave and choose later."))
                            .small()
                            .color(p.muted),
                    );
                }
                ui.add_space(8.0);
                ui.columns(2, |columns| {
                    columns[0].label(RichText::new(tr("Queue")).small().color(p.muted));
                    let selected = self
                        .queues
                        .iter()
                        .find(|queue| queue.id == self.add_queue.as_deref().unwrap_or(&self.queue_id))
                        .map(|queue| display_queue_name(&queue.id, &queue.name))
                        .unwrap_or(tr("Main queue"))
                        .to_owned();
                    egui::ComboBox::from_id_salt("add-queue")
                        .width(columns[0].available_width())
                        .selected_text(short_queue_name(&selected))
                        .show_ui(&mut columns[0], |ui| {
                            for queue in &self.queues {
                                ui.selectable_value(
                                    &mut self.add_queue,
                                    Some(queue.id.clone()),
                                    display_queue_name(&queue.id, &queue.name),
                                );
                            }
                        });
                    columns[1].label(RichText::new(tr("Save to")).small().color(p.muted));
                    let folder = self.folder_picker.field(
                        &mut columns[1],
                        &mut self.folder,
                        &self.store.paths.downloads,
                        &mut self.form_error,
                    );
                    editing |= focused == Some(folder.id);
                });
                if let Some(error) = &self.form_error {
                    ui.label(RichText::new(error).small().color(p.warning));
                }
            });
            let draft = self.torrents.draft.as_ref();
            let ready = draft.and_then(|draft| draft.picker.as_ref());
            let fits = draft.is_none_or(|draft| {
                ready.is_none_or(|picker| {
                    !picker.chosen_indices().is_empty()
                        && draft.free.is_none_or(|free| picker.chosen_size() <= free)
                })
            });
            let waiting_on_peers = matches!(source, Source::Magnet(_))
                && ready.is_none()
                && draft.is_some_and(|draft| draft.is_waiting());
            let action = if self.torrents.saving.is_some() {
                tr("Adding…")
            } else if !enabled {
                tr("Turn on torrents")
            } else if waiting_on_peers {
                tr("Add and choose later")
            } else if self
                .background_io
                .starts
                .get(self.add_queue_id())
                .copied()
                .unwrap_or(false)
            {
                tr("Start download")
            } else {
                tr("Add to queue")
            };
            let hint = Some(tr("Enter to add · Esc to cancel"));
            let can_submit = !saving && (!enabled || waiting_on_peers || (ready.is_some() && fits));
            ui.add_enabled_ui(can_submit, |ui| {
                let pressed = form_dialog::footer(ui, action, hint, p)
                    || (editing
                        && can_submit
                        && ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                        }));
                if !enabled {
                    turn_on = pressed;
                } else {
                    submit = pressed;
                }
            });
        });
        if self.torrents.question.is_some() {
            ctx.request_repaint();
        } else if self.torrents.draft.as_ref().is_some_and(Draft::is_waiting) {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if !saving && (cancel || response.should_close()) {
            self.forget_torrent_draft();
            self.add_queue = None;
            self.dialog = None;
            self.form_error = None;
            self.url.clear();
        } else if turn_on {
            self.torrents.question = Some(Question::AddWindow);
        } else if submit {
            self.form_error = None;
            self.submit_torrent(ctx);
        } else if proxy {
            self.forget_torrent_draft();
            self.dialog = None;
            self.open_proxy();
        } else if let Some(id) = show {
            self.forget_torrent_draft();
            self.url.clear();
            self.dialog = None;
            self.set_selection(Some(id));
            self.filter = Filter::All;
            self.view = View::Downloads;
        }
    }

    /// Sample screens for checking how these look, without any helper or network.
    #[cfg(feature = "screenshot")]
    pub(super) fn torrent_preview(&mut self, mode: &str) {
        const LINK: &str =
            "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335aa7c1367a88a&dn=Open+Movies";
        let file = |index: usize, path: &str, len: u64| File {
            index,
            path: path.into(),
            saved_as: path.into(),
            len,
        };
        let files = vec![
            file(0, "Big Buck Bunny/big-buck-bunny-4k.mp4", 2_800_000_000),
            file(1, "Sintel/sintel-1080p.mkv", 1_200_000_000),
            file(2, "Sintel/sintel-subtitles.zip", 18_000_000),
            file(3, "Tears of Steel/tears-of-steel-4k.mov", 6_300_000_000),
            file(
                4,
                "Tears of Steel/tears-of-steel-1080p-with-a-very-long-file-name-that-has-to-be-cut-short.mkv",
                1_900_000_000,
            ),
            file(5, "README.txt", 4_000),
        ];
        self.url = LINK.into();
        self.open_add();
        self.torrents.settings.enabled = mode != "torrent-question" && mode != "torrent-not-now";
        let mut draft = Draft::new(Source::Magnet(LINK.into()));
        draft.asked = mode == "torrent-not-now";
        match mode {
            "torrent-waiting" => {
                draft.fetch = background_io::Task::spawn(None, || {
                    std::thread::sleep(Duration::from_secs(4));
                    Ok(Fetched::Cancelled)
                })
                .ok();
            }
            "torrent-files" | "torrent-no-space" | "torrent-none" => {
                draft.free = Some(if mode == "torrent-no-space" {
                    6_100_000_000
                } else {
                    38_200_000_000
                });
                draft.free_for = Some(self.store.paths.downloads.clone());
                let mut picker = Picker::new(files.clone(), None);
                picker.chosen[3] = false;
                if mode == "torrent-none" {
                    picker.set_all(false);
                }
                draft.picker = Some(picker);
                draft.details = Some(Details {
                    info_hash: "c12fe1c06bba254a9dc9f519b335aa7c1367a88a".into(),
                    name: "Blender Open Movies Collection".into(),
                    magnet: LINK.into(),
                    files,
                });
            }
            "torrent-choice" => {
                self.dialog = Some(Dialog::TorrentChoice);
                self.torrents.choice = Some(Choice {
                    id: String::new(),
                    name: "Blender Open Movies Collection".into(),
                    picker: Picker::new(files, None),
                });
            }
            _ => {}
        }
        self.torrents.draft = Some(draft);
    }

    pub(super) fn torrent_choice_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(mut choice) = self.torrents.choice.take() else {
            self.dialog = None;
            return;
        };
        let (mut cancel, mut start) = (false, false);
        let response = form_dialog::modal("torrent-choice", p).show(ctx, |ui| {
            ui.set_width(600.0);
            cancel = form_dialog::title(
                ui,
                tr("Choose files"),
                form_dialog::Icon::Symbol("\u{e716}"),
                p,
                true,
            );
            ui.add(egui::Label::new(RichText::new(&choice.name).strong()).truncate());
            choice.picker.select_buttons(ui);
            choice.picker.list(ui, p, LIST_HEIGHT);
            ui.label(
                RichText::new(choice.picker.summary())
                    .small()
                    .color(p.muted),
            );
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let enabled = !choice.picker.chosen_indices().is_empty();
                start = form_dialog::primary_enabled(ui, enabled, tr("Start download")).clicked();
                cancel |= form_dialog::secondary_button(ui, tr("Cancel")).clicked();
            });
        });
        if start {
            self.dialog = None;
            let command = Command::ChooseTorrentFiles {
                id: choice.id,
                chosen: choice.picker.chosen_indices(),
            };
            if let Err(error) = self.scheduler.send(command) {
                self.notice = Some(Notice::error(error.to_string()));
            }
            self.force_refresh();
        } else if cancel || response.should_close() {
            self.dialog = None;
        } else {
            self.torrents.choice = Some(choice);
        }
    }
}

/// What a torrent's row says, which depends on whether torrents are on.
pub(super) fn list_status(job: &Job, queue_running: bool, now: i64, torrents_on: bool) -> String {
    if job.torrent.is_some() && !torrents_on && job.status == Status::Paused {
        return tr("Torrents are off").into();
    }
    status_label(job, queue_running, now)
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::{click, fixture, frame};
    use super::*;

    fn file(index: usize, path: &str, len: u64) -> File {
        File {
            index,
            path: path.into(),
            saved_as: path.into(),
            len,
        }
    }

    fn sample() -> Picker {
        Picker::new(
            vec![
                file(0, "z.txt", 1),
                file(1, "film/a.mkv", 10),
                file(2, "film/extras/b.mkv", 20),
                file(3, "film/c.mkv", 30),
                file(4, "film-notes.txt", 40),
            ],
            None,
        )
    }

    const LINK: &str = "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335aa7c1367a88a&dn=sample";

    fn shown(output: &egui::FullOutput, label: &str) -> bool {
        output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.trim() == label)
        })
    }

    fn paste_magnet(app: &mut Desktop, ctx: &egui::Context) -> egui::FullOutput {
        app.open_add();
        app.url = LINK.into();
        let size = egui::vec2(1080.0, 800.0);
        frame(app, ctx, size, vec![]);
        frame(app, ctx, size, vec![])
    }

    #[test]
    fn a_magnet_link_asks_first_and_nothing_starts_until_the_answer_is_yes() {
        let (_temp, mut app, ctx) = fixture();
        assert!(!app.torrents.settings.enabled, "off by default");

        let output = paste_magnet(&mut app, &ctx);
        assert!(shown(&output, "Turn on torrents?"));
        assert!(app.torrents.draft.as_ref().unwrap().fetch.is_none());
        assert!(!app.store.torrents.is_running());

        click(&mut app, &ctx, "Not now");
        let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
        assert!(!shown(&output, "Turn on torrents?"), "it asks once");
        assert!(shown(&output, "Torrents are off"));
        assert!(!app.store.torrent_settings().unwrap().enabled);
        assert!(app.torrents.draft.as_ref().unwrap().fetch.is_none());
        assert!(!app.store.torrents.is_running());

        click(&mut app, &ctx, "Turn on torrents");
        click(&mut app, &ctx, "Turn on torrents");
        assert!(app.torrents.question.is_none());
        assert!(app.torrents.settings.enabled);
        assert!(
            app.store.torrent_settings().unwrap().enabled,
            "yes turns on the Preferences switch"
        );
    }

    #[test]
    fn saying_no_never_changes_the_setting() {
        let (_temp, mut app, ctx) = fixture();
        paste_magnet(&mut app, &ctx);
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
        assert!(app.torrents.question.is_none());
        assert!(!app.store.torrent_settings().unwrap().enabled);
    }

    #[test]
    fn the_preferences_switch_turns_torrents_on_and_off() {
        let (_temp, mut app, ctx) = fixture();
        click(&mut app, &ctx, "Preferences");
        click(&mut app, &ctx, "Torrents");
        click(&mut app, &ctx, "Torrent downloads");
        click(&mut app, &ctx, "Save");
        assert!(app.torrents.settings.enabled);
        assert!(app.store.torrent_settings().unwrap().enabled);

        click(&mut app, &ctx, "Preferences");
        click(&mut app, &ctx, "Torrents");
        click(&mut app, &ctx, "Torrent downloads");
        click(&mut app, &ctx, "Save");
        assert!(!app.store.torrent_settings().unwrap().enabled);
    }

    #[test]
    fn cancelling_the_preferences_window_leaves_torrents_off() {
        let (_temp, mut app, ctx) = fixture();
        click(&mut app, &ctx, "Preferences");
        click(&mut app, &ctx, "Torrents");
        click(&mut app, &ctx, "Torrent downloads");
        click(&mut app, &ctx, "Cancel");
        assert!(!app.store.torrent_settings().unwrap().enabled);
    }

    #[test]
    fn resuming_a_torrent_while_off_asks_instead_of_starting() {
        let (_temp, mut app, _ctx) = fixture();
        let mut job = Job::new_torrent(
            LINK,
            "sample",
            app.store.paths.downloads.clone(),
            Uuid::new_v4().to_string(),
            Stage::Chosen,
        )
        .unwrap();
        job.status = Status::Paused;
        let job = app.store.enqueue(&job).unwrap();
        app.refresh_and_wait();
        app.command(Command::Resume(job.id.clone()));
        assert!(app.torrents.question.is_some());
        assert_eq!(
            app.store.get(&job.id).unwrap().unwrap().status,
            Status::Paused,
            "the question comes before anything reaches the scheduler"
        );
    }

    #[test]
    fn a_row_says_when_torrents_are_off() {
        let mut job = Job::new_torrent(
            LINK,
            "sample",
            std::env::temp_dir(),
            "request".into(),
            Stage::Chosen,
        )
        .unwrap();
        job.status = Status::Paused;
        assert_eq!(list_status(&job, true, 0, false), "Torrents are off");
        assert_eq!(list_status(&job, true, 0, true), "Paused");
    }

    #[test]
    fn dropping_a_draft_ends_its_search() {
        let draft = Draft::new(Source::Magnet(LINK.into()));
        let search = draft.cancel.clone();
        drop(draft);
        assert!(search.is_cancelled());
    }

    #[test]
    fn sources_are_recognized() {
        assert_eq!(
            Source::detect(" magnet:?xt=urn:btih:abc "),
            Some(Source::Magnet("magnet:?xt=urn:btih:abc".into()))
        );
        let path = std::env::temp_dir().join("A File.TORRENT");
        assert_eq!(
            Source::detect(&format!("\"{}\"", path.display())),
            Some(Source::File(path))
        );
        for other in [
            "https://example.com/a.torrent",
            "relative.torrent",
            "https://example.com/a.zip",
            "",
        ] {
            assert_eq!(Source::detect(other), None, "{other}");
        }
    }

    #[test]
    fn folders_become_rows_that_cover_their_files() {
        let picker = sample();
        let rows: Vec<(usize, &str, u64)> = picker
            .rows
            .iter()
            .map(|row| (row.depth, row.name.as_str(), row.size))
            .collect();
        assert_eq!(
            rows,
            [
                (0, "film-notes.txt", 40),
                (0, "film", 60),
                (1, "a.mkv", 10),
                (1, "c.mkv", 30),
                (1, "extras", 20),
                (2, "b.mkv", 20),
                (0, "z.txt", 1),
            ]
        );
        let folder = &picker.rows[1];
        assert_eq!(folder.files.len(), 3);
    }

    #[test]
    fn choosing_a_folder_chooses_what_is_in_it() {
        let mut picker = sample();
        assert_eq!(picker.chosen_indices().len(), 5);
        picker.set_all(false);
        let folder = picker.rows[1].files.clone();
        picker.chosen[folder].fill(true);
        let mut chosen = picker.chosen_indices();
        chosen.sort_unstable();
        assert_eq!(chosen, [1, 2, 3]);
        assert_eq!(picker.chosen_size(), 60);
    }

    #[test]
    fn an_earlier_choice_comes_back() {
        let picker = Picker::new(vec![file(5, "a", 1), file(6, "b", 1)], Some(&[6]));
        assert_eq!(picker.chosen_indices(), [6]);
    }

    #[test]
    fn changed_names_are_counted() {
        let mut renamed = file(0, "C:evil.dll", 1);
        renamed.saved_as = "C_evil.dll".into();
        let picker = Picker::new(vec![renamed, file(1, "folder/fine.bin", 1)], None);
        assert_eq!(picker.renamed(), 1);
    }
}
