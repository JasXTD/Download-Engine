//! The main window chrome: menus, download toolbar, categories and status bar.
use super::view::icon;
use super::*;
use egui::{Align, Align2, FontId, Layout, Sense, Stroke, pos2, vec2};

const ADD: &str = "\u{e710}";
const DOWNLOAD: &str = "\u{e896}";
const ALL: &str = "\u{e8f1}";
const QUEUE: &str = "\u{e8fd}";
const CLOCK: &str = "\u{e823}";
const CHECK: &str = "\u{e73e}";
const PLAY: &str = "\u{e768}";
const PAUSE: &str = "\u{e769}";
const STOP: &str = "\u{e71a}";
const DELETE: &str = "\u{e74d}";
const FOLDER: &str = "folder";
const CALENDAR: &str = "\u{e787}";
const SETTINGS: &str = "\u{e713}";
const AUTOMATION: &str = "\u{e945}";
const GRABBER: &str = "\u{e774}";
const LOGS: &str = "\u{e9d9}";
const WARNING: &str = "\u{e783}";

fn menu(ui: &mut egui::Ui, label: &str, content: impl FnOnce(&mut egui::Ui)) {
    let _response = ui.menu_button(label, |ui| {
        content(ui);
        #[cfg(feature = "screenshot")]
        capture_menu_contents(ui, label);
    });
    #[cfg(feature = "screenshot")]
    capture_target(ui, label, &_response.response);
}

/// Toolbar label sizes: labels shrink from the largest size to fit whole words.
const LABEL_LARGEST: f32 = 11.0;
const LABEL_SMALLEST: f32 = 9.0;
/// Horizontal space a toolbar button keeps around its label.
const LABEL_PADDING: f32 = 6.0;

/// egui may wrap between CJK characters, so such runs never force a word split.
fn breaks_between_characters(word: &str) -> bool {
    word.chars()
        .any(|c| matches!(c, '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}'))
}

/// The widest space-separated word of `label` at `size`.
fn widest_word(ui: &egui::Ui, label: &str, size: f32) -> f32 {
    ui.fonts_mut(|fonts| {
        label
            .split_whitespace()
            .map(|word| {
                fonts
                    .layout_no_wrap(word.into(), FontId::proportional(size), Color32::WHITE)
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max)
    })
}

/// Words of `label` as they can start a toolbar row. egui moves the space after a
/// full row to the start of the next one, so later words keep their leading space.
fn row_words(label: &str) -> impl Iterator<Item = String> + '_ {
    label.split(' ').enumerate().map(|(index, word)| {
        if index == 0 {
            word.to_owned()
        } else {
            format!(" {word}")
        }
    })
}

/// The text width a label needs at `size` so that egui wraps it only between words.
fn whole_word_width(ui: &egui::Ui, label: &str, size: f32) -> f32 {
    ui.fonts_mut(|fonts| {
        row_words(label)
            .filter(|word| !breaks_between_characters(word))
            .map(|word| {
                fonts
                    .layout_no_wrap(word, FontId::proportional(size), Color32::WHITE)
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max)
    })
}

/// Shortens words that cannot fit `width` with … so that a label wraps only between
/// words. Callers show the full label on hover.
fn elide_long_words(ui: &egui::Ui, label: &str, size: f32, width: f32) -> String {
    ui.fonts_mut(|fonts| {
        row_words(label)
            .map(|word| {
                if breaks_between_characters(&word) {
                    return word;
                }
                let mut job = egui::text::LayoutJob::simple_singleline(
                    word.clone(),
                    FontId::proportional(size),
                    Color32::WHITE,
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(width);
                let galley = fonts.layout_job(job);
                if galley.elided {
                    galley.rows.iter().map(|row| row.text()).collect()
                } else {
                    word
                }
            })
            .collect()
    })
}

/// Widens toolbar buttons whose longest word does not fit even at the smallest label
/// size, so long translations wrap between words instead of inside one. The width
/// comes from unused toolbar space, then from buttons with room to spare: first room
/// beyond their largest-size label, then room beyond their smallest-size label. A row
/// that already overflows (negative `free`) also gets that space back from them.
/// Each tool is (base width, widest word at the largest size, whole-word width at the
/// smallest size). When every word fits, the base widths are returned unchanged.
fn fit_tool_widths<const N: usize>(tools: &[(f32, f32, f32); N], free: f32) -> [f32; N] {
    let need = |word: f32| word + LABEL_PADDING;
    let short = |&(base, _, small): &(f32, f32, f32)| (need(small) - base).max(0.0);
    let mut widths = tools.map(|tool| tool.0);
    let missing: f32 = tools.iter().map(short).sum();
    if missing <= 0.0 {
        return widths;
    }
    let unused = free.clamp(0.0, missing);
    let mut wanted = missing - unused + (-free).max(0.0);
    let mut taken = 0.0;
    for keep_largest in [true, false] {
        let spare: Vec<f32> = tools
            .iter()
            .zip(&widths)
            .map(|(tool, width)| {
                let keep = need(if keep_largest { tool.1 } else { tool.2 });
                if short(tool) > 0.0 {
                    0.0
                } else {
                    (width - keep).max(0.0)
                }
            })
            .collect();
        let total: f32 = spare.iter().sum();
        let take = wanted.min(total);
        if take <= 0.0 {
            continue;
        }
        for (width, spare) in widths.iter_mut().zip(spare) {
            *width -= spare * take / total;
        }
        wanted -= take;
        taken += take;
    }
    // Whole words come first when there is not enough room for both.
    let granted = unused + taken.min(missing - unused);
    for (width, tool) in widths.iter_mut().zip(tools) {
        *width += short(tool) * granted / missing;
    }
    widths
}

/// The largest label size, in half points, at which `label` wraps only between whole
/// words in `text_width`. At the minimum window size this fits words such as
/// Automation/Pengumpul before wrapping at spaces, never splitting a word.
fn label_size(ui: &egui::Ui, label: &str, text_width: f32) -> f32 {
    let longest_word = widest_word(ui, label, LABEL_LARGEST);
    let mut size = ((LABEL_LARGEST * (text_width / longest_word.max(1.0)).min(1.0) * 2.0).floor()
        / 2.0)
        .clamp(LABEL_SMALLEST, LABEL_LARGEST);
    // The estimate ignores the space a wrapped word may carry; step down until it fits.
    while size > LABEL_SMALLEST && whole_word_width(ui, label, size) > text_width {
        size -= 0.5;
    }
    size
}

/// A toolbar label laid out in up to two centred rows, and whether it was shortened.
fn label_galley(
    ui: &egui::Ui,
    label: &str,
    size: f32,
    text_width: f32,
    color: Color32,
) -> (std::sync::Arc<egui::Galley>, bool) {
    let shown = elide_long_words(ui, label, size, text_width);
    let mut job =
        egui::text::LayoutJob::simple(shown.clone(), FontId::proportional(size), color, text_width);
    job.halign = Align::Center;
    job.wrap.max_rows = 2;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let shortened = galley.elided || shown != label;
    (galley, shortened)
}

/// How every label in one toolbar row is drawn: one size, and one top line below the
/// icons, so one- and two-line labels start together.
#[derive(Clone, Copy)]
struct LabelLook {
    size: f32,
    top: f32,
}

/// The shared look for (label, button width) pairs: the smallest size any label needs,
/// with the row's tallest label centred where a single line sits.
fn row_look(ui: &egui::Ui, labels: &[(&str, f32)]) -> LabelLook {
    let size = labels
        .iter()
        .map(|&(label, width)| label_size(ui, label, width - LABEL_PADDING))
        .fold(LABEL_LARGEST, f32::min);
    let height = labels
        .iter()
        .map(|&(label, width)| {
            label_galley(ui, label, size, width - LABEL_PADDING, Color32::WHITE)
                .0
                .size()
                .y
        })
        .fold(0.0, f32::max);
    LabelLook {
        size,
        top: TOOL_HEIGHT - 13.0 - height / 2.0,
    }
}

const TOOL_HEIGHT: f32 = 64.0;

fn tool(
    ui: &mut egui::Ui,
    label: &str,
    glyph: &str,
    color: Color32,
    width: f32,
    active: bool,
    look: LabelLook,
) -> egui::Response {
    let response = ui.add_sized(
        [width, TOOL_HEIGHT],
        egui::Button::new("")
            .frame(ui.is_enabled())
            .frame_when_inactive(active)
            .selected(active),
    );
    #[cfg(feature = "screenshot")]
    capture_target(ui, label, &response);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let color = if ui.is_enabled() {
        color
    } else {
        color.gamma_multiply(0.35)
    };
    icon(
        ui,
        pos2(response.rect.center().x, response.rect.top() + 20.0),
        glyph,
        22.0,
        color,
    );
    let (galley, shortened) = label_galley(ui, label, look.size, width - LABEL_PADDING, color);
    let at = pos2(response.rect.center().x, response.rect.top() + look.top);
    ui.painter().galley(at, galley, color);
    if shortened {
        response.on_hover_text(label).on_disabled_hover_text(label)
    } else {
        response
    }
}

#[cfg(feature = "screenshot")]
pub(super) fn capture_target(ui: &egui::Ui, label: &str, response: &egui::Response) {
    if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
        && std::env::var("IDM_EX_TEST_MENU").is_ok_and(|path| path.split('|').next() == Some(label))
    {
        let popup = egui::Popup::default_response_id(response);
        if !egui::Popup::is_id_open(ui.ctx(), popup) {
            egui::Popup::open_id(ui.ctx(), popup);
        }
    }
    if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
        && std::env::var("IDM_EX_TEST_TOOLBAR").as_deref() == Ok(label)
    {
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new("toolbar-capture-target"),
                (response.id, response.rect),
            );
        });
    }
}

#[cfg(feature = "screenshot")]
pub(super) fn capture_menu_contents(ui: &egui::Ui, label: &str) {
    if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_none() {
        return;
    }
    let Ok(path) = std::env::var("IDM_EX_TEST_MENU") else {
        return;
    };
    let parts: Vec<_> = path.split('|').collect();
    let Some(depth) = parts.iter().position(|part| *part == label) else {
        return;
    };
    let Some(next) = parts.get(depth + 1) else {
        return;
    };
    let rect = ui.ctx().graphics(|graphics| {
        graphics.get(ui.layer_id()).and_then(|list| {
            list.all_entries()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == *next => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .last()
        })
    });
    if let Some(rect) = rect {
        let frame = ui.ctx().cumulative_frame_nr();
        ui.ctx().data_mut(|data| {
            let target = data
                .get_temp_mut_or_insert_with(egui::Id::new("menu-capture-target"), || {
                    (rect, 0_usize, frame)
                });
            // A child popup is drawn before its parent finishes; keep the deeper target.
            if target.2 != frame || target.1 <= depth {
                *target = (rect, depth + 1, frame);
            }
        });
    }
}

pub(super) fn category_icon(category: Category) -> &'static str {
    match category {
        Category::Programs => "app",
        Category::Video => "\u{e714}",
        Category::Music => "\u{e8d6}",
        Category::Documents => "\u{e8a5}",
        Category::Compressed => "archive",
        Category::Other => "\u{e7c3}",
    }
}

fn navigation(
    ui: &mut egui::Ui,
    label: &str,
    glyph: &str,
    suffix: &str,
    active: bool,
    labels: bool,
    p: Palette,
) -> bool {
    let row_height = if ui.ctx().content_rect().height() < 700.0 {
        22.0
    } else {
        28.0
    };
    let response = ui.add_sized(
        [ui.available_width(), row_height],
        egui::Button::new("")
            .fill(if active {
                p.selected
            } else {
                Color32::TRANSPARENT
            })
            .stroke(Stroke::NONE)
            .corner_radius(4),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, ui.is_enabled(), label)
    });
    let color = if active { p.blue } else { p.text };
    icon(
        ui,
        pos2(response.rect.left() + 15.0, response.rect.center().y),
        glyph,
        16.0,
        color,
    );
    if labels {
        // Counts stay whole; a long label ends with … before them and shows in full
        // on hover.
        let count = ui.painter().text(
            pos2(response.rect.right() - 8.0, response.rect.center().y),
            Align2::RIGHT_CENTER,
            suffix,
            FontId::proportional(11.0),
            p.muted,
        );
        let label_right = if suffix.is_empty() {
            response.rect.right() - 8.0
        } else {
            count.left() - 6.0
        };
        let mut job = egui::text::LayoutJob::simple_singleline(
            label.into(),
            FontId::proportional(12.0),
            color,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(
            (label_right - response.rect.left() - 32.0).max(0.0),
        );
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(
            pos2(
                response.rect.left() + 32.0,
                response.rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            color,
        );
    }
    response.on_hover_text(label).clicked()
}

impl Desktop {
    fn selected_job(&self) -> Option<Job> {
        if self.selected_jobs().len() != 1 {
            return None;
        }
        self.jobs
            .iter()
            .find(|job| self.selected.as_deref() == Some(&job.id))
            .cloned()
    }

    fn pause_all(&mut self) {
        for queue in self.queues.clone() {
            self.command(Command::PauseQueue(queue.id));
        }
    }

    fn clear_finished(&mut self) {
        for job in self
            .jobs
            .clone()
            .into_iter()
            .filter(|job| job.status == Status::Complete)
        {
            self.command(Command::Remove(job.id));
        }
    }

    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui, p: Palette) {
        egui::Panel::top("application-menu")
            .exact_size(30.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.background)
                    .inner_margin(egui::Margin::symmetric(14, 2)),
            )
            .show(ui, |ui| {
                egui::MenuBar::new()
                    .style(|style: &mut egui::Style| {
                        theme::menu_style(style);
                        style.spacing.button_padding = vec2(6.0, 0.0);
                    })
                    .config(theme::menu_config())
                    .ui(ui, |ui| {
                        menu(ui, tr("Tasks"), |ui| {
                            if ui
                                .add(egui::Button::new(tr("Add URL")).shortcut_text("Ctrl+N"))
                                .clicked()
                            {
                                self.open_add();
                                ui.close();
                            }
                            if ui.button(tr("Add batch")).clicked() {
                                self.open_queue_dialog(Dialog::Batch);
                                ui.close();
                            }
                            ui.separator();
                            if ui.add_enabled(
                                self.selected_jobs().iter().any(|job| matches!(job.status, Status::Downloading | Status::Queued)),
                                egui::Button::new(tr("Stop selected")),
                            ).on_hover_text(tr("Stop the selected downloads and close their status windows. Saved progress is kept.")).clicked() {
                                self.stop_selected_downloads();
                                ui.close();
                            }
                            if ui.button(tr("Stop all")).on_hover_text(tr("Stop current and waiting downloads in every queue, including files started immediately. Saved progress is kept; resume files when ready.")).clicked() {
                                self.stop_all_downloads();
                                ui.close();
                            }
                            if ui.add_enabled(self.queues.iter().any(|q| q.running || q.has_schedule()), egui::Button::new(tr("Pause all")))
                                .on_hover_text(tr("Pause every queue and its active downloads. Start the queues again to continue their waiting work.")).clicked() {
                                self.pause_all();
                                ui.close();
                            }
                            ui.separator();
                            if ui.button(tr("New queue")).clicked() {
                                self.queue_name.clear();
                                self.open_queue_dialog(Dialog::NewQueue);
                                ui.close();
                            }
                            if ui.button(tr("Manage queues")).clicked() {
                                self.view = View::Queues;
                                ui.close();
                            }
                            ui.separator();
                            if ui
                                .add_enabled(!self.list_transfer.busy(), egui::Button::new(tr("Export list…")))
                                .on_hover_text(tr("Save your downloads to a file to back them up or move them to another PC."))
                                .clicked()
                            {
                                self.open_list_export();
                                ui.close();
                            }
                            if ui
                                .add_enabled(!self.list_transfer.busy(), egui::Button::new(tr("Import list…")))
                                .on_hover_text(tr("Add downloads from a Download Engine list or a text file of links."))
                                .clicked()
                            {
                                self.open_list_import();
                                ui.close();
                            }
                            ui.separator();
                            if ui
                                .add_enabled(
                                    self.tray_available(),
                                    egui::Button::new(tr("Hide to tray")),
                                )
                                .clicked()
                            {
                                self.hide_to_tray(ui.ctx());
                                ui.close();
                            }
                            if ui.button(tr("Exit")).clicked() {
                                self.exit(ui.ctx());
                                ui.close();
                            }
                        });
                        ui.add(
                            egui::Separator::default()
                                .vertical()
                                .spacing(8.0)
                                .shrink(3.0),
                        );
                        menu(ui, tr("View"), |ui| {
                            for (view, label) in [
                                (View::Downloads, tr("Download list")),
                                (View::Queues, tr("Manage queues")),
                                (View::Scheduler, tr("Schedules")),
                            ] {
                                if ui.selectable_label(self.view == view, label).clicked() {
                                    self.view = view;
                                    ui.close();
                                }
                            }
                            ui.separator();
                            self.drop_target_menu_item(ui);
                        });
                        ui.add(
                            egui::Separator::default()
                                .vertical()
                                .spacing(8.0)
                                .shrink(3.0),
                        );
                        menu(ui, tr("Tools"), |ui| {
                            if ui.button(tr("Preferences")).clicked() {
                                self.open_preferences_window();
                                ui.close();
                            }
                            ui.separator();
                            if ui.selectable_label(self.view == View::Automation, tr("Scripting")).clicked() {
                                self.view = View::Automation;
                                self.refresh_automation();
                                ui.close();
                            }
                            if ui.button(tr("Antivirus Scan")).clicked() {
                                self.open_antivirus_scan(antivirus::ScanMode::Single);
                                ui.close();
                            }
                            ui.separator();
                            if ui.button(tr("Browser setup")).clicked() {
                                self.open_browser_setup();
                                ui.close();
                            }
                            if ui.button(tr("MCP control")).clicked() {
                                self.open_mcp();
                                ui.close();
                            }
                            if ui.button(tr("Download folders")).clicked() {
                                self.open_download_folders();
                                ui.close();
                            }
                            if ui.button(tr("Speed limits")).clicked() {
                                self.open_speed_limits();
                                ui.close();
                            }
                            ui.separator();
                            if ui.button(tr("Preview video")).clicked() {
                                self.open_hls("");
                                ui.close();
                            }
                            if ui.button(tr("Website grabber")).clicked() {
                                self.open_grabber();
                                ui.close();
                            }
                        });
                        ui.add(
                            egui::Separator::default()
                                .vertical()
                                .spacing(8.0)
                                .shrink(3.0),
                        );
                        menu(ui, tr("Help"), |ui| {
                            if ui
                                .button(tr("Guide"))
                                .on_hover_text(tr("How to use Download Engine (F1)"))
                                .clicked()
                            {
                                self.open_guide("Getting started");
                                ui.close();
                            }
                            if ui.button(tr("Changelog")).clicked() {
                                self.view = View::Changelog;
                                ui.close();
                            }
                            if ui.button(tr("Check for updates")).clicked() {
                                self.open_updates();
                                ui.close();
                            }
                            if ui.button(tr("About")).clicked() {
                                self.dialog = Some(Dialog::About);
                                ui.close();
                            }
                        });
                    });
            });
    }

    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui, p: Palette) {
        egui::Panel::top("download-toolbar").exact_size(78.0).resizable(false)
            .frame(egui::Frame::new().fill(p.background).inner_margin(egui::Margin::symmetric(10, 7)))
            .show(ui, |ui| {
                // Keep all navigation/view tools equal width while fitting 860px minimum windows.
                let width = ((ui.available_width() - 38.0) / 14.35).clamp(56.0, 76.0);
                // Use one label size across the row, including the wider file-action buttons.
                let label_size = if width < 60.0 { 11.0 } else if width < 64.0 { 11.5 } else { 12.0 };
                ui.style_mut().text_styles.insert(egui::TextStyle::Button, FontId::proportional(label_size));
                let selected = self.selected_job();
                let selection = self.selected_jobs();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    // Base widths in toolbar order. Resume measures both of its labels so
                    // the row does not shift when the selection changes.
                    let labels: [(&[&str], f32); 15] = [
                        (&[tr("Add URL")], width - 6.0),
                        (&[tr("Resume"), tr("Start again")], width - 1.0),
                        (&[tr("Pause")], width - 14.0),
                        (&[tr("Stop")], width - 16.0),
                        (&[tr("Stop all")], width - 10.0),
                        (&[tr("Remove")], width - 10.0),
                        (&[tr("Open folder")], width + 6.0),
                        (&[tr("Clear finished")], width + 12.0),
                        (&[tr("Queues")], width),
                        (&[tr("Scheduler")], width),
                        (&[tr("Scripting")], width),
                        (&[tr("Grabber")], width),
                        (&["MCP"], width),
                        (&[tr("Preferences")], width),
                        (&[tr("Logs")], width),
                    ];
                    let tools = labels.map(|(labels, base)| {
                        let largest = labels.iter().map(|label| widest_word(ui, label, LABEL_LARGEST)).fold(0.0_f32, f32::max);
                        let smallest = labels.iter().map(|label| whole_word_width(ui, label, LABEL_SMALLEST)).fold(0.0_f32, f32::max);
                        (base, largest, smallest)
                    });
                    // Three 4-point separators and the spacing between all 18 items.
                    let used = tools.iter().map(|tool| tool.0).sum::<f32>() + 3.0 * 4.0 + 17.0 * ui.spacing().item_spacing.x;
                    let widths = fit_tool_widths(&tools, ui.available_width() - used);
                    // One label size and top line for the whole row; Resume counts both labels
                    // so the row does not change when the selection does.
                    let look = row_look(ui, &labels.iter().zip(widths).flat_map(|((labels, _), width)| labels.iter().map(move |label| (*label, width))).collect::<Vec<_>>());
                    let [add, resume, pause, stop, stop_all, remove, open, clear, queues, scheduler, automation, grabber, mcp, settings, logs] = widths;
                    if tool(ui, tr("Add URL"), ADD, p.accent, add, false, look).on_hover_text(tr("Add a download (Ctrl+N)")).clicked() { self.open_add(); }
                    ui.add(egui::Separator::default().vertical().spacing(4.0));
                    let can_resume = selection.iter().any(|j| matches!(j.status, Status::Paused | Status::Failed));
                    ui.add_enabled_ui(can_resume, |ui| {
                        let label = selected.as_ref().filter(|_| can_resume).map(resume_action).unwrap_or(tr("Resume"));
                        if tool(ui, label, PLAY, p.accent, resume, false, look).on_hover_text(selected.as_ref().map(resume_description).unwrap_or(if can_resume { tr("Resume selected downloads; files without a resume plan start again") } else { tr("Select a paused download") })).clicked()
                            { self.selection_command(false); }
                    });
                    ui.add_enabled_ui(selection.iter().any(|j| matches!(j.status, Status::Downloading | Status::Queued)), |ui| {
                        if tool(ui, tr("Pause"), PAUSE, p.orange, pause, false, look).clicked() { self.selection_command(true); }
                        if tool(ui, tr("Stop"), STOP, p.warning, stop, false, look).on_hover_text(tr("Stop the selected downloads and close their status windows. Saved progress is kept.")).clicked() { self.stop_selected_downloads(); }
                    });
                    if tool(ui, tr("Stop all"), STOP, p.warning, stop_all, false, look).on_hover_text(tr("Stop current and waiting downloads in every queue, including files started immediately. Saved progress is kept; resume files when ready.")).clicked() { self.stop_all_downloads(); }
                    ui.add(egui::Separator::default().vertical().spacing(4.0));
                    ui.add_enabled_ui(!selection.is_empty() && selection.iter().all(|j| j.status != Status::Downloading), |ui| {
                        if tool(ui, tr("Remove"), DELETE, p.text, remove, false, look).on_hover_text(tr("Remove the selected entry and its partial data. Completed files are kept.")).clicked()
                            { self.request_destructive(DestructiveAction::Remove, selection.clone()); }
                    });
                    ui.add_enabled_ui(selected.is_some(), |ui| {
                        if tool(ui, tr("Open folder"), FOLDER, p.text, open, false, look).clicked() && let Some(job) = &selected { self.show_folder(job); }
                    });
                    ui.add_enabled_ui(self.jobs.iter().any(|j| j.status == Status::Complete), |ui| {
                        if tool(ui, tr("Clear finished"), CHECK, p.text, clear, false, look).on_hover_text(tr("Clear completed entries from the selected queue. Saved files are kept.")).clicked() { self.clear_finished(); }
                    });
                    ui.add(egui::Separator::default().vertical().spacing(4.0));
                    if tool(ui, tr("Queues"), QUEUE, p.text, queues, self.view == View::Queues, look).on_hover_text(tr("Manage download queues")).clicked() {
                        self.view = View::Queues;
                        self.refresh_queue_summaries();
                    }
                    if tool(ui, tr("Scheduler"), CALENDAR, p.text, scheduler, false, look).clicked() { self.open_queue_dialog(Dialog::Schedule); }
                    if tool(ui, tr("Scripting"), AUTOMATION, p.text, automation, self.view == View::Automation, look).on_hover_text(tr("Rules, scripts, site readers and run history")).clicked() {
                        self.view = View::Automation;
                        self.refresh_automation();
                    }
                    if tool(ui, tr("Grabber"), GRABBER, p.text, grabber, self.dialog == Some(Dialog::Grabber), look).on_hover_text(tr("Find website files or save an offline website")).clicked() {
                        self.open_grabber();
                    }
                    if tool(ui, "MCP", "\u{e839}", p.text, mcp, self.dialog == Some(Dialog::Mcp), look).on_hover_text(tr("AI connection settings and server controls")).clicked() {
                        self.open_mcp();
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if tool(ui, tr("Preferences"), SETTINGS, p.text, settings, self.preferences_window.is_some(), look).clicked() { self.open_preferences_window(); }
                        if tool(ui, tr("Logs"), LOGS, p.text, logs, self.view == View::Logs, look).on_hover_text(tr("Application logs and diagnostics")).clicked() {
                            self.view = View::Logs;
                            self.logs.request_refresh();
                        }
                    });
                });
            });
    }

    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui, p: Palette) {
        let labels = self.interface.sidebar_labels;
        let small = ui.ctx().content_rect().height() < 700.0;
        egui::Panel::left("navigation")
            .exact_size(if labels { 182.0 } else { 52.0 })
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.background)
                    .inner_margin(egui::Margin::symmetric(8, if small { 6 } else { 14 })),
            )
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = if small { 1.5 } else { 2.0 };
                if small {
                    ui.spacing_mut().button_padding.y = 2.0;
                }
                egui::ScrollArea::vertical()
                    .id_salt("download-categories")
                    .show(ui, |ui| {
                        if labels {
                            ui.label(RichText::new(tr("Downloads")).small().color(p.muted));
                            ui.add_space(if small { 3.0 } else { 6.0 });
                        }
                        for (filter, glyph) in [
                            (Filter::All, ALL),
                            (Filter::Downloading, DOWNLOAD),
                            (Filter::Queued, QUEUE),
                            (Filter::Unfinished, CLOCK),
                            (Filter::Complete, CHECK),
                            (Filter::Failed, WARNING),
                        ] {
                            let count = self
                                .jobs
                                .iter()
                                .filter(|job| {
                                    filter.accepts(job.status)
                                        && self.accepts_category(job)
                                        && job
                                            .name
                                            .to_lowercase()
                                            .contains(&self.search.trim().to_lowercase())
                                })
                                .count();
                            if navigation(
                                ui,
                                filter.title(),
                                glyph,
                                &count.to_string(),
                                self.view == View::Downloads && self.filter == filter,
                                labels,
                                p,
                            ) {
                                self.filter = filter;
                                self.view = View::Downloads;
                                self.reconcile_selection();
                            }
                        }
                        ui.add_space(if small { 6.0 } else { 14.0 });
                        if labels {
                            ui.label(RichText::new(tr("File types")).small().color(p.muted));
                            ui.add_space(if small { 3.0 } else { 6.0 });
                        }
                        for category in [
                            Category::Programs,
                            Category::Video,
                            Category::Music,
                            Category::Documents,
                            Category::Compressed,
                            Category::Other,
                        ] {
                            let count = self
                                .jobs
                                .iter()
                                .filter(|job| {
                                    job.category() == category
                                        && self.filter.accepts(job.status)
                                        && job
                                            .name
                                            .to_lowercase()
                                            .contains(&self.search.trim().to_lowercase())
                                })
                                .count();
                            if navigation(
                                ui,
                                tr(category.label()),
                                category_icon(category),
                                &count.to_string(),
                                self.view == View::Downloads && self.category == Some(category),
                                labels,
                                p,
                            ) {
                                self.category = if self.category == Some(category) {
                                    None
                                } else {
                                    Some(category)
                                };
                                self.view = View::Downloads;
                                self.reconcile_selection();
                            }
                        }
                        ui.add_space(if small { 6.0 } else { 14.0 });
                        if labels {
                            ui.label(RichText::new(tr("Queues")).small().color(p.muted));
                            ui.add_space(if small { 3.0 } else { 6.0 });
                        }
                        for queue in self.queues.clone() {
                            if navigation(
                                ui,
                                display_queue_name(&queue.id, &queue.name),
                                QUEUE,
                                if queue.running { "●" } else { "" },
                                self.view == View::Downloads && self.queue_id == queue.id,
                                labels,
                                p,
                            ) {
                                self.view = View::Downloads;
                                if self.queue_id != queue.id {
                                    self.queue_id = queue.id;
                                    self.set_selection(None);
                                    self.force_refresh();
                                }
                            }
                        }
                        if navigation(
                            ui,
                            tr("Manage queues"),
                            SETTINGS,
                            "",
                            self.view == View::Queues,
                            labels,
                            p,
                        ) {
                            self.view = View::Queues;
                        }
                    });
            });
    }

    pub(super) fn footer(&mut self, ui: &mut egui::Ui, p: Palette) {
        egui::Panel::bottom("status")
            .exact_size(34.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.background)
                    .inner_margin(egui::Margin::symmetric(16, 5)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let count = self
                        .jobs
                        .iter()
                        .filter(|j| j.status == Status::Downloading)
                        .count();
                    let rate = self.jobs.iter().map(|job| self.rate(job)).sum::<f64>();
                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 20.0), Sense::hover());
                    ui.painter().circle_filled(
                        r.center(),
                        3.0,
                        if count > 0 { p.accent } else { p.muted },
                    );
                    ui.label(RichText::new(tf!("{count} downloading", count = count)).small())
                        .on_hover_text(self.summary());
                    ui.label(
                        RichText::new(format!("{}/s", bytes(rate as u64)))
                            .small()
                            .color(p.muted),
                    );
                    if let Some(at) = self.quota_status.resumes_at()
                        && ui
                            .small_button(
                                RichText::new(tf!(
                                    "Data quota reached · Resets {}",
                                    quotas::reset_label(at)
                                ))
                                .color(p.warning),
                            )
                            .clicked()
                    {
                        self.open_data_quotas();
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(
                                    self.limits
                                        .global
                                        .map(speed_limit_label)
                                        .unwrap_or_else(|| tr("No speed limit").into()),
                                )
                                .small()
                                .color(p.muted),
                            )
                            .frame(false),
                        )
                        .on_hover_text(tr("Speed limits"))
                        .clicked()
                    {
                        self.open_speed_limits();
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(if self.view == View::Logs {
                                tr("Local logs").into()
                            } else {
                                match self.selected_jobs().len() {
                                    0 => tr("No selection").into(),
                                    count => tf!("{count} selected", count = count),
                                }
                            })
                            .small()
                            .color(p.muted),
                        );
                    });
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::{fixture, frame};
    use super::*;
    use download_engine::i18n::Language;
    use std::sync::Arc;

    const SIZES: [egui::Vec2; 2] = [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 800.0)];
    /// The sidebar panel width when labels are shown, and its inner margin.
    const SIDEBAR: f32 = 182.0;
    const SIDEBAR_MARGIN: f32 = 8.0;
    /// The toolbar sits below the 30-point menu bar and is 78 points tall.
    const TOOLBAR: std::ops::Range<f32> = 30.0..108.0;

    /// Painted, non-empty text as (screen rect, galley). Icon-only buttons paint
    /// empty captions, which are skipped.
    fn texts(out: &egui::FullOutput) -> Vec<(egui::Rect, Arc<egui::Galley>)> {
        out.shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if !text.galley.job.text.is_empty() => Some((
                    text.galley.rect.translate(text.pos.to_vec2()),
                    text.galley.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    fn shown(galley: &egui::Galley) -> String {
        galley.rows.iter().map(|row| row.text()).collect()
    }

    /// A galley may only wrap between words: a space ends one row or starts the next.
    fn assert_whole_words(galley: &egui::Galley) {
        for pair in galley.rows.windows(2) {
            let (end, start) = (pair[0].text(), pair[1].text());
            assert!(
                end.ends_with(char::is_whitespace) || start.starts_with(char::is_whitespace),
                "Word split across lines: {end:?} / {start:?}"
            );
        }
    }

    #[test]
    fn long_translations_keep_toolbar_words_whole_and_sidebar_labels_in_their_rows() {
        let (_temp, mut app, ctx) = fixture();
        app.interface.sidebar_labels = true;
        for language in [Language::English, Language::German, Language::Russian] {
            app.interface.language = language;
            let _scope = download_engine::i18n::scope(language);
            for size in SIZES {
                frame(&mut app, &ctx, size, vec![]);
                let out = frame(&mut app, &ctx, size, vec![]);
                let texts = texts(&out);
                let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);

                // Toolbar labels in visual order; Preferences is the rightmost tool.
                let resume = app
                    .selected_job()
                    .map_or(tr("Resume"), |job| resume_action(&job));
                let mut previous: Option<egui::Rect> = None;
                for label in [
                    tr("Add URL"),
                    resume,
                    tr("Pause"),
                    tr("Stop"),
                    tr("Stop all"),
                    tr("Remove"),
                    tr("Open folder"),
                    tr("Clear finished"),
                    tr("Queues"),
                    tr("Scheduler"),
                    tr("Scripting"),
                    tr("Grabber"),
                    "MCP",
                    tr("Logs"),
                    tr("Preferences"),
                ] {
                    let (rect, galley) = texts
                        .iter()
                        .find(|(rect, galley)| {
                            galley.job.text == label
                                && rect.top() >= TOOLBAR.start
                                && rect.bottom() <= TOOLBAR.end
                        })
                        .unwrap_or_else(|| {
                            panic!("{language:?} {size:?}: toolbar label missing: {label}")
                        });
                    assert!(
                        !galley.elided && shown(galley) == label,
                        "{language:?} {size:?}: toolbar label shortened: {label}"
                    );
                    assert_whole_words(galley);
                    let font = galley.job.sections[0].format.font_id.size;
                    assert!(
                        (LABEL_SMALLEST..=LABEL_LARGEST).contains(&font),
                        "{language:?} {size:?}: {label} uses {font} pt"
                    );
                    assert!(
                        viewport.contains_rect(*rect),
                        "Toolbar label clipped: {label}"
                    );
                    if let Some(previous) = previous {
                        assert!(
                            previous.right() + 4.0 < rect.left(),
                            "{language:?} {size:?}: toolbar labels touch at {label}"
                        );
                    }
                    previous = Some(*rect);
                }

                let filters = [
                    Filter::All,
                    Filter::Downloading,
                    Filter::Queued,
                    Filter::Unfinished,
                    Filter::Complete,
                    Filter::Failed,
                ]
                .map(Filter::title);
                let categories = [
                    Category::Programs,
                    Category::Video,
                    Category::Music,
                    Category::Documents,
                    Category::Compressed,
                    Category::Other,
                ]
                .map(|category| tr(category.label()));
                let mut shortened = Vec::new();
                for label in filters
                    .into_iter()
                    .chain(categories)
                    .chain([tr("Main queue"), tr("Manage queues")])
                {
                    let (rect, galley) = texts
                        .iter()
                        .find(|(rect, galley)| {
                            galley.job.text == label
                                && rect.right() < SIDEBAR
                                && rect.top() >= TOOLBAR.end
                        })
                        .unwrap_or_else(|| {
                            panic!("{language:?} {size:?}: sidebar label missing: {label}")
                        });
                    let count = texts.iter().find(|(count, galley)| {
                        galley.job.text != label
                            && count.left() > rect.left()
                            && count.left() < SIDEBAR
                            && (count.center().y - rect.center().y).abs() < 2.0
                    });
                    let row_right = SIDEBAR - SIDEBAR_MARGIN;
                    match count {
                        Some((count, _)) => {
                            assert!(
                                rect.right() <= count.left() - 5.5,
                                "{language:?} {size:?}: sidebar label runs into its count: {label}"
                            );
                            assert!(
                                count.right() <= row_right,
                                "{language:?} {size:?}: count clipped beside {label}"
                            );
                        }
                        None => assert!(
                            rect.right() <= row_right - 7.5,
                            "{language:?} {size:?}: sidebar label leaves its row: {label}"
                        ),
                    }
                    assert_eq!(galley.rows.len(), 1, "Sidebar labels stay on one line");
                    if galley.elided {
                        let kept = shown(galley);
                        let kept = kept.strip_suffix('…').expect("Shortened labels end with …");
                        assert!(
                            !kept.is_empty() && label.starts_with(kept),
                            "{label}: {kept}"
                        );
                        shortened.push(label);
                    } else {
                        assert_eq!(shown(galley), label);
                    }
                }
                match language {
                    Language::English => assert!(shortened.is_empty(), "{shortened:?}"),
                    Language::German => {
                        for label in ["Wird heruntergeladen", "Warteschlangen verwalten"] {
                            assert!(shortened.contains(&label), "{size:?}: {shortened:?}");
                        }
                    }
                    _ => assert!(
                        shortened.contains(&"Управление очередями"),
                        "{size:?}: {shortened:?}"
                    ),
                }
            }
        }
    }

    #[test]
    fn shortened_sidebar_labels_show_their_full_text_on_hover() {
        let (_temp, mut app, ctx) = fixture();
        app.interface.sidebar_labels = true;
        app.interface.language = Language::German;
        let _scope = download_engine::i18n::scope(Language::German);
        let size = SIZES[0];
        frame(&mut app, &ctx, size, vec![]);
        let out = frame(&mut app, &ctx, size, vec![]);
        let label = tr("Manage queues");
        let (rect, galley) = texts(&out)
            .into_iter()
            .find(|(rect, galley)| galley.job.text == label && rect.right() < SIDEBAR)
            .unwrap();
        assert!(galley.elided);
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerMoved(rect.center())],
        );
        // Each frame advances egui's clock by 1/60 s; tooltips wait 0.5 s.
        assert!((0..90).any(|_| {
            texts(&frame(&mut app, &ctx, size, vec![]))
                .iter()
                .any(|(_, galley)| galley.job.text == label && !galley.elided)
        }));
        assert_eq!(app.view, View::Downloads, "Hovering must not navigate");
    }

    #[test]
    fn toolbar_words_too_long_for_their_button_end_with_ellipsis_and_show_on_hover() {
        let ctx = egui::Context::default();
        configure(&ctx);
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        let label = "Donaudampfschifffahrt Kapitänsmütze";
        let run = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let look = row_look(ui, &[(label, 44.0)]);
                    tool(ui, label, ADD, Color32::WHITE, 44.0, false, look);
                },
            );
            output.textures_delta.clear();
            output
        };
        run(vec![]);
        let out = run(vec![]);
        let (rect, galley) = texts(&out)
            .into_iter()
            .find(|(_, galley)| galley.job.text.starts_with("Donau"))
            .unwrap();
        assert_whole_words(&galley);
        assert!(rect.width() <= 44.0 - LABEL_PADDING + 0.5);
        let words: Vec<_> = label.split(' ').collect();
        for word in shown(&galley).split_whitespace() {
            let kept = word.strip_suffix('…').expect("Long words end with …");
            assert!(
                words
                    .iter()
                    .any(|whole| !kept.is_empty() && whole.starts_with(kept))
            );
        }
        run(vec![egui::Event::PointerMoved(rect.center())]);
        assert!((0..90).any(|_| {
            texts(&run(vec![]))
                .iter()
                .any(|(_, galley)| galley.job.text == label && !galley.elided)
        }));
    }

    #[test]
    fn tool_widths_change_only_to_fit_whole_words() {
        // (base width, widest word at the largest size, at the smallest size)
        let fitting = [(50.0, 40.0, 30.0), (60.0, 20.0, 15.0)];
        assert_eq!(fit_tool_widths(&fitting, 100.0), [50.0, 60.0]);
        assert_eq!(fit_tool_widths(&fitting, -20.0), [50.0, 60.0]);

        let tools = [(40.0, 50.0, 45.0), (80.0, 30.0, 25.0), (60.0, 30.0, 25.0)];
        assert_eq!(fit_tool_widths(&tools, 20.0), [51.0, 80.0, 60.0]);
        let borrowed = fit_tool_widths(&tools, 0.0);
        assert!((borrowed[0] - 51.0).abs() < 0.01);
        assert!((borrowed.iter().sum::<f32>() - 180.0).abs() < 0.01);
        // Donors keep room for their largest label size.
        assert!(borrowed[1] >= 36.0 && borrowed[2] >= 36.0);
        // A rebalanced row that overflowed by 5 points fits afterwards.
        let overflowing = fit_tool_widths(&tools, -5.0);
        assert!((overflowing[0] - 51.0).abs() < 0.01);
        assert!((overflowing.iter().sum::<f32>() - 175.0).abs() < 0.01);
        // An overflow alone does not change a row whose words fit.
        assert_eq!(fit_tool_widths(&fitting, -5.0), [50.0, 60.0]);

        // Without enough room, take what is spare down to the smallest size, and
        // fit words before recovering an overflow.
        let crowded = fit_tool_widths(&[(30.0, 60.0, 50.0), (40.0, 30.0, 30.0)], 0.0);
        assert!((crowded[0] - 34.0).abs() < 0.01 && (crowded[1] - 36.0).abs() < 0.01);
        let crowded = fit_tool_widths(&[(30.0, 60.0, 50.0), (40.0, 30.0, 30.0)], -3.0);
        assert!((crowded[0] - 34.0).abs() < 0.01 && (crowded[1] - 36.0).abs() < 0.01);
    }
}
