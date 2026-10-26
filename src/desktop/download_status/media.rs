//! Media segments use playlist order, not byte positions in the merged output.
use super::*;
use download_engine::{hls::Download, transfer_activity::SegmentState};

impl Window {
    pub(super) fn media_part(
        &self,
        job: &Job,
        download: &Download,
        index: usize,
    ) -> (u64, Option<u64>, &'static str) {
        if let Some(part) = download.completed.get(index) {
            return (part.size, Some(part.size), tr("Completed"));
        }
        if job.status == Status::Downloading
            && let Some(live) = self.live_media.filter(|part| part.index == index)
        {
            return (
                live.downloaded,
                live.total,
                super::segment_label(live.state),
            );
        }
        let state = if index == download.completed.len() {
            match job.status {
                Status::Paused => tr("Paused (restarts segment)"),
                Status::Failed => tr("Interrupted"),
                _ => tr("Waiting"),
            }
        } else {
            tr("Waiting")
        };
        let received = if index == download.completed.len()
            && matches!(job.status, Status::Paused | Status::Failed)
        {
            let saved: u64 = download.completed.iter().map(|part| part.size).sum();
            job.downloaded.saturating_sub(saved)
        } else {
            0
        };
        (received, None, state)
    }

    pub(super) fn draw_media_details(
        &self,
        ui: &mut egui::Ui,
        job: &Job,
        download: &Download,
        p: Palette,
    ) {
        let count = download.playlist.part_count();
        let completed = download.completed.len();
        ui.label(
            RichText::new(tf!(
                "Video segments ({completed} / {count} complete)",
                completed = completed,
                count = count
            ))
            .size(11.0)
            .color(p.muted),
        );
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3, p.track);
        let inner = rect.shrink(1.0);
        if count > 0 {
            let unit = inner.width() / count as f32;
            let end = inner.left() + unit * completed as f32;
            if completed > 0 {
                painter.rect_filled(
                    Rect::from_min_max(inner.min, pos2(end, inner.bottom())),
                    0,
                    p.blue,
                );
            }
            if let Some(live) = self.live_media.filter(|part| {
                job.status == Status::Downloading && part.index >= completed && part.index < count
            }) {
                let left = inner.left() + unit * live.index as f32;
                // Unknown byte lengths get an outlined active segment, never a
                // made-up percentage. The overall bar counts validated parts.
                if let Some(total) = live.total.filter(|total| *total > 0) {
                    let fraction = (live.downloaded as f64 / total as f64).clamp(0.0, 1.0) as f32;
                    painter.rect_filled(
                        Rect::from_min_max(
                            pos2(left, inner.top()),
                            pos2(left + unit * fraction, inner.bottom()),
                        ),
                        0,
                        p.blue,
                    );
                }
                painter.rect_stroke(
                    Rect::from_min_max(
                        pos2(left, inner.top()),
                        pos2(left + unit.max(2.0), inner.bottom()),
                    ),
                    0,
                    Stroke::new(1.0, p.orange),
                    StrokeKind::Inside,
                );
            }
            // Keep long playlists readable and bounded at pixel resolution.
            let stride = (count as f32 / (inner.width() / 5.0).max(1.0)).ceil() as usize;
            for index in (stride..count).step_by(stride.max(1)) {
                let x = inner.left() + unit * index as f32;
                painter.line_segment(
                    [pos2(x, inner.top()), pos2(x, inner.bottom())],
                    Stroke::new(1.0, p.border),
                );
            }
            if let Some(pos) = response.hover_pos() {
                let index = (((pos.x - inner.left()) / unit).max(0.0) as usize).min(count - 1);
                let (received, _, state) = self.media_part(job, download, index);
                response.on_hover_text(tf!(
                    "Segment {} · {} · {state}",
                    index + 1,
                    bytes(received),
                    state = state
                ));
            }
        }
        painter.rect_stroke(rect, 3, Stroke::new(1.0, p.border), StrokeKind::Inside);
        if let Some(live) = self.live_media.filter(|part| {
            job.status == Status::Downloading
                && part.index < count
                && matches!(
                    part.state,
                    SegmentState::SendingGet | SegmentState::ReceivingData
                )
        }) {
            ui.label(
                RichText::new(format!(
                    "1 connection · Segment {} / {} · {}",
                    live.index + 1,
                    count,
                    bytes(live.downloaded)
                ))
                .size(11.0)
                .color(p.text),
            );
        }
        ui.label(RichText::new(tr("Video segments download one at a time.")).size(11.0).color(p.muted))
            .on_hover_text(tr("The per-file connection setting applies to direct HTTP files. YouTube, HLS and DASH currently fetch video and audio segments sequentially. The map follows playlist order; segment widths do not represent bytes."));

        egui::Frame::new()
            .fill(p.background)
            .stroke((1.0, p.border))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().interact_size.y = 17.0;
                ui.spacing_mut().item_spacing.y = 1.0;
                media_row(ui, "#", tr("Downloaded"), tr("Status"), p.muted);
                let row_height = 17.0;
                egui::ScrollArea::vertical()
                    .id_salt("media-parts")
                    // Four complete rows fit the compact native window.
                    .max_height(row_height * 4.0 + ui.spacing().item_spacing.y * 3.0)
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .auto_shrink([false, true])
                    .show_rows(ui, row_height, count, |ui, range| {
                        for index in range {
                            let (received, _, state) = self.media_part(job, download, index);
                            let video_count = download.playlist.parts.len();
                            let (track, part) = if index < video_count {
                                (tr("Video"), &download.playlist.parts[index])
                            } else {
                                (
                                    tr("Audio"),
                                    &download
                                        .playlist
                                        .audio
                                        .as_ref()
                                        .expect("Audio parts counted")
                                        .parts[index - video_count],
                                )
                            };
                            let info = format!(
                                "{track}{} · {state}",
                                if part.initialization { " init" } else { "" }
                            );
                            media_row(
                                ui,
                                &(index + 1).to_string(),
                                &bytes(received),
                                &info,
                                p.text,
                            );
                        }
                    });
            });
    }
}

fn media_row(ui: &mut egui::Ui, number: &str, downloaded: &str, info: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (text, width) in [
            (number, 30.0),
            (downloaded, 92.0),
            (info, (ui.available_width() - 138.0).max(80.0)),
        ] {
            ui.allocate_ui_with_layout(
                vec2(width, 17.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_min_width(width);
                    ui.set_max_width(width);
                    ui.add(
                        egui::Label::new(RichText::new(text).size(11.0).color(color)).truncate(),
                    );
                },
            );
        }
    });
}
