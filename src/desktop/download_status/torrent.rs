//! A torrent's piece map and its Files, Peers and Trackers lists, under "Show details".
//! The worker that runs the torrent supplies the details while a window watches. The window
//! only asks for changes through the scheduler, so it can never start the torrent helper.
use super::*;
use download_engine::torrent::{
    Files, Request, RequestOutcome, Source, SourceState, Stage, TorrentDetails, TrackerState,
};
use std::sync::Arc;

/// How long to wait for the torrent's worker to answer a request.
const REQUEST_PATIENCE: Duration = Duration::from_secs(30);
/// A change of files is saved by the scheduler first, so the list is read again after this.
pub(super) const FILES_SETTLE: Duration = Duration::from_millis(800);
const ROW: f32 = 20.0;
const MIN_LIST: f32 = 96.0;
/// Saved trackers shown while the torrent isn't running. A magnet link can name more, but
/// the helper only ever uses 32.
const MAX_SHOWN_TRACKERS: usize = 64;

pub(super) fn peer_connection(peer: &download_engine::torrent::Peer) -> String {
    use de_torrent::details::PeerTransport;
    if peer.connection.transport == PeerTransport::Unknown && peer.connection.encrypted.is_none() {
        return tr("Unknown").to_owned();
    }
    let protocol = match peer.connection.transport {
        PeerTransport::Tcp => "TCP",
        PeerTransport::Utp => "uTP",
        PeerTransport::Unknown => tr("Unknown"),
    };
    let protection = match peer.connection.encrypted {
        Some(true) => tr("Encrypted"),
        Some(false) => tr("Plaintext"),
        None => tr("Unknown"),
    };
    format!("{protocol} · {protection}")
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) enum TorrentTab {
    #[default]
    Files,
    Peers,
    Trackers,
}

/// What the window still has to hand to the scheduler.
pub(super) enum Outgoing {
    Files(Vec<usize>),
    Request(Request),
}

pub(super) struct Pending {
    pub(super) ticket: u64,
    sent: Instant,
    request: Request,
}

#[derive(Default)]
pub(super) struct TorrentView {
    pub(super) tab: TorrentTab,
    /// The last details seen. They stay, marked with their age, once the torrent stops.
    pub(super) details: Option<Arc<TorrentDetails>>,
    pub(super) seen: Option<Instant>,
    /// Each file's progress as last seen, for the same reason.
    pub(super) progress: Option<Arc<[u64]>>,
    pub(super) picker: Option<torrents::Picker>,
    /// The chosen files the picker was built from, to notice changes made elsewhere.
    chosen: Vec<usize>,
    pub(super) reload_files_at: Option<Instant>,
    pub(super) added: Vec<String>,
    pub(super) tracker_text: String,
    pub(super) pending: Option<Pending>,
    /// The outcome of the last request: whether it worked, and what to say.
    pub(super) message: Option<(bool, String)>,
    pub(super) outbox: Vec<Outgoing>,
}

impl TorrentView {
    /// Whether the next refresh should read the file list.
    pub(super) fn wants_files(&self, now: Instant) -> bool {
        self.picker.is_none() || self.reload_files_at.is_some_and(|at| now >= at)
    }

    pub(super) fn apply(
        &mut self,
        details: Option<Arc<TorrentDetails>>,
        files: Option<Files>,
        added: Vec<String>,
    ) {
        if let Some(details) = details {
            self.details = Some(details);
            self.seen = Some(Instant::now());
        }
        if let Some(files) = files {
            self.reload_files_at = None;
            if self.picker.is_none() || files.chosen != self.chosen {
                self.chosen = files.chosen.clone();
                let chosen = (!files.chosen.is_empty()).then_some(files.chosen.as_slice());
                let mut picker = torrents::Picker::new(files.list, chosen);
                if chosen.is_none() {
                    // A torrent waiting for a choice has none chosen yet.
                    for row in 0..picker.row_count() {
                        picker.set_row(row, false);
                    }
                }
                self.picker = Some(picker);
            }
        }
        self.added = added;
    }

    /// Settles a request once its outcome has arrived, or gives up on it.
    pub(super) fn settle(&mut self, outcome: Option<RequestOutcome>) {
        let Some(pending) = &self.pending else {
            return;
        };
        let message = match outcome {
            Some(Ok(Some(text))) => (true, tr(text).to_owned()),
            Some(Ok(None)) => (
                true,
                tr(match pending.request {
                    Request::Recheck => "Checking the files again.",
                    Request::AddTracker(_) => "Tracker added.",
                    Request::RemoveTracker(_) => "Tracker removed.",
                })
                .to_owned(),
            ),
            Some(Err(error)) => (false, tr(&error).to_owned()),
            None if pending.sent.elapsed() > REQUEST_PATIENCE => (
                false,
                tr("The torrent didn't answer. Try again.").to_owned(),
            ),
            None => return,
        };
        if message.0 && matches!(pending.request, Request::AddTracker(_)) {
            self.tracker_text.clear();
        }
        self.message = Some(message);
        self.pending = None;
    }

    pub(super) fn sent(&mut self, ticket: u64, request: Request) {
        self.message = None;
        self.pending = Some(Pending {
            ticket,
            sent: Instant::now(),
            request,
        });
    }
}

/// Tracker addresses in the download's own magnet link, for a torrent that isn't running.
fn link_trackers(link: &str) -> Vec<String> {
    reqwest::Url::parse(link)
        .map(|url| {
            url.query_pairs()
                .filter(|(key, _)| key == "tr")
                .map(|(_, value)| value.into_owned())
                .filter(|value| value.len() <= 2048)
                .take(MAX_SHOWN_TRACKERS)
                .collect()
        })
        .unwrap_or_default()
}

fn wait_label(seconds: u64) -> String {
    if seconds < 60 {
        tf!("{seconds} sec", seconds = seconds)
    } else if seconds < 3600 {
        tf!("{} min {} sec", seconds / 60, seconds % 60)
    } else {
        tf!("{} hr {} min", seconds / 3600, seconds % 3600 / 60)
    }
}

fn speed(bytes_per_second: u64) -> String {
    if bytes_per_second == 0 {
        "—".into()
    } else {
        tf!("{}/sec", bytes(bytes_per_second))
    }
}

/// A word in a thin rounded outline, for states.
fn pill(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), egui::FontId::proportional(10.5), color);
    let size = vec2(galley.size().x + 12.0, 15.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_stroke(
        rect,
        7,
        Stroke::new(1.0, color.gamma_multiply(0.5)),
        StrokeKind::Inside,
    );
    ui.painter().galley(
        pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response
}

/// One cell of a list row, `width` wide, holding a label cut short with … when needed.
fn cell(
    ui: &mut egui::Ui,
    width: f32,
    text: impl Into<egui::WidgetText>,
    right: bool,
) -> egui::Response {
    ui.allocate_ui_with_layout(
        vec2(width, ROW),
        if right {
            Layout::right_to_left(Align::Center)
        } else {
            Layout::left_to_right(Align::Center)
        },
        |ui| {
            ui.set_min_width(width);
            ui.add(egui::Label::new(text).truncate().selectable(false))
        },
    )
    .inner
}

fn header(ui: &mut egui::Ui, p: Palette, columns: &[(&str, f32, bool)]) {
    ui.horizontal(|ui| {
        ui.set_height(18.0);
        for (label, width, right) in columns {
            cell(
                ui,
                *width,
                RichText::new(tr(label)).size(10.5).strong().color(p.muted),
                *right,
            );
        }
    });
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0, p.border);
}

/// The flexible column takes what the `columns` fixed ones leave.
fn first_width(ui: &egui::Ui, fixed: f32, columns: usize) -> f32 {
    (ui.available_width() - fixed - columns as f32 * ui.spacing().item_spacing.x).max(60.0)
}

fn text_width(ui: &egui::Ui, text: &str, font: egui::FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER)
        .size()
        .x
}

fn pill_width(ui: &egui::Ui, text: &str) -> f32 {
    text_width(ui, text, egui::FontId::proportional(10.5)) + 12.0
}

/// A fixed column as wide as its header or its widest entry, so that a long translation
/// widens the column instead of the window.
fn fit(ui: &egui::Ui, header: &str, widths: impl IntoIterator<Item = f32>, least: f32) -> f32 {
    let header = text_width(ui, tr(header), egui::FontId::proportional(10.5));
    widths.into_iter().fold(header.max(least), f32::max).ceil() + 2.0
}

fn flow_word(peer: &download_engine::torrent::Peer) -> &'static str {
    match (peer.receiving, peer.sending) {
        (true, true) => "Both ways",
        (true, false) => "Receiving",
        (false, true) => "Sending",
        (false, false) => "Waiting",
    }
}

fn stripe(ui: &mut egui::Ui, p: Palette, index: usize) {
    if index % 2 == 1 {
        let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), ROW));
        ui.painter()
            .rect_filled(rect, 0, p.text.gamma_multiply(0.035));
    }
}

/// The piece map: one run per stretch of equal pixels, so a huge torrent stays cheap.
fn draw_piece_bar(ui: &mut egui::Ui, map: &str, p: Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3, p.track);
    let inner = rect.shrink(1.0);
    let cells: Vec<u8> = map.bytes().collect();
    if !cells.is_empty() {
        let columns = inner.width().floor().max(1.0) as usize;
        let color_of = |column: usize| -> Option<Color32> {
            let start = column * cells.len() / columns;
            let end = ((column + 1) * cells.len() / columns).max(start + 1);
            let part = &cells[start..end.min(cells.len())];
            if part.contains(&b'*') {
                return Some(p.orange);
            }
            let wanted: Vec<u8> = part.iter().copied().filter(|cell| *cell != b'-').collect();
            if wanted.is_empty() {
                return Some(p.background);
            }
            let done: u32 = wanted.iter().map(|cell| u32::from(cell - b'0')).sum();
            let share = done as f32 / (9 * wanted.len()) as f32;
            (share > 0.0).then(|| p.track.lerp_to_gamma(p.blue, share.max(0.35)))
        };
        let mut column = 0;
        while column < columns {
            let color = color_of(column);
            let mut end = column + 1;
            while end < columns && color_of(end) == color {
                end += 1;
            }
            if let Some(color) = color {
                let left = inner.left() + inner.width() * column as f32 / columns as f32;
                let right = inner.left() + inner.width() * end as f32 / columns as f32;
                painter.rect_filled(
                    Rect::from_min_max(pos2(left, inner.top()), pos2(right, inner.bottom())),
                    0,
                    color,
                );
            }
            column = end;
        }
    }
    painter.rect_stroke(rect, 3, Stroke::new(1.0, p.border), StrokeKind::Inside);
}

fn legend(ui: &mut egui::Ui, p: Palette) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (color, label) in [
            (p.blue, "Finished"),
            (p.orange, "In progress"),
            (p.track, "Missing"),
            (p.background, "Skipped"),
        ] {
            let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().rect_filled(rect, 2, color);
            ui.painter()
                .rect_stroke(rect, 2, Stroke::new(1.0, p.border), StrokeKind::Inside);
            ui.label(RichText::new(tr(label)).size(10.5).color(p.muted));
            ui.add_space(8.0);
        }
    });
}

impl Window {
    fn torrent_running(&self) -> bool {
        self.phase.is_some()
    }

    fn recheck_allowed(&self, job: &Job) -> Result<(), &'static str> {
        if !self.torrents_on {
            return Err("Torrents are off");
        }
        if job.status == Status::Complete {
            return Err("Finished torrents can't be checked again here.");
        }
        if job
            .torrent
            .as_ref()
            .is_none_or(|state| state.stage != Stage::Chosen)
        {
            return Err("Choose the torrent's files first.");
        }
        if self.phase == Some(Phase::CheckingSaved) {
            return Err("The files are being checked.");
        }
        if self.torrent.pending.is_some() || self.waiting.is_some() {
            return Err("Wait for the last change to finish.");
        }
        Ok(())
    }

    pub(super) fn draw_torrent_details(&mut self, ui: &mut egui::Ui, job: &Job, p: Palette) {
        let running = self.torrent_running();
        let details = self.torrent.details.clone();
        let pieces = details.as_ref().and_then(|details| details.pieces.clone());
        ui.horizontal(|ui| {
            let line = match &pieces {
                Some(pieces) if pieces.checked.is_some() && running => tf!(
                    "Checking files: {done} of {total} pieces",
                    done = pieces.checked.unwrap_or(0),
                    total = pieces.total
                ),
                Some(pieces) if pieces.in_progress > 0 => tf!(
                    "{have} of {wanted} pieces · {busy} in progress",
                    have = pieces.have,
                    wanted = pieces.wanted,
                    busy = pieces.in_progress
                ),
                Some(pieces) => tf!(
                    "{have} of {wanted} pieces",
                    have = pieces.have,
                    wanted = pieces.wanted
                ),
                None if running => tr("Pieces show once the file list is known.").into(),
                None => tr("Pieces show while the torrent runs.").into(),
            };
            ui.label(
                RichText::new(format!("{} · {line}", tr("Pieces")))
                    .size(11.0)
                    .color(p.muted),
            );
            if job.status != Status::Complete {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let allowed = self.recheck_allowed(job);
                    let button = ui
                        .add_enabled(
                            allowed.is_ok(),
                            egui::Button::new(RichText::new(tr("Check files again")).size(11.5))
                                .small(),
                        )
                        .on_hover_text(tr(
                            "Reads every chosen piece from disk again. Pieces that don't match download again.",
                        ));
                    if let Err(reason) = allowed {
                        button.clone().on_disabled_hover_text(tr(reason));
                    }
                    if button.clicked() {
                        self.torrent
                            .outbox
                            .push(Outgoing::Request(Request::Recheck));
                    }
                });
            }
        });
        draw_piece_bar(
            ui,
            pieces.as_ref().map_or("", |pieces| pieces.map.as_str()),
            p,
        );
        legend(ui, p);
        if let (Some(seen), false) = (self.torrent.seen, running) {
            let minutes = seen.elapsed().as_secs() / 60;
            ui.label(
                RichText::new(if minutes == 0 {
                    tr("Last seen less than a minute ago, while the torrent ran.").to_owned()
                } else {
                    tf!(
                        "Last seen {minutes} min ago, while the torrent ran.",
                        minutes = minutes
                    )
                })
                .size(10.5)
                .color(p.muted),
            );
        }
        ui.add_space(2.0);

        let peers = running.then(|| details.as_ref().map(|details| details.peers.len()));
        let trackers = self.tracker_rows(job).len();
        let files = self
            .torrent
            .picker
            .as_ref()
            .map(|picker| picker.file_count());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (tab, label, count) in [
                (TorrentTab::Files, tr("Files"), files),
                (TorrentTab::Peers, tr("Peers"), peers.flatten()),
                (TorrentTab::Trackers, tr("Trackers"), Some(trackers)),
            ] {
                let selected = self.torrent.tab == tab;
                let text = match count {
                    Some(count) => format!("{label}  {count}"),
                    None => label.to_owned(),
                };
                let response = ui.add(
                    egui::Button::new(RichText::new(text).size(11.5).color(if selected {
                        p.text
                    } else {
                        p.muted
                    }))
                    .fill(if selected {
                        p.selected
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::new(
                        1.0,
                        if selected {
                            p.border
                        } else {
                            Color32::TRANSPARENT
                        },
                    ))
                    .corner_radius(6),
                );
                if response.clicked() {
                    self.torrent.tab = tab;
                }
                if selected {
                    ui.painter().line_segment(
                        [
                            pos2(response.rect.left() + 6.0, response.rect.bottom()),
                            pos2(response.rect.right() - 6.0, response.rect.bottom()),
                        ],
                        Stroke::new(2.0, p.accent),
                    );
                }
            }
        });
        ui.add_space(2.0);
        // Lists fill what the window leaves, so a taller window shows more rows.
        // What goes under the rows: the list's header and summary, and a message line.
        let height = (ui.clip_rect().bottom() - ui.cursor().top() - 66.0).clamp(MIN_LIST, 640.0);
        egui::Frame::new()
            .fill(p.background)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().interact_size.y = ROW;
                match self.torrent.tab {
                    TorrentTab::Files => self.draw_files(ui, job, p, height),
                    TorrentTab::Peers => self.draw_peers(ui, p, height),
                    TorrentTab::Trackers => self.draw_trackers(ui, job, p, height),
                }
            });
        if let Some((ok, message)) = &self.torrent.message {
            ui.add(
                egui::Label::new(RichText::new(message).size(11.0).color(if *ok {
                    p.success
                } else {
                    p.warning
                }))
                .wrap(),
            );
        } else if self.torrent.pending.is_some() {
            ui.spinner();
        }
    }

    fn draw_files(&mut self, ui: &mut egui::Ui, job: &Job, p: Palette, height: f32) {
        let Some(picker) = self.torrent.picker.as_mut() else {
            ui.label(
                RichText::new(tr(
                    if job.torrent.as_ref().is_some_and(|state| state.stage == Stage::Details) {
                        "The file list isn't known yet. It comes from other people once the torrent runs."
                    } else {
                        "Reading the file list…"
                    },
                ))
                .size(11.0)
                .color(p.muted),
            );
            return;
        };
        let progress: Vec<u64> = self
            .live_torrent
            .as_ref()
            .map(|live| live.file_progress.clone())
            .or_else(|| self.torrent.progress.clone())
            .map(|progress| progress.to_vec())
            .unwrap_or_default();
        let known = !progress.is_empty() || job.status == Status::Complete;
        let editable = job.status != Status::Complete
            && self.torrent.pending.is_none()
            && self.waiting.is_none();
        let small = egui::FontId::proportional(11.0);
        let bar = 110.0;
        let percent = fit(ui, "", [text_width(ui, "100%", small.clone())], 30.0);
        let size = fit(ui, "Size", [text_width(ui, "1023.9 MB", small)], 50.0);
        let name_width = first_width(ui, 18.0 + bar + percent + size, 4);
        header(
            ui,
            p,
            &[
                ("", 18.0, false),
                ("Name", name_width, false),
                ("Progress", bar, false),
                ("", percent, true),
                ("Size", size, true),
            ],
        );
        let mut toggled = None;
        egui::ScrollArea::vertical()
            .id_salt("status-torrent-files")
            .max_height(height)
            .auto_shrink([false, true])
            .show_rows(ui, ROW, picker.row_count(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for index in range {
                    let row = picker.row_view(index, &progress);
                    let skipped = row.picked == 0;
                    stripe(ui, p, index);
                    ui.horizontal(|ui| {
                        ui.set_height(ROW);
                        let mut all = row.picked == row.files;
                        let tick = ui
                            .add_enabled_ui(editable, |ui| {
                                ui.add(
                                    view::Checkbox::new(&mut all, "")
                                        .indeterminate(row.picked > 0 && row.picked < row.files),
                                )
                            })
                            .inner;
                        if tick.clicked() {
                            toggled = Some((index, all));
                        }
                        let name = format!(
                            "{}{}",
                            "    ".repeat(row.depth),
                            if row.folder {
                                format!("{}/", row.name)
                            } else {
                                row.name.to_owned()
                            }
                        );
                        cell(
                            ui,
                            name_width,
                            RichText::new(&name).size(11.5).color(if skipped {
                                p.muted
                            } else {
                                p.text
                            }),
                            false,
                        )
                        .on_hover_text(row.name);
                        let share = row.progress.filter(|_| known);
                        ui.allocate_ui_with_layout(
                            vec2(bar, ROW),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.set_min_width(bar);
                                if let Some(share) = share {
                                    let (rect, _) =
                                        ui.allocate_exact_size(vec2(bar, 7.0), Sense::hover());
                                    ui.painter().rect_filled(rect, 3, p.track);
                                    let mut done = rect;
                                    done.set_width(rect.width() * share as f32);
                                    ui.painter().rect_filled(
                                        done,
                                        3,
                                        if share >= 1.0 { p.accent } else { p.blue },
                                    );
                                }
                            },
                        );
                        cell(
                            ui,
                            percent,
                            RichText::new(match share {
                                Some(share) => format!("{:.0}%", (share * 100.0).floor()),
                                None => "—".into(),
                            })
                            .size(11.0)
                            .color(p.muted),
                            true,
                        );
                        cell(
                            ui,
                            size,
                            RichText::new(bytes(row.size)).size(11.0).color(p.muted),
                            true,
                        );
                    });
                }
            });
        if let Some((index, chosen)) = toggled {
            picker.set_row(index, chosen);
            let indices = picker.chosen_indices();
            if indices.is_empty() {
                // Undo it: a torrent needs at least one file.
                picker.set_row(index, !chosen);
                self.torrent.message =
                    Some((false, tr("Select at least one file to download").to_owned()));
            } else {
                self.torrent.message = None;
                self.torrent.chosen = indices.clone();
                self.torrent.outbox.push(Outgoing::Files(indices));
            }
        }
        ui.label(RichText::new(picker.summary()).size(10.5).color(p.muted));
    }

    fn draw_peers(&mut self, ui: &mut egui::Ui, p: Palette, height: f32) {
        ui.scope(|ui| {
            // Reserve a visible scrollbar instead of covering the last peer row.
            ui.spacing_mut().scroll.floating = false;
            egui::ScrollArea::horizontal()
                .id_salt("torrent-peers-horizontal")
                .show(ui, |ui| {
                    if self
                        .torrent
                        .details
                        .as_ref()
                        .is_some_and(|details| !details.peers.is_empty())
                    {
                        ui.set_min_width(ui.available_width().max(850.0));
                    }
                    self.draw_peer_table(ui, p, height);
                });
        });
    }

    fn draw_peer_table(&mut self, ui: &mut egui::Ui, p: Palette, height: f32) {
        let details = self.torrent.details.clone();
        let running = self.torrent_running();
        let empty = if !running {
            Some(tr("Peers show while the torrent runs."))
        } else if self.phase == Some(Phase::CheckingSaved) {
            Some(tr("Peers connect again after the check."))
        } else if details
            .as_ref()
            .is_none_or(|details| details.peers.is_empty())
        {
            Some(tr(
                "No peers yet. Download Engine is still looking for them.",
            ))
        } else {
            None
        };
        if let Some(empty) = empty {
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), MIN_LIST.min(height)),
                Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| ui.label(RichText::new(empty).size(11.5).color(p.muted)),
            );
            return;
        }
        let Some(details) = details else { return };
        let total = details.pieces.as_ref().map_or(0, |pieces| pieces.total);
        let small = egui::FontId::proportional(11.0);
        let mono = egui::FontId::monospace(11.0);
        let has = fit(ui, "Has", [text_width(ui, "100%", small.clone())], 30.0);
        let fastest = text_width(ui, &speed(999 * 1024 * 1024), small);
        let down = fit(ui, "Download rate", [fastest], 50.0);
        let up = fit(ui, "Upload rate", [fastest], 50.0);
        let incoming = pill_width(ui, tr("incoming")) + 3.0;
        let flow = fit(
            ui,
            "Data",
            details.peers.iter().map(|peer| {
                pill_width(ui, tr(flow_word(peer))) + if peer.incoming { incoming } else { 0.0 }
            }),
            60.0,
        );
        let address = fit(
            ui,
            "Address",
            details
                .peers
                .iter()
                .map(|peer| text_width(ui, peer.address.as_deref().unwrap_or("—"), mono.clone())),
            90.0,
        )
        .min(210.0);
        let connection = fit(
            ui,
            "Connection",
            details.peers.iter().map(|peer| {
                text_width(ui, &peer_connection(peer), egui::FontId::proportional(11.0))
            }),
            110.0,
        );
        let client = first_width(ui, address + has + down + up + flow + connection, 6);
        header(
            ui,
            p,
            &[
                ("Address", address, false),
                ("Client", client, false),
                ("Connection", connection, false),
                ("Has", has, true),
                ("Download rate", down, true),
                ("Upload rate", up, true),
                ("Data", flow, false),
            ],
        );
        egui::ScrollArea::vertical()
            .id_salt("status-torrent-peers")
            .max_height(height)
            .auto_shrink([false, true])
            .show_rows(ui, ROW, details.peers.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for index in range {
                    let peer = &details.peers[index];
                    stripe(ui, p, index);
                    ui.horizontal(|ui| {
                        ui.set_height(ROW);
                        cell(
                            ui,
                            address,
                            RichText::new(peer.address.as_deref().unwrap_or("—"))
                                .size(11.0)
                                .monospace(),
                            false,
                        );
                        cell(
                            ui,
                            client,
                            match &peer.client {
                                Some(name) => RichText::new(name).size(11.5),
                                None => RichText::new(tr("Unknown")).size(11.5).color(p.muted),
                            },
                            false,
                        );
                        cell(
                            ui,
                            connection,
                            RichText::new(peer_connection(peer)).size(11.0).color(
                                if peer.connection.encrypted == Some(true) {
                                    p.accent
                                } else {
                                    p.muted
                                },
                            ),
                            false,
                        );
                        let share = if total == 0 {
                            "—".to_owned()
                        } else {
                            format!("{}%", u64::from(peer.has_pieces) * 100 / u64::from(total))
                        };
                        cell(
                            ui,
                            has,
                            RichText::new(share).size(11.0).color(p.muted),
                            true,
                        );
                        cell(
                            ui,
                            down,
                            RichText::new(speed(peer.download_bps)).size(11.0),
                            true,
                        );
                        cell(
                            ui,
                            up,
                            RichText::new(speed(peer.upload_bps)).size(11.0),
                            true,
                        );
                        ui.allocate_ui_with_layout(
                            vec2(flow, ROW),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.set_min_width(flow);
                                ui.spacing_mut().item_spacing.x = 3.0;
                                let word = flow_word(peer);
                                let color = if word == "Waiting" { p.muted } else { p.accent };
                                pill(ui, tr(word), color);
                                if peer.incoming {
                                    pill(ui, tr("incoming"), p.muted);
                                }
                            },
                        );
                    });
                }
            });
    }

    /// The torrent's trackers: from the details while it runs or ran, otherwise from its
    /// magnet link and the ones the user added.
    fn tracker_rows(&self, job: &Job) -> Vec<download_engine::torrent::Tracker> {
        if let Some(details) = &self.torrent.details {
            let mut rows = details.trackers.clone();
            if !self.torrent_running() {
                // Removed while stopped: keep the list in step with what is saved.
                rows.retain(|tracker| !tracker.added || self.torrent.added.contains(&tracker.url));
                for url in &self.torrent.added {
                    if !rows.iter().any(|tracker| &tracker.url == url) {
                        rows.push(unknown_tracker(url, true));
                    }
                }
            }
            return rows;
        }
        let mut rows: Vec<_> = link_trackers(&job.url)
            .into_iter()
            .map(|url| unknown_tracker(&url, false))
            .collect();
        for url in &self.torrent.added {
            if !rows.iter().any(|tracker| &tracker.url == url) {
                rows.push(unknown_tracker(url, true));
            }
        }
        rows
    }

    fn draw_trackers(&mut self, ui: &mut egui::Ui, job: &Job, p: Palette, height: f32) {
        let running = self.torrent_running();
        let rows = self.tracker_rows(job);
        let details = self.torrent.details.clone();
        let small = egui::FontId::proportional(11.0);
        let number = |value: Option<u64>| {
            value.map_or(0.0, |value| {
                text_width(ui, &value.to_string(), small.clone())
            })
        };
        let state = fit(
            ui,
            "Status",
            rows.iter()
                .map(|tracker| pill_width(ui, &tracker_state(tracker, running, p).0))
                .chain(details.iter().flat_map(|details| {
                    [details.dht, details.pex]
                        .map(|source| pill_width(ui, source_state(&source, running, p).0))
                })),
            60.0,
        );
        let seeds = fit(
            ui,
            "Seeds",
            rows.iter().map(|tracker| number(tracker.seeds)),
            28.0,
        );
        let leechers = fit(
            ui,
            "Leechers",
            rows.iter().map(|tracker| number(tracker.leechers)),
            28.0,
        );
        let found = fit(
            ui,
            "Peers",
            rows.iter()
                .map(|tracker| number(tracker.peers.map(|peers| peers as u64)))
                .chain(details.iter().flat_map(|details| {
                    [
                        number(Some(details.dht.peers as u64)),
                        number(Some(details.pex.peers as u64)),
                    ]
                })),
            28.0,
        );
        let next = fit(
            ui,
            "Next update",
            rows.iter()
                .filter_map(|tracker| tracker.next_in)
                .map(|seconds| text_width(ui, &wait_label(seconds), small.clone())),
            40.0,
        );
        let url_width = first_width(ui, state + seeds + leechers + found + next, 5);
        let added_width = pill_width(ui, tr("added by you"));
        header(
            ui,
            p,
            &[
                ("Tracker", url_width, false),
                ("Status", state, false),
                ("Seeds", seeds, true),
                ("Leechers", leechers, true),
                ("Peers", found, true),
                ("Next update", next, true),
            ],
        );
        let editable = self.torrent.pending.is_none() && self.waiting.is_none();
        let mut remove = None;
        let sources = details
            .as_ref()
            .map(|details| [("DHT", details.dht), ("Peer exchange", details.pex)]);
        let count = rows.len() + sources.map_or(0, |sources| sources.len());
        egui::ScrollArea::vertical()
            .id_salt("status-torrent-trackers")
            .max_height((height - 30.0).max(MIN_LIST / 2.0))
            .auto_shrink([false, true])
            .show_rows(ui, ROW, count, |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for index in range {
                    stripe(ui, p, index);
                    if let Some(tracker) = rows.get(index) {
                        ui.horizontal(|ui| {
                            ui.set_height(ROW);
                            ui.allocate_ui_with_layout(
                                vec2(url_width, ROW),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.set_min_width(url_width);
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    // The address is cut short, never the × or the label.
                                    let mut label = url_width;
                                    if tracker.added {
                                        let close = ui
                                            .allocate_ui_with_layout(
                                                vec2(14.0, ROW),
                                                Layout::left_to_right(Align::Center),
                                                |ui| {
                                                    ui.add_enabled(
                                                        editable,
                                                        egui::Button::new(
                                                            RichText::new("×").size(12.0),
                                                        )
                                                        .frame(false),
                                                    )
                                                },
                                            )
                                            .inner
                                            .on_hover_text(tr("Remove"));
                                        if close.clicked() {
                                            remove = Some(tracker.url.clone());
                                        }
                                        label -= 14.0 + added_width + 2.0 * 4.0;
                                    }
                                    cell(
                                        ui,
                                        label.max(40.0),
                                        RichText::new(&tracker.url).size(11.0).monospace(),
                                        false,
                                    )
                                    .on_hover_text(&tracker.url);
                                    if tracker.added {
                                        pill(ui, tr("added by you"), p.blue);
                                    }
                                },
                            );
                            let (word, color, why) = tracker_state(tracker, running, p);
                            ui.allocate_ui_with_layout(
                                vec2(state, ROW),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.set_min_width(state);
                                    let response = pill(ui, &word, color);
                                    if let Some(why) = why {
                                        response.on_hover_text(why);
                                    }
                                },
                            );
                            let number = |value: Option<u64>| {
                                RichText::new(
                                    value.map(|value| value.to_string()).unwrap_or_default(),
                                )
                                .size(11.0)
                            };
                            cell(ui, seeds, number(tracker.seeds.filter(|_| running)), true);
                            cell(
                                ui,
                                leechers,
                                number(tracker.leechers.filter(|_| running)),
                                true,
                            );
                            cell(
                                ui,
                                found,
                                number(tracker.peers.filter(|_| running).map(|peers| peers as u64)),
                                true,
                            );
                            cell(
                                ui,
                                next,
                                RichText::new(
                                    tracker
                                        .next_in
                                        .filter(|_| running)
                                        .map(wait_label)
                                        .unwrap_or_default(),
                                )
                                .size(11.0)
                                .color(p.muted),
                                true,
                            );
                        });
                    } else if let Some((name, source)) = sources
                        .as_ref()
                        .and_then(|sources| sources.get(index - rows.len()))
                    {
                        ui.horizontal(|ui| {
                            ui.set_height(ROW);
                            cell(
                                ui,
                                url_width,
                                RichText::new(tr(name)).size(11.5).color(p.muted),
                                false,
                            );
                            let (word, color, why) = source_state(source, running, p);
                            ui.allocate_ui_with_layout(
                                vec2(state, ROW),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.set_min_width(state);
                                    let response = pill(ui, word, color);
                                    if let Some(why) = why {
                                        response.on_hover_text(tr(why));
                                    }
                                },
                            );
                            cell(ui, seeds, "", true);
                            cell(ui, leechers, "", true);
                            cell(
                                ui,
                                found,
                                RichText::new(source.peers.to_string()).size(11.0),
                                true,
                            );
                        });
                    }
                }
            });
        if let Some(url) = remove {
            self.torrent
                .outbox
                .push(Outgoing::Request(Request::RemoveTracker(url)));
        }
        let private = details.as_ref().and_then(|details| details.private);
        let blocked = if private == Some(true) {
            Some(download_engine::torrent::PRIVATE_TRACKERS_TEXT)
        } else if !running || private.is_none() {
            Some(download_engine::torrent::ADD_WHILE_RUNNING)
        } else if !editable {
            Some("Wait for the last change to finish.")
        } else {
            None
        };
        ui.add_space(3.0);
        // The button goes first from the right, so the field takes exactly what is left and
        // a long translation can never widen the window.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let button = ui.add_enabled(
                blocked.is_none(),
                egui::Button::new(RichText::new(tr("Add tracker")).size(11.5)),
            );
            let field = ui.add_enabled(
                blocked.is_none(),
                egui::TextEdit::singleline(&mut self.torrent.tracker_text)
                    .hint_text(tr("http, https or udp address"))
                    .desired_width(ui.available_width()),
            );
            let entered =
                field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if let Some(blocked) = blocked {
                button.clone().on_disabled_hover_text(tr(blocked));
                field.on_disabled_hover_text(tr(blocked));
            }
            if blocked.is_none() && (button.clicked() || entered) {
                match download_engine::torrent::check_tracker(&self.torrent.tracker_text) {
                    Ok(url) => self
                        .torrent
                        .outbox
                        .push(Outgoing::Request(Request::AddTracker(url))),
                    Err(error) => {
                        self.torrent.message = Some((false, tr(&error.to_string()).to_owned()))
                    }
                }
            }
        });
        if let Some(blocked) = blocked.filter(|_| private == Some(true) || !running) {
            ui.label(RichText::new(tr(blocked)).size(10.5).color(p.muted));
        }
    }
}

fn unknown_tracker(url: &str, added: bool) -> download_engine::torrent::Tracker {
    download_engine::torrent::Tracker {
        url: url.to_owned(),
        added,
        state: TrackerState::Waiting,
        message: None,
        peers: None,
        seeds: None,
        leechers: None,
        next_in: None,
    }
}

fn tracker_state(
    tracker: &download_engine::torrent::Tracker,
    running: bool,
    p: Palette,
) -> (String, Color32, Option<String>) {
    if !running {
        return ("—".into(), p.muted, None);
    }
    match tracker.state {
        TrackerState::Waiting => (tr("Not asked yet").into(), p.muted, None),
        TrackerState::Asking => (tr("Asking…").into(), p.muted, None),
        TrackerState::Working => (tr("Working").into(), p.accent, None),
        TrackerState::Failed => (tr("No answer").into(), p.warning, tracker.message.clone()),
        TrackerState::Refused => (
            tr("Not used").into(),
            p.warning,
            Some(
                tr("This tracker leads to this computer or its network, so it isn't asked.").into(),
            ),
        ),
        TrackerState::Off => (tr("Off").into(), p.muted, None),
    }
}

fn source_state(
    source: &Source,
    running: bool,
    p: Palette,
) -> (&'static str, Color32, Option<&'static str>) {
    if !running {
        return ("—", p.muted, None);
    }
    match source.state {
        SourceState::On if source.peers > 0 => (tr("Working"), p.accent, None),
        SourceState::On => (tr("Asking…"), p.muted, None),
        SourceState::Off => (
            tr("Off"),
            p.muted,
            Some("Turned off in Preferences → Torrents."),
        ),
        SourceState::Private => (
            tr("Not used"),
            p.muted,
            Some("Private torrents only find peers through their own trackers."),
        ),
        SourceState::Waiting => (
            tr("Waiting"),
            p.muted,
            Some("Used once the torrent's details are known."),
        ),
    }
}

/// One character per map cell for a sample: skipped files are `-`, finished pieces are
/// spread the way rarest-first leaves them, and a few cells hold pieces in progress.
#[cfg(feature = "screenshot")]
fn sample_map(files: &[(&str, u64, f64, bool)], checked: Option<f64>, busy: bool) -> String {
    const CELLS: u64 = 1024;
    let total: u64 = files.iter().map(|file| file.1).sum();
    let mut state = 7u64;
    let mut random = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % 1000) as f64 / 1000.0
    };
    (0..CELLS)
        .map(|cell| {
            let at = cell * total / CELLS;
            let mut start = 0;
            let file = files
                .iter()
                .find(|file| {
                    start += file.1;
                    at < start
                })
                .unwrap_or(&files[files.len() - 1]);
            let roll = random();
            if !file.3 {
                '-'
            } else if checked.is_some_and(|share| cell as f64 >= share * CELLS as f64) {
                '0'
            } else if file.2 >= 1.0 {
                '9'
            } else if busy && file.2 > 0.0 && roll > 0.985 {
                '*'
            } else if roll < file.2 {
                char::from(b'5' + (roll * 40.0) as u8 % 5)
            } else {
                '0'
            }
        })
        .collect()
}

#[cfg(feature = "screenshot")]
impl Desktop {
    /// A running torrent's status window for screenshots. Torrents stay off in the saved
    /// settings and the queue stays paused, so nothing here can start the helper.
    pub(super) fn torrent_status_sample(&mut self, mode: &str) {
        use download_engine::torrent::{File, Peer, Pieces, Tracker};
        use download_engine::transfer_activity::{SharingView, TorrentSnapshot};
        const MIB: u64 = 1024 * 1024;
        let files: [(&str, u64, f64, bool); 6] = [
            (
                "Big Buck Bunny/big-buck-bunny-4k.mp4",
                2_800_000_000,
                1.0,
                true,
            ),
            ("README.txt", 4_000, 1.0, true),
            ("Sintel/sintel-1080p.mkv", 1_200_000_000, 0.37, true),
            ("Sintel/sintel-subtitles.zip", 18_000_000, 0.0, true),
            (
                "Tears of Steel/tears-of-steel-1080p-with-a-very-long-file-name-that-has-to-be-cut-short.mkv",
                1_900_000_000,
                0.12,
                true,
            ),
            (
                "Tears of Steel/tears-of-steel-4k.mov",
                6_300_000_000,
                0.0,
                false,
            ),
        ];
        let sharing = mode == "status-torrent-sharing";
        let paused = mode == "status-torrent-paused";
        let checking = mode == "status-torrent-checking";
        let share = |file: &(&str, u64, f64, bool)| if sharing && file.3 { 1.0 } else { file.2 };
        let hash = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";
        let link = format!(
            "magnet:?xt=urn:btih:{hash}&dn=Blender+Open+Movies+Collection\
             &tr=udp%3A%2F%2Ftracker.example.org%3A1337%2Fannounce\
             &tr=https%3A%2F%2Ftracker.example.net%2Fannounce\
             &tr=http%3A%2F%2Fold-tracker.example.com%3A6969%2Fannounce"
        );
        let mut job = Job::new_torrent(
            &link,
            "Blender Open Movies Collection",
            self.store.paths.downloads.clone(),
            Uuid::new_v4().to_string(),
            Stage::Chosen,
        )
        .unwrap();
        job.status = Status::Paused;
        let mut job = self.store.enqueue(&job).unwrap();
        let list: Vec<File> = files
            .iter()
            .enumerate()
            .map(|(index, file)| File {
                index,
                path: file.0.into(),
                saved_as: file.0.into(),
                len: file.1,
            })
            .collect();
        let chosen: Vec<usize> = (0..files.len()).filter(|index| files[*index].3).collect();
        self.store
            .seed_torrent_preview(
                &job.id,
                &Files { list, chosen },
                &["udp://open.example.org:6969/announce"],
            )
            .unwrap();
        let wanted: u64 = files.iter().filter(|file| file.3).map(|file| file.1).sum();
        let done: u64 = files
            .iter()
            .filter(|file| file.3)
            .map(|file| (file.1 as f64 * share(file)) as u64)
            .sum();
        job.total = Some(wanted);
        job.downloaded = done;
        job.status = if sharing {
            Status::Complete
        } else if paused {
            Status::Paused
        } else {
            Status::Downloading
        };
        if sharing {
            job.saved_path = Some(job.directory.join(&job.name));
        }
        self.store.save(&job).unwrap();

        let piece = 2 * MIB;
        let total_pieces = files.iter().map(|file| file.1).sum::<u64>().div_ceil(piece) as u32;
        let wanted_pieces = wanted.div_ceil(piece) as u32;
        let have = if checking {
            (f64::from(wanted_pieces) * 0.25) as u32
        } else {
            (done / piece) as u32
        };
        let peer =
            |address: &str, client: Option<&str>, has: f64, down: u64, up: u64, incoming| Peer {
                connection: de_torrent::details::ConnectionInfo {
                    transport: if incoming {
                        de_torrent::details::PeerTransport::Utp
                    } else {
                        de_torrent::details::PeerTransport::Tcp
                    },
                    encrypted: Some(up > 0),
                },
                address: Some(address.into()),
                client: client.map(str::to_owned),
                has_pieces: (f64::from(total_pieces) * has) as u32,
                download_bps: down,
                upload_bps: up,
                downloaded_bytes: down * 300,
                uploaded_bytes: up * 300,
                incoming,
                receiving: down > 0,
                sending: up > 0,
            };
        let peers = if sharing {
            vec![
                peer(
                    "203.0.113.81:6889",
                    Some("\u{b5}Torrent 3.6.0"),
                    0.23,
                    0,
                    220 * 1024,
                    true,
                ),
                peer(
                    "[2001:db8:4::17]:49160",
                    Some("libtorrent/2.0.10"),
                    0.71,
                    0,
                    130 * 1024,
                    false,
                ),
                peer(
                    "198.51.100.33:6881",
                    Some("qBittorrent/4.6.7"),
                    0.05,
                    0,
                    60 * 1024,
                    false,
                ),
                peer(
                    "192.0.2.9:51413",
                    Some("Transmission 3.00"),
                    0.48,
                    0,
                    0,
                    false,
                ),
            ]
        } else if checking {
            Vec::new()
        } else {
            vec![
                peer(
                    "203.0.113.24:51413",
                    Some("Transmission 4.0.6"),
                    1.0,
                    1_240_000,
                    0,
                    false,
                ),
                peer(
                    "198.51.100.7:6881",
                    Some("qBittorrent/5.0.3"),
                    1.0,
                    880_000,
                    12_000,
                    false,
                ),
                peer(
                    "[2001:db8:4::17]:49160",
                    Some("libtorrent/2.0.10"),
                    0.64,
                    420_000,
                    98_000,
                    true,
                ),
                peer(
                    "192.0.2.140:55021",
                    Some("Deluge 2.1.1"),
                    1.0,
                    390_000,
                    0,
                    false,
                ),
                peer(
                    "203.0.113.81:6889",
                    Some("\u{b5}Torrent 3.6.0"),
                    0.23,
                    0,
                    143_000,
                    true,
                ),
                peer("198.51.100.200:40000", None, 1.0, 154_000, 0, false),
                peer("192.0.2.17:6881", Some("rTorrent 0.9.8"), 0.91, 0, 0, false),
            ]
        };
        let tracker = |url: &str,
                       added,
                       state,
                       numbers: Option<(u64, u64, usize)>,
                       next,
                       message: Option<&str>| Tracker {
            url: url.into(),
            added,
            state,
            message: message.map(str::to_owned),
            peers: numbers.map(|numbers| numbers.2),
            seeds: numbers.map(|numbers| numbers.0),
            leechers: numbers.map(|numbers| numbers.1),
            next_in: next,
        };
        let details = download_engine::torrent::TorrentDetails {
            private: Some(false),
            pieces: Some(Pieces {
                total: total_pieces,
                wanted: wanted_pieces,
                have,
                in_progress: if mode.contains("checking") || paused || sharing {
                    0
                } else {
                    6
                },
                checked: checking.then_some((f64::from(total_pieces) * 0.42) as u32),
                map: sample_map(
                    &files.map(|file| (file.0, file.1, share(&file), file.3)),
                    checking.then_some(0.42),
                    !checking && !paused && !sharing,
                ),
            }),
            peers,
            trackers: vec![
                tracker(
                    "udp://tracker.example.org:1337/announce",
                    false,
                    TrackerState::Working,
                    Some((212, 37, 50)),
                    Some(1704),
                    None,
                ),
                tracker(
                    "https://tracker.example.net/announce",
                    false,
                    TrackerState::Working,
                    Some((198, 41, 50)),
                    Some(845),
                    None,
                ),
                tracker(
                    "http://old-tracker.example.com:6969/announce",
                    false,
                    TrackerState::Failed,
                    None,
                    Some(100),
                    Some("timed out"),
                ),
                tracker(
                    "udp://open.example.org:6969/announce",
                    true,
                    TrackerState::Working,
                    Some((87, 12, 31)),
                    Some(1322),
                    None,
                ),
            ],
            dht: Source {
                state: SourceState::On,
                peers: 41,
            },
            pex: Source {
                state: SourceState::On,
                peers: 12,
            },
        };
        let progress: Arc<[u64]> = files
            .iter()
            .map(|file| (file.1 as f64 * if file.3 { share(file) } else { 0.0 }) as u64)
            .collect();
        let phase = if sharing {
            Phase::Seeding
        } else if checking {
            Phase::CheckingSaved
        } else {
            Phase::Downloading
        };
        if !paused {
            self.store.activity.seed_torrent_preview(
                &job.id,
                phase,
                TorrentSnapshot {
                    peers: details.peers.len(),
                    download_speed: if sharing || checking { 0 } else { 3_250_000 },
                    upload_speed: if sharing { 410 * 1024 } else { 253_000 },
                    uploaded: 310 * MIB,
                    file_progress: progress.clone(),
                    // Ratio 1.42 of 2.00 after 9 h 12 min, as in the approved mockup.
                    sharing: sharing.then_some(SharingView {
                        ratio_hundredths: 142,
                        shared_secs: 9 * 3600 + 12 * 60,
                        limit_ratio: Some(200),
                        limit_seconds: Some(86_400),
                    }),
                },
                Some(details.clone()),
            );
        }
        // Shown as with torrents on, without saving that.
        self.torrents.settings.enabled = true;
        self.open_download_status(&job);
        self.last_refresh = Instant::now();
        let window = self.download_status.windows.get_mut(&job.id).unwrap();
        window.details = true;
        window.rate = 3_250_000.0;
        window.torrents_on = true;
        window.phase = (!paused).then_some(phase);
        window.torrent.tab = if mode.contains("peers") || sharing {
            TorrentTab::Peers
        } else if mode.contains("trackers") {
            TorrentTab::Trackers
        } else {
            TorrentTab::Files
        };
        // Seed the view too: a fast capture can precede the first asynchronous
        // activity refresh. This fixture never starts a real torrent worker.
        window.torrent.details = Some(Arc::new(details));
        window.torrent.progress = Some(progress);
        window.torrent.seen = if paused {
            Instant::now().checked_sub(Duration::from_secs(180))
        } else {
            Some(Instant::now())
        };
        if mode.contains("typed") {
            window.torrent.tracker_text = "udp://tracker.example.org:6969/announce".into();
        }
    }
}
