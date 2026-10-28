//! Application information and offline license documents.
use super::*;
use egui::{Align, FontFamily, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

const GITHUB_URL: &str = download_engine::updates::REPOSITORY_URL;
// Keep the reference design's proportions while presenting a compact About card.
const SCALE: f32 = 0.7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum LegalPage {
    Application,
    ThirdParty,
}

impl LegalPage {
    fn title(self) -> &'static str {
        match self {
            Self::Application => tr("Application license"),
            Self::ThirdParty => tr("Third-party notices"),
        }
    }

    fn text(self) -> &'static str {
        // Embed the authoritative files so this view also works offline and in
        // standalone builds. Copying preserves their original wording and format.
        match self {
            Self::Application => include_str!("../../LICENSE"),
            Self::ThirdParty => include_str!("../../THIRD-PARTY-NOTICES.md"),
        }
    }
}

impl Desktop {
    pub(super) fn about_dialog(&mut self, ctx: &egui::Context) {
        let dark = ctx.theme() == egui::Theme::Dark;
        let p = Palette::new(dark);
        let background = if dark {
            Color32::from_rgb(29, 32, 39)
        } else {
            p.background
        };
        let border = if dark {
            Color32::from_rgb(60, 65, 80)
        } else {
            Color32::from_rgb(207, 214, 229)
        };
        let ink = if dark {
            Color32::from_rgb(245, 246, 250)
        } else {
            p.text
        };
        let muted = if dark {
            Color32::from_rgb(168, 175, 197)
        } else {
            p.muted
        };
        let blue = if dark {
            Color32::from_rgb(105, 170, 255)
        } else {
            p.blue
        };
        let bold = FontFamily::Name("bold".into());
        let title_family = if ctx.fonts(|fonts| fonts.definitions().families.contains_key(&bold)) {
            bold
        } else {
            FontFamily::Proportional
        };
        let build_version = env!("CARGO_PKG_VERSION");
        let frame = egui::Frame::new()
            .fill(background)
            .stroke(Stroke::new(1.5 * SCALE, border))
            .corner_radius(13)
            .inner_margin(20)
            .shadow(egui::Shadow {
                offset: [0, 7],
                blur: 22,
                spread: 0,
                color: Color32::from_black_alpha(80),
            });
        let response = egui::Modal::new(egui::Id::new("about-download-engine"))
            .frame(frame)
            .show(ctx, |ui| {
                let width = (ctx.content_rect().width() - 112.0 * SCALE)
                    .clamp(300.0 * SCALE, 580.0 * SCALE);
                let compact = width < 500.0 * SCALE;
                let logo_size = if compact { 60.0 } else { 76.0 } * SCALE;
                let close_size = 32.0;
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 0.0;
                let (header, _) = ui.allocate_exact_size(vec2(width, logo_size), Sense::hover());
                download_emblem(
                    ui.painter(),
                    Rect::from_min_size(header.min, vec2(logo_size, logo_size)),
                );
                let close_rect = Rect::from_center_size(
                    pos2(header.right() - close_size / 2.0, header.center().y),
                    vec2(close_size, close_size),
                );
                let title_rect = Rect::from_min_max(
                    pos2(header.left() + logo_size + 24.0 * SCALE, header.top()),
                    pos2(close_rect.left() - 18.0 * SCALE, header.bottom()),
                );
                let mut title_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(title_rect)
                        .layout(egui::Layout::left_to_right(Align::Center)),
                );
                title_ui.add(
                    egui::Label::new(
                        RichText::new("Download Engine")
                            .font(FontId::new(
                                if compact { 27.0 } else { 36.0 } * SCALE,
                                title_family.clone(),
                            ))
                            .color(ink),
                    )
                    .truncate(),
                );
                // Same outlined × as every other dialog, with a stable id for focus handling.
                let close = ui
                    .interact(close_rect, egui::Id::new("about-close"), Sense::click())
                    .on_hover_text(tr("Close About"));
                view::paint_close(ui, &close);
                close.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        close.enabled(),
                        tr("Close About"),
                    )
                });
                #[cfg(feature = "screenshot")]
                if std::env::var("IDM_EX_TEST_TOOLBAR").as_deref() == Ok(tr("Close About")) {
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(
                            egui::Id::new("toolbar-capture-target"),
                            (close.id, close.rect),
                        );
                    });
                }
                ui.add_space(20.0 * SCALE);
                let (divider, _) = ui.allocate_exact_size(vec2(width, SCALE), Sense::hover());
                ui.painter().line_segment(
                    [divider.left_center(), divider.right_center()],
                    Stroke::new(SCALE, border),
                );
                ui.add_space(16.0 * SCALE);
                ui.label(
                    RichText::new(tf!(
                        "Version {build_version}",
                        build_version = build_version
                    ))
                    .font(FontId::new(32.0 * SCALE, title_family))
                    .color(blue),
                );
                ui.add_space(18.0 * SCALE);
                info_row(
                    ui,
                    InfoIcon::Author,
                    tr("Created by Xetro"),
                    None,
                    muted,
                    background,
                );
                ui.add_space(4.0 * SCALE);
                info_row(
                    ui,
                    InfoIcon::Github,
                    "GitHub",
                    Some(GITHUB_URL),
                    blue,
                    background,
                );
                ui.add_space(16.0 * SCALE);
                let legal = ui
                    .add_sized(
                        vec2(width, view::CONTROL_HEIGHT),
                        egui::Button::new(tr("Legal & licenses")),
                    )
                    .clicked();
                (close.clicked(), legal)
            });
        if response.inner.0 || response.should_close() {
            self.dialog = None;
        } else if response.inner.1 {
            self.dialog = Some(Dialog::Legal(LegalPage::Application));
        }
    }

    pub(super) fn legal_dialog(&mut self, ctx: &egui::Context, mut page: LegalPage) {
        let p = Palette::new(ctx.theme() == egui::Theme::Dark);
        let response = form_dialog::modal("about-legal", p).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 80.0).clamp(280.0, 680.0));
            ui.spacing_mut().interact_size.y = view::CONTROL_HEIGHT;
            let close = view::dialog_header_enabled(ui, tr("Legal & licenses"), true);
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for tab in [LegalPage::Application, LegalPage::ThirdParty] {
                    ui.selectable_value(&mut page, tab, tab.title());
                }
            });
            ui.add_space(8.0);
            egui::Frame::new()
                .fill(p.surface)
                .stroke((1.0, p.border))
                .corner_radius(6)
                .inner_margin(14)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("legal-document", page))
                        .max_height((ctx.content_rect().height() - 240.0).clamp(120.0, 480.0))
                        .auto_shrink([false, false])
                        .show(ui, |ui| license_text(ui, page.text()));
                });
            ui.add_space(8.0);
            let back = ui
                .horizontal_wrapped(|ui| {
                    let back = ui.button(tr("Back to About")).clicked();
                    if ui.button(tr("Copy text")).clicked() {
                        ctx.copy_text(page.text().to_owned());
                    }
                    back
                })
                .inner;
            close || back
        });
        self.dialog = Some(if response.inner || response.should_close() {
            Dialog::About
        } else {
            Dialog::Legal(page)
        });
    }
}

fn license_text(ui: &mut egui::Ui, text: &str) {
    // These bundled documents are displayed as text only: paths, links and any
    // commands in the notices cannot execute. Preserve license wording verbatim.
    let text = text.replace("\r\n", "\n");
    for (index, paragraph) in text.split("\n\n").enumerate() {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        let heading = paragraph
            .strip_prefix("# ")
            .or_else(|| paragraph.strip_prefix("## "));
        let label = if let Some(heading) = heading {
            RichText::new(heading).strong().size(16.0)
        } else if index == 0 {
            RichText::new(paragraph).strong().size(18.0)
        } else if paragraph.starts_with("- ") {
            RichText::new(paragraph)
        } else {
            RichText::new(paragraph.replace('\n', " "))
        };
        ui.add(egui::Label::new(label).wrap().selectable(true));
        ui.add_space(8.0);
    }
}

fn download_emblem(painter: &egui::Painter, rect: Rect) {
    view::gradient_panel(
        painter,
        rect,
        15.0 * SCALE,
        Color32::from_rgb(68, 46, 142),
        Color32::from_rgb(35, 28, 74),
    );
    painter.rect_stroke(
        rect,
        11,
        Stroke::new(1.8 * SCALE, Color32::from_rgb(137, 108, 241)),
        StrokeKind::Inside,
    );
    let point = |x, y| rect.min + vec2(x, y) * (rect.width() / 40.0);
    let stroke = Stroke::new(rect.width() * 0.05, Color32::from_rgb(225, 214, 255));
    painter.line_segment([point(20.0, 10.0), point(20.0, 24.0)], stroke);
    painter.add(egui::Shape::line(
        vec![point(13.0, 18.0), point(20.0, 25.0), point(27.0, 18.0)],
        stroke,
    ));
    painter.add(egui::Shape::line(
        vec![
            point(9.0, 26.0),
            point(9.0, 31.0),
            point(31.0, 31.0),
            point(31.0, 26.0),
        ],
        stroke,
    ));
}

enum InfoIcon {
    Author,
    Github,
}

fn info_row(
    ui: &mut egui::Ui,
    icon: InfoIcon,
    label: &str,
    url: Option<&str>,
    color: Color32,
    background: Color32,
) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0 * SCALE), Sense::hover());
    let rect = Rect::from_center_size(
        pos2(row.left() + 14.0 * SCALE, row.center().y),
        vec2(26.0, 26.0) * SCALE,
    );
    let point = |x, y| rect.min + vec2(x, y) * (rect.width() / 24.0);
    let painter = ui.painter();
    match icon {
        InfoIcon::Author => {
            let stroke = Stroke::new(1.8 * SCALE, color);
            painter.circle_stroke(point(12.0, 6.0), 4.3 * SCALE, stroke);
            painter.add(egui::Shape::closed_line(
                vec![
                    point(3.5, 22.0),
                    point(4.0, 18.0),
                    point(6.0, 15.0),
                    point(9.0, 14.0),
                    point(15.0, 14.0),
                    point(18.0, 15.0),
                    point(20.0, 18.0),
                    point(20.5, 22.0),
                ],
                stroke,
            ));
        }
        InfoIcon::Github => {
            painter.circle_filled(rect.center(), 12.0 * SCALE, color);
            painter.rect_filled(
                Rect::from_min_max(point(5.0, 7.0), point(19.0, 16.0)),
                3,
                background,
            );
            for points in [
                [(5.0, 9.0), (5.5, 4.0), (11.0, 7.0)],
                [(19.0, 9.0), (18.5, 4.0), (13.0, 7.0)],
            ] {
                painter.add(egui::Shape::convex_polygon(
                    points.map(|(x, y)| point(x, y)).to_vec(),
                    background,
                    Stroke::NONE,
                ));
            }
            painter.rect_filled(
                Rect::from_min_max(point(10.0, 14.0), point(16.0, 24.0)),
                2,
                background,
            );
            painter.add(egui::Shape::line(
                vec![
                    point(11.0, 20.0),
                    point(7.0, 20.0),
                    point(5.0, 17.0),
                    point(3.0, 16.0),
                ],
                Stroke::new(2.3 * SCALE, background),
            ));
        }
    }
    let label_rect = Rect::from_min_max(pos2(row.left() + 44.0 * SCALE, row.top()), row.max);
    let text = RichText::new(label).size(19.0 * SCALE).color(color);
    let mut label_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(label_rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    if let Some(url) = url {
        label_ui.add(egui::Hyperlink::from_label_and_url(text, url));
    } else {
        label_ui.label(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};

    fn click_output(app: &mut Desktop, ctx: &egui::Context, label: &str) -> egui::FullOutput {
        let size = vec2(860.0, 600.0);
        frame(app, ctx, size, vec![]);
        let output = frame(app, ctx, size, vec![]);
        let pos = text_rect(&output, label).center();
        frame(
            app,
            ctx,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(
            app,
            ctx,
            size,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
    }

    #[test]
    fn legal_documents_open_copy_and_return_without_changing_downloads() {
        let (_temp, mut app, ctx) = fixture();
        let selected = app.selected.clone();
        let jobs = serde_json::to_value(&app.jobs).unwrap();
        click(&mut app, &ctx, "Help");
        click(&mut app, &ctx, "About");
        click(&mut app, &ctx, "Legal & licenses");
        assert_eq!(app.dialog, Some(Dialog::Legal(LegalPage::Application)));
        for (page, source) in [
            (LegalPage::Application, include_str!("../../LICENSE")),
            (
                LegalPage::ThirdParty,
                include_str!("../../THIRD-PARTY-NOTICES.md"),
            ),
        ] {
            click(&mut app, &ctx, page.title());
            assert_eq!(app.dialog, Some(Dialog::Legal(page)));
            let output = click_output(&mut app, &ctx, "Copy text");
            assert!(output.platform_output.commands.iter().any(
                |command| matches!(command, egui::OutputCommand::CopyText(text) if text == source)
            ));
            assert!(
                !output
                    .platform_output
                    .commands
                    .iter()
                    .any(|command| matches!(command, egui::OutputCommand::OpenUrl(_)))
            );
            // The original document's final paragraph must be reachable in the
            // smallest supported window; the footer must stay outside the scroll.
            let size = vec2(860.0, 600.0);
            frame(
                &mut app,
                &ctx,
                size,
                vec![
                    egui::Event::PointerMoved(egui::pos2(430.0, 300.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: vec2(0.0, -10000.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            for _ in 0..20 {
                frame(&mut app, &ctx, size, vec![]);
            }
            let output = frame(&mut app, &ctx, size, vec![]);
            let normalized = source.replace("\r\n", "\n");
            let ending = normalized
                .trim()
                .rsplit("\n\n")
                .next()
                .unwrap()
                .replace('\n', " ");
            let rect = text_rect(&output, &ending);
            assert!(
                output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text)
                    if text.galley.job.text.trim() == ending && shape.clip_rect.contains_rect(rect))
                }),
                "Final license paragraph cannot be read"
            );
            assert!(text_rect(&output, "Back to About").top() > rect.bottom());
        }
        click(&mut app, &ctx, "Back to About");
        assert_eq!(app.dialog, Some(Dialog::About));
        click(&mut app, &ctx, "Legal & licenses");
        // Reopening always starts with the application's own license.
        assert_eq!(app.dialog, Some(Dialog::Legal(LegalPage::Application)));
        // Let egui register the newly opened modal before sending its next input.
        frame(&mut app, &ctx, vec2(860.0, 600.0), vec![]);
        frame(
            &mut app,
            &ctx,
            vec2(860.0, 600.0),
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(app.dialog, Some(Dialog::About));
        click(&mut app, &ctx, "Legal & licenses");
        click(&mut app, &ctx, "×");
        assert_eq!(app.dialog, Some(Dialog::About));
        assert_eq!(app.selected, selected);
        assert_eq!(serde_json::to_value(&app.jobs).unwrap(), jobs);
    }

    #[test]
    fn legal_entry_opens_from_the_keyboard() {
        let (_temp, mut app, ctx) = fixture();
        let size = vec2(860.0, 600.0);
        app.dialog = Some(Dialog::About);
        frame(&mut app, &ctx, size, vec![egui::Event::PointerGone]);
        frame(&mut app, &ctx, size, vec![]);
        // Close, GitHub, then Legal & licenses are keyboard reachable in visual order.
        let mut opened_repository = false;
        for key in [
            egui::Key::Tab,
            egui::Key::Tab,
            egui::Key::Enter,
            egui::Key::Tab,
            egui::Key::Enter,
        ] {
            for pressed in [true, false] {
                let output = frame(
                    &mut app,
                    &ctx,
                    size,
                    vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                opened_repository |= output.platform_output.commands.iter().any(|command|
                    matches!(command, egui::OutputCommand::OpenUrl(url) if url.url == GITHUB_URL));
            }
        }
        assert!(opened_repository);
        assert_eq!(app.dialog, Some(Dialog::Legal(LegalPage::Application)));
    }

    #[test]
    fn legal_controls_fit_all_languages_and_both_themes() {
        use download_engine::i18n::Language;
        let (_temp, mut app, ctx) = fixture();
        for language in Language::ALL {
            app.interface.language = language;
            let _scope = download_engine::i18n::scope(language);
            for dark in [false, true] {
                app.interface.appearance = if dark {
                    Appearance::Dark
                } else {
                    Appearance::Light
                };
                app.configure_appearance(&ctx);
                for size in [vec2(860.0, 600.0), vec2(1280.0, 860.0)] {
                    for page in [LegalPage::Application, LegalPage::ThirdParty] {
                        app.dialog = Some(Dialog::Legal(page));
                        frame(&mut app, &ctx, size, vec![]);
                        let output = frame(&mut app, &ctx, size, vec![]);
                        for label in [
                            tr("Legal & licenses"),
                            tr("Application license"),
                            tr("Third-party notices"),
                            tr("Back to About"),
                            tr("Copy text"),
                        ] {
                            let rect = text_rect(&output, label);
                            assert!(
                                Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                                "{language:?}: {label} outside the window"
                            );
                            assert!(
                                output.shapes.iter().any(|shape| {
                                    matches!(&shape.shape, egui::Shape::Text(text)
                                    if text.galley.job.text.trim() == label
                                        && shape.clip_rect.contains_rect(rect))
                                }),
                                "{language:?}: {label} clipped"
                            );
                        }
                    }
                }
            }
        }
    }
}
