//! Shared modal presentation. Callers own validation, cancellation and submission.
use super::*;
use egui::{Align, Align2, FontId, Layout, Rect, Sense, Stroke, StrokeKind, vec2};

pub(super) const VIOLET: Color32 = Color32::from_rgb(56, 178, 134);

pub(super) enum Icon {
    Download,
    Batch,
    Queue,
    Symbol(&'static str),
}

pub(super) fn modal(id: &'static str, p: Palette) -> egui::Modal {
    egui::Modal::new(egui::Id::new(id)).frame(frame(p))
}

/// The same card can also be hosted in a non-modal native status window.
pub(super) fn frame(p: Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.background)
        .stroke(Stroke::new(1.0, p.border))
        .inner_margin(egui::Margin::symmetric(16, 14))
        .corner_radius(10)
}

pub(super) fn header(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: &str,
    icon: Icon,
    p: Palette,
    close_enabled: bool,
) -> bool {
    ui.set_width((ui.ctx().content_rect().width() - 80.0).clamp(280.0, 448.0));
    heading(ui, title, subtitle, icon, p, close_enabled)
}

/// A header for forms with their own content width (lists, grids and compact cards).
pub(super) fn title(ui: &mut egui::Ui, title: &str, icon: Icon, p: Palette, enabled: bool) -> bool {
    heading(ui, title, "", icon, p, enabled)
}

fn heading(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: &str,
    icon: Icon,
    p: Palette,
    close_enabled: bool,
) -> bool {
    ui.style_mut()
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::proportional(20.0));
    ui.spacing_mut().interact_size.y = view::CONTROL_HEIGHT;
    ui.spacing_mut().item_spacing.y = 4.0;
    let cancel = ui
        .horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            mark(ui, rect, icon);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                let cancel = view::dialog_header_enabled(ui, title, close_enabled);
                if !subtitle.is_empty() {
                    ui.label(RichText::new(subtitle).size(12.0).color(p.muted));
                }
                cancel
            })
            .inner
        })
        .inner;
    ui.add_space(6.0);
    cancel
}

/// Returns whether the primary action was clicked. The header × and Escape dismiss.
pub(super) fn footer(ui: &mut egui::Ui, action: &str, hint: Option<&str>, p: Palette) -> bool {
    controls(ui, |ui| {
        let submit = primary_button(ui, action).clicked();
        if let Some(hint) = hint {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(hint).size(11.0).color(p.muted));
            });
        }
        submit
    })
}

/// Right-aligned footer; put the primary action first, then secondary actions.
/// Do not add Cancel or Close: the header × already dismisses the dialog.
pub(super) fn controls<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(2.0);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), view::PRIMARY_HEIGHT),
        Layout::right_to_left(Align::Center),
        content,
    )
    .inner
}

/// A footer action beside the primary one; it shares the primary height.
pub(super) fn secondary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(13.0))
            .min_size(vec2(74.0, view::PRIMARY_HEIGHT))
            .corner_radius(7),
    )
}

pub(super) fn primary_enabled(ui: &mut egui::Ui, enabled: bool, label: &str) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| primary_button(ui, label))
        .inner
}

pub(super) fn secondary_enabled(ui: &mut egui::Ui, enabled: bool, label: &str) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| secondary_button(ui, label))
        .inner
}

pub(super) fn danger_button(ui: &mut egui::Ui, label: &str, p: Palette) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(13.0).color(p.warning))
            .fill(p.warning.gamma_multiply(0.12))
            .stroke(Stroke::new(1.0, p.warning))
            .min_size(vec2(90.0, view::PRIMARY_HEIGHT))
            .corner_radius(7),
    )
}

/// Keep actions outside the scrolling form. Use the viewport, not the modal's
/// previous frame size, so long forms cannot progressively collapse their body.
pub(super) fn body(ui: &mut egui::Ui, id: &'static str, content: impl FnOnce(&mut egui::Ui)) {
    let height = (ui.ctx().content_rect().height() - 220.0).max(150.0);
    egui::ScrollArea::vertical()
        .id_salt(id)
        .min_scrolled_height(height)
        .max_height(height)
        .show(ui, content);
}

fn mark(ui: &egui::Ui, rect: Rect, icon: Icon) {
    let (fill, ink) = if theme::custom(ui.ctx()).is_some() {
        let p = Palette::for_context(ui.ctx());
        (p.selected, p.accent)
    } else if ui.visuals().dark_mode {
        (
            Color32::from_rgb(28, 60, 50),
            Color32::from_rgb(122, 226, 184),
        )
    } else {
        (
            Color32::from_rgb(222, 247, 236),
            Color32::from_rgb(20, 118, 84),
        )
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 9, fill);
    painter.rect_stroke(
        rect,
        9,
        Stroke::new(
            1.0,
            if theme::custom(ui.ctx()).is_some() {
                Palette::for_context(ui.ctx()).accent.gamma_multiply(0.55)
            } else {
                VIOLET.gamma_multiply(0.55)
            },
        ),
        StrokeKind::Inside,
    );
    let point = |x, y| rect.min + vec2(x, y);
    let stroke = Stroke::new(2.0, ink);
    match icon {
        Icon::Symbol(glyph) => view::icon(ui, rect.center(), glyph, 22.0, ink),
        Icon::Download => {
            painter.line_segment([point(20.0, 10.0), point(20.0, 24.0)], stroke);
            painter.add(egui::Shape::line(
                vec![point(14.0, 19.0), point(20.0, 25.0), point(26.0, 19.0)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![
                    point(11.0, 26.0),
                    point(11.0, 30.0),
                    point(29.0, 30.0),
                    point(29.0, 26.0),
                ],
                stroke,
            ));
        }
        Icon::Batch => {
            painter.add(egui::Shape::line(
                vec![
                    point(13.0, 25.0),
                    point(10.0, 25.0),
                    point(10.0, 10.0),
                    point(24.0, 10.0),
                    point(24.0, 13.0),
                ],
                stroke,
            ));
            painter.rect_stroke(
                Rect::from_min_max(point(16.0, 16.0), point(30.0, 30.0)),
                2,
                stroke,
                StrokeKind::Inside,
            );
            for y in [21.0, 25.0] {
                painter.line_segment([point(20.0, y), point(26.0, y)], Stroke::new(1.5, ink));
            }
        }
        Icon::Queue => {
            for (y, width) in [(13.0, 17.0), (19.0, 12.0), (25.0, 7.0)] {
                painter.line_segment(
                    [point(10.0, y), point(10.0 + width, y)],
                    Stroke::new(3.0, ink),
                );
            }
            let center = point(27.0, 26.0);
            painter.circle_filled(center, 6.5, ink);
            for axis in [vec2(3.0, 0.0), vec2(0.0, 3.0)] {
                painter.line_segment([center - axis, center + axis], Stroke::new(1.5, fill));
            }
        }
    }
}

pub(super) fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let custom = theme::custom(ui.ctx()).is_some();
    let p = Palette::for_context(ui.ctx());
    let ink = if custom {
        p.accent_text
    } else {
        Color32::WHITE
    };
    let font = FontId::proportional(13.0);
    let text = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let width = (text.size().x + 28.0).max(130.0);
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, view::PRIMARY_HEIGHT), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), label)
    });
    // White label text keeps at least 4.5:1 contrast against every gradient stop.
    let (top, bottom) = if custom {
        let factor = if response.is_pointer_button_down_on() {
            0.84
        } else if response.hovered() {
            1.0
        } else {
            0.95
        };
        (
            p.accent.gamma_multiply(factor),
            p.accent.gamma_multiply(factor - 0.04),
        )
    } else if response.is_pointer_button_down_on() {
        (
            Color32::from_rgb(20, 110, 80),
            Color32::from_rgb(16, 92, 66),
        )
    } else if response.hovered() {
        (
            Color32::from_rgb(26, 134, 96),
            Color32::from_rgb(22, 114, 82),
        )
    } else {
        (
            Color32::from_rgb(24, 128, 92),
            Color32::from_rgb(20, 108, 78),
        )
    };
    view::gradient_panel(ui.painter(), rect, 7.0, top, bottom);
    ui.painter().rect_stroke(
        rect,
        7,
        Stroke::new(
            if response.has_focus() { 2.0 } else { 1.0 },
            if custom {
                p.accent
            } else {
                Color32::from_rgb(118, 224, 185)
            },
        ),
        StrokeKind::Inside,
    );
    let text_rect = Align2::CENTER_CENTER.anchor_size(rect.center(), text.size());
    ui.painter().galley(text_rect.min, text, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
