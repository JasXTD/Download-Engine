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
        if let Some(part) = download.saved_part(index) {
            return (part.size, Some(part.size), tr("Completed"));
        }
        if job.status == Status::Downloading
            && let Some(live) = self
                .live_media_parts
                .iter()
                .find(|part| part.index == index)
                .copied()
                .or_else(|| self.live_media.filter(|part| part.index == index))
        {
            return (
                live.downloaded,
                live.total,
                super::segment_label(live.state),
            );
        }
        let state = match job.status {
            Status::Paused => tr("Paused (restarts segment)"),
            Status::Failed => tr("Interrupted"),
            _ => tr("Waiting"),
        };
        // A paused job's aggregate includes several unfinished workers. Their
        // bytes cannot be attributed to one segment and are fetched again.
        (0, None, state)
    }

    pub(super) fn draw_media_details(
        &self,
        ui: &mut egui::Ui,
        job: &Job,
        download: &Download,
        p: Palette,
    ) {
        let count = download.playlist.part_count();
        let completed = download.completed_count();
        let live_parts = if self.live_media_parts.is_empty() {
            self.live_media.into_iter().collect::<Vec<_>>()
        } else {
            self.live_media_parts.clone()
        };
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
            let end = inner.left() + unit * download.completed.len() as f32;
            if !download.completed.is_empty() {
                painter.rect_filled(
                    Rect::from_min_max(inner.min, pos2(end, inner.bottom())),
                    0,
                    p.blue,
                );
            }
            for index in download.completed_ahead.keys() {
                let left = inner.left() + unit * *index as f32;
                painter.rect_filled(
                    Rect::from_min_max(pos2(left, inner.top()), pos2(left + unit, inner.bottom())),
                    0,
                    p.blue,
                );
            }
            for live in live_parts.iter().filter(|part| {
                job.status == Status::Downloading
                    && download.saved_part(part.index).is_none()
                    && part.index < count
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
        let active = live_parts
            .iter()
            .filter(|part| {
                job.status == Status::Downloading
                    && part.index < count
                    && download.saved_part(part.index).is_none()
                    && matches!(
                        part.state,
                        SegmentState::SendingGet | SegmentState::ReceivingData
                    )
            })
            .count();
        if active > 0 {
            ui.label(
                RichText::new(if active == 1 {
                    tr("1 active connection").to_owned()
                } else {
                    tf!("{} active connections", active)
                })
                .size(11.0)
                .color(p.text),
            );
        }
        ui.label(RichText::new(tr("Video segments download in parallel (up to 4).")).size(11.0).color(p.muted))
            .on_hover_text(tr("Verified parts are kept on pause. Segment widths represent playlist order, not byte size."));

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
