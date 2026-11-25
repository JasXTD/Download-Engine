use super::*;
use download_engine::audio_conversion::{Format, Mp3Bitrate, convert_with_bitrate};
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub(super) const GLYPH: &str = "convert";

#[derive(Default)]
pub(super) struct Converter {
    format: Format,
    bitrate: Mp3Bitrate,
    rows: Vec<Row>,
    folder: Option<PathBuf>,
    running: Option<Conversion>,
    error: Option<String>,
}

struct Row {
    source: PathBuf,
    output: PathBuf,
    status: FileStatus,
}

#[derive(Debug, PartialEq, Eq)]
enum FileStatus {
    Ready,
    Saved,
    Failed(String),
    Cancelled,
}

struct Conversion {
    index: usize,
    cancel: CancellationToken,
    task: background_io::Task<()>,
}

impl Drop for Conversion {
    fn drop(&mut self) {
        self.cancel.cancel(); // Task joins the worker during shutdown.
    }
}

impl Converter {
    fn select_sources(&mut self, files: Vec<PathBuf>) {
        let mut seen = HashSet::new();
        let paths: Vec<_> = files
            .into_iter()
            .filter(|p| seen.insert(p.clone()))
            .collect();
        if paths.is_empty() || paths.iter().eq(self.rows.iter().map(|row| &row.source)) {
            return;
        }
        self.rows = paths
            .into_iter()
            .map(|source| Row {
                output: self.format.suggested_output(&source),
                source,
                status: FileStatus::Ready,
            })
            .collect();
        self.plan_outputs();
    }

    fn change_format(&mut self, format: Format) {
        if self.format == format {
            return;
        }
        let previous = self.format;
        self.format = format;
        if let [row] = self.rows.as_mut_slice()
            && row.output != previous.suggested_output(&row.source)
        {
            let output = row.output.with_extension(format.extension());
            row.output = if output == row.source {
                format.suggested_output(&row.source)
            } else {
                output
            };
            row.status = FileStatus::Ready;
            self.error = None;
            return;
        }
        self.plan_outputs();
    }

    fn plan_outputs(&mut self) {
        // Reserve the inputs too: a batch may contain clip.mp4 and clip.wav.
        let key = |path: &Path| path.to_string_lossy().to_lowercase();
        let mut used: HashSet<_> = self.rows.iter().map(|r| key(&r.source)).collect();
        let mut next_suffix = HashMap::new();
        for row in &mut self.rows {
            let mut base = self.format.suggested_output(&row.source);
            if let Some(folder) = &self.folder {
                base = folder.join(base.file_name().unwrap_or_default());
            }
            let mut path = base.clone();
            while !used.insert(key(&path)) {
                let suffix = next_suffix.entry(key(&base)).or_insert(2);
                let mut name = base.file_stem().unwrap_or_default().to_owned();
                name.push(format!(" ({suffix}).{}", self.format.extension()));
                *suffix += 1;
                path = base.with_file_name(name);
            }
            if row.output != path || row.status != FileStatus::Saved {
                row.status = FileStatus::Ready;
            }
            row.output = path;
        }
        self.error = None;
    }
}

impl Desktop {
    pub(super) fn open_converter(&mut self) {
        self.dialog = Some(Dialog::Converter);
    }

    fn pick_conversion_sources(&mut self) -> anyhow::Result<()> {
        let state = &mut self.extras.converter;
        let folder = state
            .rows
            .first()
            .and_then(|r| r.source.parent())
            .unwrap_or(&self.store.paths.downloads);
        if let Some(files) =
            self.folder_picker
                .open_files(tr("Choose files"), folder, (tr("Media file"), "*.*"))?
        {
            state.select_sources(files);
        }
        Ok(())
    }

    fn pick_conversion_output(&mut self) -> anyhow::Result<()> {
        let state = &mut self.extras.converter;
        match state.rows.as_mut_slice() {
            [] => {}
            [row] => {
                let format = state.format;
                let filter = format!("*.{}", format.extension());
                if let Some(path) = self.folder_picker.save_file(
                    tr("Save as…"),
                    row.output.parent().unwrap_or(&self.store.paths.downloads),
                    &row.output.file_name().unwrap_or_default().to_string_lossy(),
                    (format.label(), &filter),
                    format.extension(),
                )? {
                    anyhow::ensure!(
                        format.matches(&path),
                        "Choose a filename ending in .{}",
                        format.extension()
                    );
                    row.output = path;
                    row.status = FileStatus::Ready;
                    state.error = None;
                }
            }
            rows => {
                let initial = state
                    .folder
                    .as_deref()
                    .or_else(|| rows[0].source.parent())
                    .unwrap_or(&self.store.paths.downloads);
                if let Some(path) =
                    self.folder_picker
                        .choose_titled("", initial, tr("Choose a folder"))?
                {
                    state.folder = Some(path.into());
                    state.plan_outputs();
                }
            }
        }
        Ok(())
    }

    fn start_conversion(&mut self) -> anyhow::Result<()> {
        let state = &mut self.extras.converter;
        if state.running.is_some() {
            return Ok(());
        }
        for row in &mut state.rows {
            if row.status != FileStatus::Saved {
                row.status = FileStatus::Ready;
            }
        }
        state.error = None;
        self.start_next_conversion(0)
    }

    fn start_next_conversion(&mut self, from: usize) -> anyhow::Result<()> {
        let state = &mut self.extras.converter;
        if let Some((index, row)) = state
            .rows
            .iter()
            .enumerate()
            .skip(from)
            .find(|(_, r)| r.status == FileStatus::Ready)
        {
            let (source, output) = (row.source.clone(), row.output.clone());
            let bitrate = state.bitrate;
            let cancel = CancellationToken::new();
            let worker_cancel = cancel.clone();
            let task = background_io::Task::spawn(self.background_io.context.clone(), move || {
                convert_with_bitrate(&source, &output, bitrate, &worker_cancel)
            })?;
            state.running = Some(Conversion {
                index,
                cancel,
                task,
            });
        }
        Ok(())
    }

    pub(super) fn poll_conversion(&mut self, ctx: &egui::Context) {
        let state = &mut self.extras.converter;
        let Some(run) = state.running.as_mut() else {
            return;
        };
        let Some(result) = run.task.poll() else {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        };
        let index = run.index;
        let stopped = run.cancel.is_cancelled();
        state.running = None;
        state.rows[index].status = match result {
            Ok(()) => FileStatus::Saved,
            Err(_) if stopped => FileStatus::Cancelled,
            Err(e) => FileStatus::Failed(format!("{e:#}")),
        };
        if stopped {
            for row in &mut state.rows[index + 1..] {
                if row.status == FileStatus::Ready {
                    row.status = FileStatus::Cancelled;
                }
            }
        } else if let Err(e) = self.start_next_conversion(index + 1) {
            self.extras.converter.error = Some(format!("{e:#}"));
            return;
        }
        let state = &mut self.extras.converter;
        if state.running.is_some() {
            return;
        }
        if let [row] = state.rows.as_slice() {
            self.notice = Some(match &row.status {
                FileStatus::Saved => {
                    Notice::success(tf!("Saved to {path}", path = row.output.display()))
                }
                FileStatus::Cancelled => Notice::info(tr("Cancelled").into()),
                _ => return,
            });
            *state = Converter::default();
            if self.dialog == Some(Dialog::Converter) {
                self.dialog = None;
            }
        }
    }

    pub(super) fn converter_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let state = &mut self.extras.converter;
        let running = state.running.is_some();
        let response = form_dialog::modal("converter", p).show(ctx, |ui| {
            let close = form_dialog::header(
                ui,
                tr("Converter"),
                tr("Convert the first audio track from local media files."),
                form_dialog::Icon::Symbol(GLYPH),
                p,
                true,
            );
            let count = state.rows.len();
            let source_summary = if count > 1 {
                tf!("{count} files selected", count = count)
            } else {
                tr("No file selected").into()
            };
            let choose = file_row(
                ui,
                p,
                tr("Files"),
                if count == 1 {
                    Some(&state.rows[0].source)
                } else {
                    None
                },
                &source_summary,
                tr("Choose files"),
                !running,
            );
            ui.add_space(4.0);
            let mut format = state.format;
            ui.horizontal_wrapped(|ui| {
                let label = ui.label(tr("Output format"));
                ui.add_enabled_ui(!running, |ui| {
                    egui::ComboBox::from_id_salt("conversion-output-format")
                        .width(85.0)
                        .selected_text(format.label())
                        .show_ui(ui, |ui| {
                            for choice in Format::ALL {
                                ui.selectable_value(&mut format, choice, choice.label());
                            }
                        })
                        .response
                        .labelled_by(label.id);
                });
                if format == Format::Mp3 {
                    let label = ui.label(tr("Bitrate"));
                    let previous = state.bitrate;
                    ui.add_enabled_ui(!running, |ui| {
                        egui::ComboBox::from_id_salt("conversion-mp3-bitrate")
                            .width(92.0)
                            .selected_text(format!("{} kbps", state.bitrate.kbps()))
                            .show_ui(ui, |ui| {
                                for rate in Mp3Bitrate::ALL {
                                    ui.selectable_value(
                                        &mut state.bitrate,
                                        rate,
                                        format!("{} kbps", rate.kbps()),
                                    );
                                }
                            })
                            .response
                            .labelled_by(label.id);
                    });
                    if state.bitrate != previous {
                        for row in &mut state.rows {
                            row.status = FileStatus::Ready;
                        }
                        state.error = None;
                    }
                } else {
                    ui.weak("PCM · 16-bit · 44.1 kHz");
                }
            });
            state.change_format(format);
            ui.add_space(4.0);
            let output = if count == 1 {
                Some(&state.rows[0].output)
            } else {
                state.folder.as_ref()
            };
            let destination = file_row(
                ui,
                p,
                tr("Save to"),
                output.map(|p| p.as_path()),
                tr("Same folder as source"),
                if count > 1 {
                    tr("Choose a folder")
                } else {
                    tr("Save as…")
                },
                !running && count > 0,
            );
            if count > 1 {
                if state.folder.is_some()
                    && ui
                        .add_enabled(!running, egui::Button::new(tr("Same folder as source")))
                        .clicked()
                {
                    state.folder = None;
                    state.plan_outputs();
                }
                egui::ScrollArea::vertical()
                    .id_salt("conversion-files")
                    .max_height(130.0)
                    .show(ui, |ui| {
                        for (i, row) in state.rows.iter().enumerate() {
                            let status = if state.running.as_ref().is_some_and(|r| r.index == i) {
                                tr("Converting…")
                            } else {
                                match &row.status {
                                    FileStatus::Ready => tr("Waiting"),
                                    FileStatus::Saved => tr("Completed"),
                                    FileStatus::Failed(_) => tr("Failed"),
                                    FileStatus::Cancelled => tr("Cancelled"),
                                }
                            };
                            let line = format!(
                                "{} → {} · {status}",
                                row.source.file_name().unwrap_or_default().to_string_lossy(),
                                row.output.file_name().unwrap_or_default().to_string_lossy()
                            );
                            ui.add(egui::Label::new(line).truncate())
                                .on_hover_text(format!(
                                    "{}\n{}",
                                    row.source.display(),
                                    row.output.display()
                                ));
                            if let FileStatus::Failed(error) = &row.status {
                                ui.colored_label(p.warning, error);
                            }
                        }
                    });
            } else if let Some(Row {
                status: FileStatus::Failed(error),
                ..
            }) = state.rows.first()
            {
                ui.colored_label(p.warning, error);
            }
            if let Some(error) = &state.error {
                ui.colored_label(p.warning, error);
            }
            let all_saved = count > 0 && state.rows.iter().all(|r| r.status == FileStatus::Saved);
            let start = form_dialog::controls(ui, |ui| {
                if let Some(run) = &state.running {
                    if form_dialog::secondary_enabled(ui, !run.cancel.is_cancelled(), tr("Cancel"))
                        .clicked()
                    {
                        run.cancel.cancel();
                    }
                    ui.label(if run.cancel.is_cancelled() {
                        tr("Cancelling…")
                    } else {
                        tr("Converting…")
                    });
                    ui.spinner();
                    false
                } else {
                    form_dialog::primary_enabled(
                        ui,
                        count > 0,
                        if all_saved {
                            tr("Close")
                        } else {
                            tr("Convert")
                        },
                    )
                    .clicked()
                }
            });
            (
                close || (start && all_saved),
                choose,
                destination,
                start && !all_saved,
            )
        });
        let (close, choose, dest, start) = response.inner;
        if close || form_dialog::dismissed(&response, !state.rows.is_empty()) {
            if let Some(run) = &state.running {
                run.cancel.cancel();
            } else {
                *state = Converter::default();
                self.dialog = None;
            }
            return;
        }
        let result = if choose {
            self.pick_conversion_sources()
        } else if dest {
            self.pick_conversion_output()
        } else if start {
            self.start_conversion()
        } else {
            Ok(())
        };
        if let Err(e) = result {
            self.extras.converter.error = Some(format!("{e:#}"));
        }
    }
}

fn file_row(
    ui: &mut egui::Ui,
    p: Palette,
    label: &str,
    path: Option<&Path>,
    empty: &str,
    button: &str,
    enabled: bool,
) -> bool {
    egui::Frame::new()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let button_width = view::button_width(ui, tr("Choose files"))
                    .max(view::button_width(ui, tr("Save as…")))
                    .max(view::button_width(ui, tr("Choose a folder")))
                    .ceil()
                    .max(86.0);
                let width =
                    (ui.available_width() - button_width - ui.spacing().item_spacing.x).max(1.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 36.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(RichText::new(label).small().color(p.muted));
                        if let Some(path) = path {
                            ui.add(
                                egui::Label::new(
                                    path.file_name()
                                        .unwrap_or(path.as_os_str())
                                        .to_string_lossy(),
                                )
                                .truncate(),
                            )
                            .on_hover_text(path.display().to_string());
                            if let Some(folder) = path.parent() {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(folder.to_string_lossy())
                                            .small()
                                            .color(p.muted),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(folder.display().to_string());
                            }
                        } else {
                            ui.label(RichText::new(empty).color(p.muted));
                        }
                    },
                );
                ui.add_enabled_ui(enabled, |ui| {
                    ui.add_sized(
                        [button_width, view::CONTROL_HEIGHT],
                        egui::Button::new(button),
                    )
                })
                .inner
                .clicked()
            })
            .inner
        })
        .inner
}

#[cfg(test)]
mod tests;
