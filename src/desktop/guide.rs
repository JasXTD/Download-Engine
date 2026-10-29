//! Offline help, compiled from the same document shipped in the package.
use super::*;

static DOCUMENT: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| include_str!("../../docs/user-guide.md").replace("\r\n", "\n"));
static SIMPLIFIED: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    include_str!("../../docs/locales/zh-CN/user-guide.md").replace("\r\n", "\n")
});
static TRADITIONAL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    include_str!("../../docs/locales/zh-TW/user-guide.md").replace("\r\n", "\n")
});
static MALAY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    include_str!("../../docs/locales/ms/user-guide.md").replace("\r\n", "\n")
});
static ADDITIONAL: std::sync::LazyLock<[String; 15]> = std::sync::LazyLock::new(|| {
    [
        include_str!("../../docs/locales/ja/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/ko/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/vi/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/th/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/pl/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/ru/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/uk/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/ar/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/de/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/fr/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/sv/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/no/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/de-CH/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/it/user-guide.md").replace("\r\n", "\n"),
        include_str!("../../docs/locales/rm/user-guide.md").replace("\r\n", "\n"),
    ]
});
static CHANGELOG: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| include_str!("../../CHANGELOG.md").replace("\r\n", "\n"));

fn topics() -> Vec<(&'static str, &'static str)> {
    use download_engine::i18n::Language;
    let document: &str = match download_engine::i18n::current() {
        Language::SimplifiedChinese => &SIMPLIFIED,
        Language::TraditionalChinese => &TRADITIONAL,
        Language::Malay => &MALAY,
        Language::Japanese => &ADDITIONAL[0],
        Language::Korean => &ADDITIONAL[1],
        Language::Vietnamese => &ADDITIONAL[2],
        Language::Thai => &ADDITIONAL[3],
        Language::Polish => &ADDITIONAL[4],
        Language::Russian => &ADDITIONAL[5],
        Language::Ukrainian => &ADDITIONAL[6],
        Language::Arabic => &ADDITIONAL[7],
        Language::German => &ADDITIONAL[8],
        Language::French => &ADDITIONAL[9],
        Language::Swedish => &ADDITIONAL[10],
        Language::NorwegianBokmal => &ADDITIONAL[11],
        Language::SwissGerman => &ADDITIONAL[12],
        Language::Italian => &ADDITIONAL[13],
        Language::Romansh => &ADDITIONAL[14],
        _ => &DOCUMENT,
    };
    document
        .split("\n## ")
        .skip(1)
        .filter_map(|section| section.split_once('\n'))
        .collect()
}

impl Desktop {
    pub(super) fn open_guide(&mut self, topic: &str) {
        // Callers use stable English topic IDs; the visible title is translated.
        self.guide_topic = DOCUMENT
            .split("\n## ")
            .skip(1)
            .position(|section| section.lines().next() == Some(topic))
            .unwrap_or(0);
        self.view = View::Guide;
    }

    pub(super) fn guide_page(&mut self, ui: &mut egui::Ui, p: Palette) {
        ui.label(RichText::new(tr("Guide")).size(20.0).strong());
        ui.label(RichText::new(tr("Quick steps and examples. Available offline.")).color(p.muted));
        ui.add_space(12.0);
        let topics = topics();
        // One topic per row beside the article: nothing wraps at any width or language.
        let list_width = (ui.available_width() * 0.26).clamp(150.0, 210.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(list_width, ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("guide-topics")
                        .auto_shrink([true, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            for (index, (title, _)) in topics.iter().enumerate() {
                                if topic_row(ui, title, index == self.guide_topic, list_width, p)
                                    .clicked()
                                {
                                    self.guide_topic = index;
                                }
                            }
                        });
                },
            );
            ui.add_space(6.0);
            let Some((title, body)) = topics.get(self.guide_topic) else {
                return;
            };
            // The row is horizontal; the article itself must still flow top to bottom.
            ui.allocate_ui_with_layout(
                ui.available_size(),
                egui::Layout::top_down(egui::Align::Min),
                |ui| article(ui, p, ("guide", self.guide_topic), title, body),
            );
        });
    }

    pub(super) fn changelog_page(&mut self, ui: &mut egui::Ui, p: Palette) {
        ui.label(RichText::new(tr("Changelog")).size(20.0).strong());
        ui.label(
            RichText::new(tf!("Installed version {}", env!("CARGO_PKG_VERSION"))).color(p.accent),
        );
        ui.label(
            RichText::new(tr("What's changed, newest first. Available offline.")).color(p.muted),
        );
        ui.add_space(12.0);
        // The introductory document heading is already provided by the page header.
        let start = CHANGELOG.find("\n## ").expect("Changelog release heading");
        use download_engine::i18n::Language;
        let latest = match download_engine::i18n::current() {
            Language::SimplifiedChinese => {
                Some(include_str!("../../docs/locales/zh-CN/release-notes.md"))
            }
            Language::TraditionalChinese => {
                Some(include_str!("../../docs/locales/zh-TW/release-notes.md"))
            }
            Language::Malay => Some(include_str!("../../docs/locales/ms/release-notes.md")),
            Language::Japanese => Some(include_str!("../../docs/locales/ja/release-notes.md")),
            Language::Korean => Some(include_str!("../../docs/locales/ko/release-notes.md")),
            Language::Vietnamese => Some(include_str!("../../docs/locales/vi/release-notes.md")),
            Language::Thai => Some(include_str!("../../docs/locales/th/release-notes.md")),
            Language::Polish => Some(include_str!("../../docs/locales/pl/release-notes.md")),
            Language::Russian => Some(include_str!("../../docs/locales/ru/release-notes.md")),
            Language::Ukrainian => Some(include_str!("../../docs/locales/uk/release-notes.md")),
            Language::Arabic => Some(include_str!("../../docs/locales/ar/release-notes.md")),
            Language::German => Some(include_str!("../../docs/locales/de/release-notes.md")),
            Language::French => Some(include_str!("../../docs/locales/fr/release-notes.md")),
            Language::Swedish => Some(include_str!("../../docs/locales/sv/release-notes.md")),
            Language::NorwegianBokmal => {
                Some(include_str!("../../docs/locales/no/release-notes.md"))
            }
            Language::SwissGerman => {
                Some(include_str!("../../docs/locales/de-CH/release-notes.md"))
            }
            Language::Italian => Some(include_str!("../../docs/locales/it/release-notes.md")),
            Language::Romansh => Some(include_str!("../../docs/locales/rm/release-notes.md")),
            _ => None,
        };
        let localized;
        let body = if let Some(latest) = latest {
            localized = localized_changelog(latest);
            localized.as_str()
        } else {
            &CHANGELOG[start + 1..]
        };
        article(ui, p, "changelog", tr("Release notes"), body);
    }
}

/// Keep archived translations in source without repeating releases in Help.
fn current_release(notes: &str) -> &str {
    notes
        .split_once("\n## ")
        .map_or(notes, |(latest, _)| latest)
}
fn localized_changelog(notes: &str) -> String {
    let normalized = notes.replace("\r\n", "\n");
    let latest = current_release(&normalized).trim_end();
    let history = CHANGELOG.splitn(3, "\n## ").nth(2).unwrap_or("");
    format!(
        "{latest}\n\n### {}\n\n## {history}",
        tr("Earlier release notes (English)")
    )
}

/// A full-width, left-aligned topic row. Long translated titles end with … and show
/// in full on hover.
fn topic_row(
    ui: &mut egui::Ui,
    title: &str,
    selected: bool,
    width: f32,
    p: Palette,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            response.enabled(),
            selected,
            title,
        )
    });
    if selected {
        ui.painter().rect_filled(rect, 6, p.selected);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 6, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            6,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    let mut label = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(10.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let text = RichText::new(title).color(p.text);
    label.add(
        egui::Label::new(if selected { text.strong() } else { text })
            .truncate()
            .selectable(false),
    );
    response
}

fn article(ui: &mut egui::Ui, p: Palette, id: impl egui::AsIdSalt, title: &str, body: &str) {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .fill(p.background)
                .stroke((1.0, p.border))
                .corner_radius(8)
                .inner_margin(16)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(title).size(18.0).strong());
                    ui.add_space(8.0);
                    // These owned documents use paragraphs, headings and fenced code only.
                    // Render text without executing links, Markdown HTML or code samples.
                    for (index, part) in body.split("```").enumerate() {
                        if index % 2 == 1 {
                            if let Some((language, code)) = part.split_once('\n') {
                                ui.push_id(index, |ui| {
                                    code_block(ui, p, language, code.trim_end())
                                });
                            }
                        } else {
                            for paragraph in
                                part.split("\n\n").map(str::trim).filter(|s| !s.is_empty())
                            {
                                if let Some(heading) = paragraph.strip_prefix("## ") {
                                    ui.add_space(8.0);
                                    ui.label(RichText::new(heading).size(18.0).strong());
                                } else if let Some(heading) = paragraph.strip_prefix("### ") {
                                    ui.add_space(8.0);
                                    ui.label(RichText::new(heading).strong().color(p.accent));
                                } else {
                                    ui.add(
                                        egui::Label::new(paragraph.replace('\n', " "))
                                            .wrap()
                                            .selectable(true),
                                    );
                                }
                                ui.add_space(6.0);
                            }
                        }
                    }
                });
        });
}

fn code_block(ui: &mut egui::Ui, p: Palette, language: &str, code: &str) {
    egui::Frame::new()
        .fill(p.surface)
        .stroke((1.0, p.border))
        .corner_radius(6)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(language).small().color(p.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(if language == "lua" {
                            tr("Copy script")
                        } else {
                            tr("Copy commands")
                        })
                        .on_hover_text(tr("Copy this example to the clipboard"))
                        .clicked()
                    {
                        ui.ctx().copy_text(code.to_owned());
                    }
                });
            });
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(RichText::new(code).monospace())
                    .wrap()
                    .selectable(true),
            );
        });
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::{click, fixture, frame, text_rect};
    use super::*;

    #[test]
    fn translated_release_notes_cover_the_current_release() {
        // Help → Changelog shows these in place of the newest English release, followed
        // by the English history from the second release on. A stale file would repeat
        // one release and hide the newest.
        let latest = CHANGELOG.split("\n## ").nth(1).unwrap();
        let heading = format!("## {}", latest.lines().next().unwrap());
        let sections = latest.matches("\n### ").count();
        let languages: serde_json::Value =
            serde_json::from_str(include_str!("../../locales/languages.json")).unwrap();
        for code in languages
            .as_array()
            .unwrap()
            .iter()
            .map(|language| language["code"].as_str().unwrap())
            .filter(|code| *code != "en")
        {
            let path = format!(
                "{}/docs/locales/{code}/release-notes.md",
                env!("CARGO_MANIFEST_DIR")
            );
            let notes = std::fs::read_to_string(&path).unwrap();
            assert_eq!(
                notes.lines().next(),
                Some(heading.as_str()),
                "{code}: release notes are not for the current release"
            );
            assert_eq!(
                current_release(&notes).matches("\n### ").count(),
                sections,
                "{code}: sections differ"
            );
            // Older translations may be retained, but the displayed history must
            // contain exactly one copy of every release, in canonical order.
            let displayed = localized_changelog(&notes);
            let actual: Vec<_> = displayed
                .lines()
                .filter(|line| line.starts_with("## "))
                .collect();
            let expected: Vec<_> = CHANGELOG
                .lines()
                .filter(|line| line.starts_with("## "))
                .collect();
            assert_eq!(
                actual, expected,
                "{code}: duplicated or missing displayed release"
            );
            // The article view splits paragraphs and headings on "\n\n".
            assert!(!notes.contains('\r'), "{code}: use LF line endings");
        }
    }

    #[test]
    fn changelog_menu_shows_current_release_and_preserves_download_state() {
        let (_temp, mut app, ctx) = fixture();
        app.search = "first".into();
        let selected = app.selected.clone();
        click(&mut app, &ctx, "Help");
        click(&mut app, &ctx, "Changelog");
        assert_eq!(app.view, View::Changelog);
        let version = env!("CARGO_PKG_VERSION");
        let release = CHANGELOG
            .split("\n## ")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap();
        assert!(release.starts_with(&format!("{version} — ")));
        for dark in [true, false] {
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
                frame(&mut app, &ctx, size, vec![]);
                let output = frame(&mut app, &ctx, size, vec![]);
                let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                for label in [format!("Installed version {version}"), release.to_owned()] {
                    assert!(
                        viewport.contains_rect(text_rect(&output, &label)),
                        "Clipped: {label}"
                    );
                }
                for clipped in output.shapes {
                    if let egui::epaint::Shape::Text(text) = clipped.shape {
                        let rect = text.galley.rect.translate(text.pos.to_vec2());
                        if rect.intersects(clipped.clip_rect) {
                            assert!(
                                rect.right() <= size.x + 1.0,
                                "Text overflows: {}",
                                text.galley.job.text
                            );
                        }
                    }
                }
            }
        }
        click(&mut app, &ctx, "Download list");
        assert_eq!(app.view, View::Downloads);
        assert_eq!(app.search, "first");
        assert_eq!(app.selected, selected);
    }

    #[test]
    fn guide_menu_topics_clipboard_and_return_preserve_download_state() {
        let (_temp, mut app, ctx) = fixture();
        app.search = "first".into();
        let selected = app.selected.clone();
        click(&mut app, &ctx, "Help");
        click(&mut app, &ctx, "Guide");
        assert_eq!(app.view, View::Guide);
        click(&mut app, &ctx, "Command line");
        let size = egui::vec2(1080.0, 800.0);
        frame(&mut app, &ctx, size, vec![]);
        let out = frame(&mut app, &ctx, size, vec![]);
        let pos = text_rect(&out, "Copy commands").center();
        let pointer = |pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame(&mut app, &ctx, size, pointer(true));
        let output = frame(&mut app, &ctx, size, pointer(false));
        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if text.starts_with(".\\de-cli.exe --help\n") && text.ends_with(".\\de-cli.exe list"))
        }));
        click(&mut app, &ctx, "Download list");
        assert_eq!(app.view, View::Downloads);
        assert_eq!(app.search, "first");
        assert_eq!(app.selected, selected);
    }

    #[test]
    fn guide_topics_and_content_fit_both_themes_and_window_sizes() {
        let (_temp, mut app, ctx) = fixture();
        for dark in [true, false] {
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
                for (title, body) in topics() {
                    app.open_guide(title);
                    frame(&mut app, &ctx, size, vec![]);
                    let out = frame(&mut app, &ctx, size, vec![]);
                    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                    for (topic, _) in topics() {
                        assert!(
                            viewport.contains_rect(text_rect(&out, topic)),
                            "Topic clipped: {topic}"
                        );
                    }
                    let first_heading = body
                        .trim()
                        .lines()
                        .next()
                        .unwrap()
                        .strip_prefix("### ")
                        .unwrap();
                    let heading = text_rect(&out, first_heading);
                    assert!(viewport.contains_rect(heading));
                    // One aligned column of topics beside the article; nothing wraps.
                    // Toolbar labels and the article title can repeat a topic name, so
                    // measure only text below the page subtitle and left of the article.
                    let subtitle = text_rect(&out, "Quick steps and examples. Available offline.");
                    let rows: Vec<_> = topics()
                        .iter()
                        .map(|(topic, _)| {
                            out.shapes
                                .iter()
                                .find_map(|shape| match &shape.shape {
                                    egui::epaint::Shape::Text(text)
                                        if text.galley.job.text.trim() == *topic =>
                                    {
                                        let rect = text.galley.rect.translate(text.pos.to_vec2());
                                        (rect.top() > subtitle.bottom()
                                            && rect.right() < heading.left())
                                        .then_some(rect)
                                    }
                                    _ => None,
                                })
                                .unwrap_or_else(|| panic!("Topic row missing: {topic}"))
                        })
                        .collect();
                    for pair in rows.windows(2) {
                        assert!(
                            (pair[0].left() - pair[1].left()).abs() < 1.0,
                            "Topics are not one aligned list"
                        );
                        assert!(pair[1].top() >= pair[0].bottom(), "Topics overlap or wrap");
                    }
                    assert!(rows.iter().all(|row| row.right() < heading.left()));
                    for clipped in out.shapes {
                        if let egui::epaint::Shape::Text(text) = clipped.shape {
                            let rect = text.galley.rect.translate(text.pos.to_vec2());
                            if rect.intersects(clipped.clip_rect) {
                                assert!(
                                    rect.right() <= size.x + 1.0,
                                    "Text overflows: {}",
                                    text.galley.job.text
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn f1_opens_guide_but_does_not_discard_a_dialog() {
        let (_temp, mut app, ctx) = fixture();
        let event = egui::Event::Key {
            key: egui::Key::F1,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            &mut app,
            &ctx,
            egui::vec2(860.0, 600.0),
            vec![event.clone()],
        );
        assert_eq!(app.view, View::Guide);
        app.view = View::Downloads;
        app.open_add();
        frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![event]);
        assert_eq!(app.dialog, Some(Dialog::Add));
        assert_eq!(app.view, View::Downloads);
    }

    #[test]
    fn mcp_setup_opens_connection_instructions() {
        let (_temp, mut app, ctx) = fixture();
        click(&mut app, &ctx, "Help");
        click(&mut app, &ctx, "Guide");
        click(&mut app, &ctx, "MCP integration");
        assert_eq!(app.view, View::Guide);
        assert_eq!(topics()[app.guide_topic].0, "MCP integration");
        let out = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
        text_rect(&out, "Connect a local AI client");
    }

    #[test]
    fn lua_help_entry_opens_the_topic_and_copies_only_the_script() {
        let (_temp, mut app, ctx) = fixture();
        click(&mut app, &ctx, "Help");
        click(&mut app, &ctx, "Guide");
        click(&mut app, &ctx, "Lua scripting");
        assert_eq!(app.view, View::Guide);
        assert_eq!(topics()[app.guide_topic].0, "Lua scripting");
        let size = egui::vec2(1280.0, 860.0);
        frame(&mut app, &ctx, size, vec![]);
        let out = frame(&mut app, &ctx, size, vec![]);
        text_rect(&out, "Run your first Lua script");
        let pos = text_rect(&out, "Copy script").center();
        let pointer = |pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame(&mut app, &ctx, size, pointer(true));
        let out = frame(&mut app, &ctx, size, pointer(false));
        let example = topics()[app.guide_topic]
            .1
            .split("```lua\n")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap()
            .trim_end();
        assert!(out.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(code) if code == example && code.starts_with("-- Example: hello\n"))
        }));
    }

    #[test]
    fn automation_links_to_its_guide_topic() {
        let (_temp, mut app, ctx) = fixture();
        app.view = View::Automation;
        click(&mut app, &ctx, "How rules work");
        assert_eq!(app.view, View::Guide);
        assert_eq!(topics()[app.guide_topic].0, "Scripting");
    }
}
