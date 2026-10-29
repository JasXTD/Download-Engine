use super::*;
use download_engine::table_layout::{Column, TableLayout};
use egui::{CursorIcon, FontId, Rect, Sense, Stroke, pos2, vec2};

#[derive(Default)]
pub(super) struct State {
    resize: Option<Resize>,
    list: Option<ListContext>,
}

#[derive(PartialEq, Eq)]
struct ListContext {
    queue: String,
    status: Filter,
    category: Option<Category>,
    search: String,
}
struct Resize {
    column: Column,
    start: f32,
    origin: f32,
    draft: TableLayout,
}
#[derive(Clone, Copy)]
struct ColumnDrag(Column);

/// Responsive visibility uses the viewport, never the scrolled content width.
/// Customized widths may overflow horizontally; names and action buttons stay usable.
pub(super) fn widths(
    available: f32,
    visible: [bool; 7],
    layout: TableLayout,
) -> Vec<(Column, f32)> {
    let mut columns: Vec<_> = layout
        .order
        .into_iter()
        .filter(|column| {
            column.visibility_index().is_none_or(|i| visible[i])
                && match column {
                    Column::Number | Column::Added => available >= 1030.0,
                    Column::Eta => available >= 830.0,
                    _ => true,
                }
        })
        .map(|column| {
            // Give filenames more room in compact windows. Explicit user widths
            // still win, and overflowing custom layouts keep horizontal scrolling.
            let automatic = if available < 830.0 {
                match column {
                    Column::Status => 140,
                    Column::Size => 90,
                    _ => column.default_width(),
                }
            } else {
                column.default_width()
            };
            (
                column,
                f32::from(layout.widths[column as usize].unwrap_or(automatic)),
            )
        })
        .collect();
    if layout.widths[Column::Name as usize].is_none() {
        let used: f32 = columns.iter().map(|(_, w)| *w).sum();
        if let Some((_, width)) = columns.iter_mut().find(|(c, _)| *c == Column::Name) {
            *width += (available - used).max(0.0);
        }
    }
    columns
}
pub(super) fn cells(rect: Rect, widths: &[(Column, f32)]) -> Vec<(Column, Rect)> {
    let mut x = rect.left();
    widths
        .iter()
        .map(|&(column, width)| {
            let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(width, rect.height()));
            x += width;
            (column, cell)
        })
        .collect()
}
impl State {
    /// Reset results on navigation, while ordinary progress refreshes keep their position.
    pub(super) fn prepare_list(
        &mut self,
        queue: &str,
        status: Filter,
        category: Option<Category>,
        search: &str,
    ) -> bool {
        let next = ListContext {
            queue: queue.into(),
            status,
            category,
            search: search.trim().to_lowercase(),
        };
        if self.list.as_ref() == Some(&next) {
            false
        } else {
            self.list = Some(next);
            true
        }
    }

    pub(super) fn preview(&self, saved: TableLayout) -> TableLayout {
        self.resize.as_ref().map_or(saved, |r| r.draft)
    }
    pub(super) fn cancel(&mut self) {
        self.resize = None;
    }
    pub(super) fn header(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        widths: &[(Column, f32)],
        saved: TableLayout,
        p: Palette,
    ) -> Option<TableLayout> {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) || !ui.is_enabled() {
            self.resize = None;
            if egui::DragAndDrop::has_payload_of_type::<ColumnDrag>(ui.ctx()) {
                egui::DragAndDrop::clear_payload(ui.ctx());
            }
            return None;
        }
        let mut edited = None;
        for (column, raw) in cells(rect, widths) {
            let cell = raw.shrink2(vec2(8.0, 7.0));
            let body = Rect::from_min_max(raw.min + vec2(4.0, 0.0), raw.max - vec2(4.0, 0.0));
            let response = ui.interact(
                body,
                ui.id().with(("column-heading", column as u8)),
                Sense::drag(),
            );
            response.dnd_set_drag_payload(ColumnDrag(column));
            if response.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                ui.painter().rect_filled(raw, 0.0, p.selected);
            }
            // Paint labels directly so they cannot steal the header's drag gesture.
            let painter = ui.painter().with_clip_rect(cell.intersect(ui.clip_rect()));
            let mut label = egui::text::LayoutJob::simple_singleline(
                tr(column.label()).into(),
                FontId::proportional(12.0),
                p.muted,
            );
            label.wrap.max_width = cell.width();
            label.wrap.max_rows = 1;
            label.wrap.break_anywhere = true;
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(label));
            painter.galley(cell.left_top(), galley, p.muted);
            if let Some(payload) = response.dnd_hover_payload::<ColumnDrag>()
                && payload.0 != column
            {
                let before = ui.input(|i| {
                    i.pointer
                        .interact_pos()
                        .is_some_and(|pos| pos.x < raw.center().x)
                });
                let x = if before { raw.left() } else { raw.right() };
                ui.painter()
                    .vline(x, raw.y_range(), Stroke::new(2.0, p.accent));
                if response.dnd_release_payload::<ColumnDrag>().is_some() {
                    let mut layout = saved;
                    layout.move_column(payload.0, column, before);
                    edited = Some(layout);
                }
            }
            response.on_hover_text(format!(
                "{}\n{}",
                tr(column.label()),
                tr("Drag to reorder columns. Drag the right edge to resize.")
            ));
            let edge = Rect::from_center_size(
                pos2(raw.right() - 2.0, raw.center().y),
                vec2(8.0, raw.height()),
            );
            let resize = ui
                .interact(
                    edge,
                    ui.id().with(("column-resize", column as u8)),
                    Sense::click_and_drag(),
                )
                .on_hover_cursor(CursorIcon::ResizeHorizontal)
                .on_hover_text(tr(
                    "Drag to resize. Double-click to restore automatic width.",
                ));
            ui.painter().vline(
                raw.right() - 1.0,
                raw.y_range().shrink(7.0),
                Stroke::new(
                    1.0,
                    if resize.hovered() || resize.dragged() {
                        p.accent
                    } else {
                        p.border
                    },
                ),
            );
            if resize.drag_started() {
                self.resize = Some(Resize {
                    column,
                    start: raw.width(),
                    origin: ui
                        .input(|i| i.pointer.press_origin())
                        .unwrap_or(raw.right_center())
                        .x,
                    draft: saved,
                });
            }
            if let Some(draft) = &mut self.resize
                && draft.column == column
                && (resize.dragged() || resize.drag_stopped())
                && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
            {
                draft.draft.widths[column as usize] =
                    Some((draft.start + pos.x - draft.origin).round().clamp(
                        f32::from(column.minimum()),
                        f32::from(TableLayout::MAX_WIDTH),
                    ) as u16);
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
            if resize.drag_stopped() && self.resize.as_ref().is_some_and(|r| r.column == column) {
                edited = self.resize.take().map(|r| r.draft);
            }
            if resize.double_clicked() {
                self.resize = None;
                let mut layout = saved;
                layout.widths[column as usize] = None;
                edited = Some(layout);
            }
        }
        // Lost capture or a release while the table is hidden cannot leave a stale draft.
        if !ui.input(|i| i.pointer.primary_down()) {
            self.resize = None;
        }
        edited.filter(|value| *value != saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn customized_columns_keep_identity_and_never_collapse_actions_or_names() {
        let mut layout = TableLayout::default();
        layout.move_column(Column::Size, Column::Name, true);
        layout.widths[Column::Size as usize] = Some(220);
        let wide = widths(1250.0, [true; 7], layout);
        assert_eq!(
            wide.iter().find(|(c, _)| *c == Column::Size).unwrap().1,
            220.0
        );
        assert!(
            wide.iter().position(|(c, _)| *c == Column::Size)
                < wide.iter().position(|(c, _)| *c == Column::Name)
        );
        let narrow = widths(320.0, [true; 7], layout);
        assert!(narrow.iter().map(|(_, w)| w).sum::<f32>() > 320.0);
        for (column, width) in &narrow {
            assert!(*width >= f32::from(column.minimum()));
        }
        assert!(
            !narrow
                .iter()
                .any(|(c, _)| matches!(c, Column::Number | Column::Added | Column::Eta))
        );
        let hidden = widths(320.0, [false; 7], layout);
        assert_eq!(
            hidden.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
            [Column::Name, Column::Actions]
        );
    }

    use super::super::interaction_tests::{fixture, frame, text_rect};
    const SIZE: egui::Vec2 = egui::vec2(1440.0, 860.0);
    #[test]
    fn long_column_headings_elide_inside_their_cell_instead_of_clipping_words() {
        let (_temp, mut app, ctx) = fixture();
        app.interface.language = download_engine::i18n::Language::German;
        let _language = download_engine::i18n::scope(app.interface.language);
        let output = settled(&mut app, &ctx);
        let text = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == tr("Speed") => {
                    Some((text, shape.clip_rect))
                }
                _ => None,
            })
            .expect("Translated speed column");
        assert!(text.0.galley.elided);
        assert!(
            text.1
                .contains_rect(Rect::from_min_size(text.0.pos, text.0.galley.size()))
        );
    }

    fn pointer(app: &mut Desktop, ctx: &egui::Context, pos: egui::Pos2, pressed: Option<bool>) {
        let mut events = vec![egui::Event::PointerMoved(pos)];
        if let Some(pressed) = pressed {
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        frame(app, ctx, SIZE, events);
    }
    fn settled(app: &mut Desktop, ctx: &egui::Context) -> egui::FullOutput {
        frame(app, ctx, SIZE, vec![]);
        frame(app, ctx, SIZE, vec![])
    }
    fn heading(output: &egui::FullOutput, label: &str) -> Rect {
        let top = text_rect(output, "File name").top();
        output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::epaint::Shape::Text(text) = &shape.shape
                    && text.galley.job.text == label
                    && (text.pos.y - top).abs() < 1.0
                {
                    Some(Rect::from_min_size(text.pos, text.galley.size()))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("Missing header {label}"))
    }
    fn drag(app: &mut Desktop, ctx: &egui::Context, from: egui::Pos2, to: egui::Pos2) {
        pointer(app, ctx, from, Some(true));
        pointer(app, ctx, from.lerp(to, 0.5), None);
        pointer(app, ctx, to, None);
        pointer(app, ctx, to, Some(false));
        settled(app, ctx);
    }
    #[test]
    fn header_drag_reorders_then_resizes_and_keeps_file_selection_working() {
        let (_temp, mut app, ctx) = fixture();
        let output = settled(&mut app, &ctx);
        let from = heading(&output, "Size").center();
        let to = text_rect(&output, "File name").left_center();
        drag(&mut app, &ctx, from, to);
        let layout = app.interface.table_layout;
        assert!(
            layout.order.iter().position(|c| *c == Column::Size)
                < layout.order.iter().position(|c| *c == Column::Name)
        );
        assert_eq!(
            app.store.interface_preferences().unwrap().table_layout,
            layout
        );
        let output = settled(&mut app, &ctx);
        // The boundary before File name is now the right edge of Size.
        let name = text_rect(&output, "File name");
        let boundary = pos2(name.left() - 10.0, name.center().y);
        drag(&mut app, &ctx, boundary, boundary + vec2(70.0, 0.0));
        assert_eq!(
            app.interface.table_layout.widths[Column::Size as usize],
            Some(168)
        );
        assert_eq!(
            app.store.interface_preferences().unwrap().table_layout,
            app.interface.table_layout
        );
        let output = settled(&mut app, &ctx);
        let pos = text_rect(&output, "guide.pdf").center();
        pointer(&mut app, &ctx, pos, Some(true));
        pointer(&mut app, &ctx, pos, Some(false));
        assert_eq!(
            app.selected.as_deref(),
            Some(
                app.jobs
                    .iter()
                    .find(|j| j.name == "guide.pdf")
                    .unwrap()
                    .id
                    .as_str()
            )
        );
    }
    #[test]
    fn resize_cancellation_and_failed_save_do_not_replace_saved_layout() {
        let (_temp, mut app, ctx) = fixture();
        let output = settled(&mut app, &ctx);
        let status = heading(&output, "Status");
        let boundary = pos2(status.left() - 10.0, status.center().y);
        pointer(&mut app, &ctx, boundary, Some(true));
        pointer(&mut app, &ctx, boundary + vec2(100.0, 0.0), None);
        assert!(app.table.resize.is_some());
        frame(
            &mut app,
            &ctx,
            SIZE,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        pointer(&mut app, &ctx, boundary, Some(false));
        assert_eq!(app.interface.table_layout, TableLayout::default());
        assert_eq!(
            app.store.interface_preferences().unwrap().table_layout,
            TableLayout::default()
        );
        let conn =
            crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3"))
                .unwrap();
        conn.execute_batch("CREATE TRIGGER reject_layout BEFORE UPDATE ON interface_preferences BEGIN SELECT RAISE(ABORT,'fixture save failure'); END;").unwrap();
        let output = settled(&mut app, &ctx);
        let from = heading(&output, "Size").center();
        let to = text_rect(&output, "File name").center();
        drag(&mut app, &ctx, from, to);
        assert_eq!(app.interface.table_layout, TableLayout::default());
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .contains("fixture save failure")
        );
    }
    #[test]
    fn header_drop_outside_table_cancels_reordering() {
        let (_temp, mut app, ctx) = fixture();
        let output = settled(&mut app, &ctx);
        let from = heading(&output, "Size").center();
        drag(&mut app, &ctx, from, egui::pos2(15.0, 15.0));
        assert_eq!(app.interface.table_layout, TableLayout::default());
        assert_eq!(
            app.store.interface_preferences().unwrap().table_layout,
            TableLayout::default()
        );
    }

    #[test]
    fn horizontal_scrolling_moves_headers_with_rows_and_reaches_actions() {
        let (_temp, mut app, ctx) = fixture();
        app.interface.table_layout.widths[Column::Name as usize] = Some(1000);
        let output = settled(&mut app, &ctx);
        let old_header = heading(&output, "Status").left();
        let row_text = |output: &egui::FullOutput| {
            let header = heading(output, "Status");
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(t)
                        if t.galley.job.text.starts_with("Paused")
                            && t.pos.y > header.bottom()
                            && t.pos.y < header.bottom() + 80.0 =>
                    {
                        Some(t.pos.x)
                    }
                    _ => None,
                })
                .expect("Paused status in the first table row")
        };
        let old_row = row_text(&output);
        let pos = pos2(1000.0, 380.0);
        frame(
            &mut app,
            &ctx,
            SIZE,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: vec2(-5000.0, 0.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let mut output = settled(&mut app, &ctx);
        for _ in 0..15 {
            output = frame(&mut app, &ctx, SIZE, vec![]);
        }
        let new_header = heading(&output, "Status").left();
        let new_row = row_text(&output);
        assert!(
            new_header < old_header - 100.0,
            "Expected horizontal scrolling"
        );
        assert!((old_header - new_header - (old_row - new_row)).abs() < 1.0);
        assert!(heading(&output, "Actions").right() < SIZE.x - 12.0);
    }
    #[test]
    fn double_clicking_a_divider_restores_automatic_width() {
        let (_temp, mut app, ctx) = fixture();
        app.interface.table_layout.widths[Column::Name as usize] = Some(500);
        app.store.save_interface_preferences(app.interface).unwrap();
        let output = settled(&mut app, &ctx);
        let size = heading(&output, "Size");
        let edge = pos2(size.left() - 10.0, size.center().y);
        for _ in 0..2 {
            pointer(&mut app, &ctx, edge, Some(true));
            pointer(&mut app, &ctx, edge, Some(false));
        }
        assert_eq!(
            app.interface.table_layout.widths[Column::Name as usize],
            None
        );
        assert_eq!(
            app.store
                .interface_preferences()
                .unwrap()
                .table_layout
                .widths[Column::Name as usize],
            None
        );
    }
}
