//! Queue creation presentation; validation and persistence stay in queues.rs.
use super::*;
use egui::{Align, FontId, Layout, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use super::form_dialog::VIOLET;
const SYNC_HELP: &str = "Check saved URLs now or on a schedule. Changed files are saved as new copies; previous copies are kept.";

impl Desktop {
    pub(super) fn new_queue_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let mut submit = false;
        let mut cancel = false;
        let response = form_dialog::modal("new-queue-dialog", p).show(ctx, |ui| {
            cancel = form_dialog::header(
                ui,
                tr("Create new queue"),
                tr("New queues wait until you start or schedule them."),
                form_dialog::Icon::Queue,
                p,
                true,
            );
            egui::Frame::new()
                .fill(p.background)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 5.0;
                    let name_label = ui
                        .horizontal(|ui| {
                            let label = ui.label(RichText::new(tr("Queue name")).strong());
                            ui.label(RichText::new("*").color(p.warning))
                                .on_hover_text(tr("Required"));
                            label
                        })
                        .inner;
                    let name = egui::Frame::new()
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, p.border))
                        .corner_radius(7)
                        .inner_margin(egui::Margin::symmetric(10, 1))
                        .show(ui, |ui| {
                            // 1px margin and 1px stroke above and below make the field
                            // the shared control height.
                            let inner = view::CONTROL_HEIGHT - 4.0;
                            ui.spacing_mut().interact_size.y = inner;
                            ui.horizontal(|ui| {
                                let (rect, _) =
                                    ui.allocate_exact_size(vec2(18.0, inner), Sense::hover());
                                tag_mark(ui, rect.center(), p.muted);
                                ui.add(
                                    view::line_edit(&mut self.queue_name)
                                        .id(egui::Id::new("new-queue-name"))
                                        .hint_text(tr("e.g. Videos, Documents, Backups"))
                                        .desired_width(f32::INFINITY)
                                        .min_size(vec2(0.0, inner))
                                        .vertical_align(Align::Center)
                                        .font(FontId::proportional(13.5))
                                        .frame(egui::Frame::NONE),
                                )
                            })
                            .inner
                        });
                    let input = name.inner.labelled_by(name_label.id);
                    if input.has_focus() {
                        ui.painter().rect_stroke(
                            name.response.rect,
                            7,
                            Stroke::new(1.5, VIOLET),
                            StrokeKind::Inside,
                        );
                    }
                    ui.separator();
                    synchronization(ui, p, &mut self.queue_synchronize);
                });
            if let Some(error) = &self.form_error {
                ui.add_space(4.0);
                ui.label(RichText::new(error).size(12.0).color(p.warning));
            }
            submit = form_dialog::footer(ui, tr("Create queue"), None, p);
        });
        if cancel || response.should_close() {
            self.dialog = None;
        } else if submit {
            self.submit_queue_dialog(Dialog::NewQueue);
        }
    }
}

fn tag_mark(ui: &egui::Ui, center: egui::Pos2, color: Color32) {
    let points = [
        (-7.0, 0.0),
        (0.0, -7.0),
        (6.0, -1.0),
        (6.0, 3.0),
        (1.5, 7.0),
        (-7.0, 0.0),
    ]
    .into_iter()
    .map(|(x, y)| center + vec2(x, y))
    .collect();
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(1.4, color)));
    ui.painter()
        .circle_filled(center + vec2(1.0, -2.0), 1.3, color);
}

fn synchronization(ui: &mut egui::Ui, p: Palette, value: &mut bool) {
    let (rect, mut response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
        response.request_focus();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            true,
            *value,
            tr("Synchronization queue"),
        )
    });
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect.expand(4.0),
            6,
            if response.hovered() {
                p.selected.gamma_multiply(0.55)
            } else {
                Color32::TRANSPARENT
            },
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(4.0),
            6,
            Stroke::new(1.0, VIOLET),
            StrokeKind::Inside,
        );
    }
    let mut labels = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                rect.min + vec2(0.0, 2.0),
                pos2(rect.right() - 48.0, rect.max.y),
            ))
            .layout(Layout::top_down(Align::Min)),
    );
    labels.spacing_mut().item_spacing.y = 2.0;
    labels.add(
        egui::Label::new(
            RichText::new(tr("Synchronization queue"))
                .size(13.0)
                .strong(),
        )
        .selectable(false)
        .sense(Sense::hover()),
    );
    labels.add(
        egui::Label::new(
            RichText::new(tr(
                "Check saved URLs for updated files when this queue runs.",
            ))
            .size(11.5)
            .color(p.muted),
        )
        .selectable(false)
        .sense(Sense::hover()),
    );
    let track =
        Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(36.0, 20.0));
    ui.painter().rect_filled(
        track,
        10,
        if *value {
            VIOLET
        } else if ui.visuals().dark_mode {
            p.track
        } else {
            p.muted
        },
    );
    let x = if *value {
        track.right() - 10.0
    } else {
        track.left() + 10.0
    };
    ui.painter()
        .circle_filled(pos2(x, track.center().y), 7.5, Color32::WHITE);
    response
        .on_hover_text(tr(SYNC_HELP))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
}
