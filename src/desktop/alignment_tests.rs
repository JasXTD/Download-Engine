//! Shared edges: controls that end one row or share one column line up exactly,
//! including with longer translated labels and at the minimum window size.
use super::interaction_tests::{click, fixture, frame};
use super::*;
use download_engine::i18n::Language;
use egui::Rect;
use egui::epaint::Shape;

const SMALL: egui::Vec2 = egui::vec2(860.0, 600.0);
const WIDE: egui::Vec2 = egui::vec2(1280.0, 860.0);
/// Pixel snapping may move a widget edge by up to one physical pixel.
const TOLERANCE: f32 = 1.0;

fn settled(app: &mut Desktop, ctx: &egui::Context, size: egui::Vec2) -> egui::FullOutput {
    for _ in 0..5 {
        frame(app, ctx, size, vec![]);
    }
    frame(app, ctx, size, vec![])
}

/// Every rendered copy of `label`, in paint order (a modal's copy comes last).
fn texts(output: &egui::FullOutput, label: &str) -> Vec<Rect> {
    texts_where(output, |text| text.trim() == label)
}

fn texts_where(output: &egui::FullOutput, matches: impl Fn(&str) -> bool) -> Vec<Rect> {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Text(text) if matches(&text.galley.job.text) => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            _ => None,
        })
        .collect()
}

fn top(output: &egui::FullOutput, label: &str) -> Rect {
    *texts(output, label)
        .last()
        .unwrap_or_else(|| panic!("No rendered text: {label}"))
}

fn rects(output: &egui::FullOutput) -> Vec<Rect> {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Rect(shape) => Some(shape.rect),
            _ => None,
        })
        .collect()
}

/// The smallest painted frame (button, field or dropdown) around `inner`.
fn frame_around(output: &egui::FullOutput, inner: Rect) -> Rect {
    rects(output)
        .into_iter()
        .filter(|rect| rect.contains(inner.center()) && rect.expand(1.0).contains_rect(inner))
        .filter(|rect| rect.width() > inner.width() + 2.0)
        .min_by(|a, b| a.area().total_cmp(&b.area()))
        .unwrap_or_else(|| panic!("No frame around {inner:?}"))
}

fn same(values: &[f32], what: &str) {
    let min = values.iter().copied().fold(f32::INFINITY, f32::min);
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!(max - min <= TOLERANCE, "{what} differ: {values:?}");
}

fn translated(language: Language, text: &'static str) -> String {
    let _scope = download_engine::i18n::scope(language);
    tr(text).to_owned()
}

fn dark(app: &mut Desktop, ctx: &egui::Context, language: Language) {
    app.interface.language = language;
    app.interface.appearance = Appearance::Dark;
    app.configure_appearance(ctx);
}

#[test]
fn settings_trailing_controls_share_one_right_edge() {
    for language in [Language::English, Language::German] {
        for size in [SMALL, WIDE] {
            let (_temp, mut app, ctx) = fixture();
            dark(&mut app, &ctx, language);
            app.open_preferences_window();
            let output = settled(&mut app, &ctx, size);
            let mut edges = vec![
                frame_around(&output, top(&output, language.native_name())).right(),
                frame_around(&output, top(&output, &translated(language, "Dark"))).right(),
            ];
            let switches: Vec<_> = rects(&output)
                .into_iter()
                .filter(|rect| {
                    (rect.width() - 34.0).abs() < 0.5 && (rect.height() - 18.0).abs() < 0.5
                })
                .map(|rect| rect.right())
                .collect();
            assert!(switches.len() >= 3, "Expected the appearance switches");
            edges.extend(switches);
            edges.extend(texts(&output, "›").iter().map(|rect| rect.right()));
            same(
                &edges,
                &format!("{language:?} {size:?} settings trailing edges"),
            );
        }
    }
}

#[test]
fn add_download_right_column_shares_one_edge() {
    for language in [Language::English, Language::German] {
        let (_temp, mut app, ctx) = fixture();
        dark(&mut app, &ctx, language);
        app.url = "https://example.com/archive.zip".into();
        app.open_add();
        let output = settled(&mut app, &ctx, SMALL);
        let paste = frame_around(&output, top(&output, &translated(language, "Paste")));
        let queue = frame_around(&output, top(&output, &translated(language, "Main queue")));
        let browse = frame_around(&output, top(&output, &translated(language, "Browse…")));
        same(
            &[paste.right(), queue.right(), browse.right()],
            &format!("{language:?} Add download right edges"),
        );
    }
}

#[test]
fn queue_page_actions_end_at_the_card_edge_inside_the_window() {
    for language in [Language::English, Language::German] {
        for size in [SMALL, WIDE] {
            let (_temp, mut app, ctx) = fixture();
            dark(&mut app, &ctx, language);
            app.view = View::Queues;
            let output = settled(&mut app, &ctx, size);
            let what = format!("{language:?} {size:?}");
            let new_queue =
                frame_around(&output, top(&output, &translated(language, "+ New queue")));
            let menu = frame_around(&output, top(&output, "⋯"));
            let track = rects(&output)
                .into_iter()
                .find(|rect| (rect.height() - 4.0).abs() < 0.5 && rect.width() > 200.0)
                .expect("Queue progress track");
            let card = rects(&output)
                .into_iter()
                .filter(|rect| rect.contains_rect(menu) && rect.contains_rect(track))
                .min_by(|a, b| a.area().total_cmp(&b.area()))
                .expect("Queue card");
            same(
                &[menu.right(), track.right()],
                &format!("{what} card action and progress edges"),
            );
            same(
                &[new_queue.right(), card.right()],
                &format!("{what} page action and card edges"),
            );
            assert!(
                card.right() <= size.x,
                "{what}: card overflows the window {card:?}"
            );
        }
    }
}

#[test]
fn long_queue_names_stop_before_the_card_actions() {
    for language in Language::ALL {
        let (_temp, mut app, ctx) = fixture();
        dark(&mut app, &ctx, language);
        app.store.create_queue(&"W".repeat(64)).unwrap();
        app.force_refresh();
        app.view = View::Queues;
        let output = settled(&mut app, &ctx, WIDE);
        let states = texts(&output, &format!("• {}", translated(language, "Paused")));
        let starts: Vec<_> = texts(&output, &translated(language, "Start"))
            .into_iter()
            .map(|text| frame_around(&output, text))
            .collect();
        // The fixture's Main queue runs; the long-named queue is paused.
        assert!(
            !states.is_empty(),
            "{language:?}: the paused queue shows its state"
        );
        for state in states {
            let button = starts
                .iter()
                .find(|button| button.y_range().intersects(state.y_range()))
                .unwrap_or_else(|| panic!("{language:?}: Start button beside the name"));
            assert!(
                state.right() < button.left(),
                "{language:?}: state {state:?} runs into {button:?}"
            );
        }
    }
}

#[test]
fn speed_limit_and_quota_inputs_share_one_column() {
    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::German);
    app.selected = Some(app.jobs[0].id.clone());
    app.open_speed_limits();
    let output = settled(&mut app, &ctx, SMALL);
    let inputs: Vec<_> = texts(&output, "1024")
        .into_iter()
        .map(|text| frame_around(&output, text).left())
        .collect();
    assert_eq!(inputs.len(), 2, "Total and per-file speed inputs");
    same(&inputs, "Speed limit inputs");

    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::German);
    app.open_data_quotas();
    let output = settled(&mut app, &ctx, SMALL);
    let inputs: Vec<_> = texts(&output, "1024")
        .into_iter()
        .map(|text| frame_around(&output, text).left())
        .collect();
    assert_eq!(inputs.len(), 2, "Hourly and daily quota inputs");
    same(&inputs, "Quota inputs");
}

#[test]
fn table_columns_dialog_keeps_one_width_column_and_the_primary_action_last() {
    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::English);
    app.open_columns();
    // Short enough that the column list scrolls with 26px rows.
    let output = settled(&mut app, &ctx, egui::vec2(860.0, 480.0));
    let autos: Vec<_> = texts(&output, "Auto")
        .into_iter()
        .map(|text| frame_around(&output, text).left())
        .collect();
    assert!(autos.len() >= 6, "Expected visible Auto buttons");
    same(&autos, "Auto buttons");
    let save = frame_around(&output, top(&output, "Save"));
    let reset = frame_around(&output, top(&output, "Reset"));
    assert!(
        save.left() > reset.right(),
        "The primary action ends the footer"
    );
    // A shrunken body puts its scrollbar in the middle of the dialog.
    let bar = rects(&output)
        .into_iter()
        .filter(|rect| rect.width() <= 10.0 && rect.height() > 60.0 && rect.top() > 100.0)
        .max_by(|a, b| a.right().total_cmp(&b.right()))
        .expect("Column list scrollbar at the minimum size");
    assert!(
        bar.right() >= save.right() - 12.0,
        "Scrollbar {bar:?} should sit at the dialog edge {save:?}"
    );
}

#[test]
fn video_review_rows_and_destination_labels_line_up() {
    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::English);
    app.browser_video_sample(false);
    let output = settled(&mut app, &ctx, SMALL);
    let file = top(&output, "Nature-documentary-1080p.mp4");
    let stream = top(&output, "Nature documentary");
    same(&[file.left(), stream.left()], "Video row names");
    for label in ["Queue", "Save to"] {
        let label_rect = top(&output, label);
        let combo = rects(&output)
            .into_iter()
            .filter(|rect| {
                rect.left() > label_rect.right()
                    && rect.left() < label_rect.right() + 24.0
                    && rect.height() >= 20.0
                    && rect.y_range().contains(label_rect.center().y)
            })
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .unwrap_or_else(|| panic!("Dropdown next to {label}"));
        same(
            &[label_rect.center().y, combo.center().y],
            &format!("{label} label and dropdown centers"),
        );
    }
    let reload = frame_around(&output, top(&output, "Reload destinations"));
    let add = frame_around(&output, top(&output, "Add selected files"));
    assert!(
        reload.height() >= 24.0,
        "Reload destinations is a regular button: {reload:?}"
    );
    assert!(add.height() >= 24.0);
}

#[test]
fn script_name_row_ends_at_the_editor_edge() {
    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::English);
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    let output = settled(&mut app, &ctx, SMALL);
    let value = *texts_where(&output, |text| text.trim().starts_with("300"))
        .last()
        .unwrap_or_else(|| {
            let shown: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    Shape::Text(text) => Some(text.galley.job.text.clone()),
                    _ => None,
                })
                .collect();
            panic!("No Timeout value among {shown:?}")
        });
    let timeout = frame_around(&output, value);
    let counter = *texts_where(&output, |text| text.ends_with("/ 64 KiB"))
        .last()
        .expect("Editor size counter");
    same(
        &[timeout.right(), counter.right()],
        "Timeout field and editor edges",
    );
}

#[test]
fn details_close_button_is_centered_on_the_file_actions_button() {
    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::English);
    app.selected = Some(app.jobs[0].id.clone());
    let output = settled(&mut app, &ctx, WIDE);
    let actions = frame_around(&output, top(&output, "File actions"));
    let close = rects(&output)
        .into_iter()
        .filter(|rect| {
            (rect.width() - rect.height()).abs() < 0.5
                && rect.left() > actions.right()
                && (rect.center().y - actions.center().y).abs() < 12.0
        })
        .min_by(|a, b| a.left().total_cmp(&b.left()))
        .expect("Hide details button");
    same(
        &[close.center().y, actions.center().y],
        "Details action centers",
    );
}

#[test]
fn connection_overrides_fill_the_dialog_width() {
    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::English);
    let original = app.store.connection_settings().unwrap();
    let mut settings = original.clone();
    for host in ["downloads.example.com", "cdn.example.com"] {
        settings.sites.push(download_engine::connections::Site {
            host: host.into(),
            subdomains: false,
            connections: 1,
        });
    }
    app.store
        .save_connection_settings(&original, settings)
        .unwrap();
    app.open_connections();
    let output = settled(&mut app, &ctx, SMALL);
    let save = frame_around(&output, top(&output, "Save settings"));
    // The toolbar also has a Remove button; the dialog's two are painted last.
    let remove: Vec<_> = texts(&output, "Remove")
        .into_iter()
        .rev()
        .take(2)
        .map(|text| frame_around(&output, text))
        .collect();
    assert!(
        remove.iter().all(|rect| rect.top() > 120.0),
        "Dialog Remove buttons"
    );
    for button in &remove {
        // Rows of the download list behind the dialog can also contain the button;
        // the card is the smallest frame that stays near the dialog's edge.
        let card = rects(&output)
            .into_iter()
            .filter(|rect| {
                rect.contains_rect(*button)
                    && rect.width() > 300.0
                    && rect.right() <= save.right() + 40.0
            })
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .expect("Website override card");
        same(
            &[card.right(), save.right()],
            "Override card and dialog edges",
        );
    }
    same(
        &remove.iter().map(|rect| rect.right()).collect::<Vec<_>>(),
        "Remove buttons",
    );
}

fn heights(output: &egui::FullOutput, labels: &[&str]) -> Vec<(String, f32)> {
    labels
        .iter()
        .map(|label| {
            let height = frame_around(output, top(output, label)).height();
            ((*label).to_owned(), height)
        })
        .collect()
}

fn assert_height(found: &[(String, f32)], expected: f32, what: &str) {
    for (label, height) in found {
        assert!(
            (height - expected).abs() <= 0.5,
            "{what}: {label} is {height}px, not {expected}px"
        );
    }
}

#[test]
fn form_controls_share_one_height_and_primary_actions_stay_taller() {
    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::English);
    app.url.clear();
    app.open_add();
    let output = settled(&mut app, &ctx, SMALL);
    let controls = heights(
        &output,
        &[
            "Paste",
            "Main queue",
            "Browse…",
            "https://example.com/file.zip",
            "Use the server's file name",
        ],
    );
    assert_height(&controls, view::CONTROL_HEIGHT, "Add download");
    assert_height(
        // The fixture's Main queue runs, so new downloads start right away.
        &heights(&output, &["Start download"]),
        view::PRIMARY_HEIGHT,
        "Add download footer",
    );

    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::English);
    app.browser_video_sample(false);
    let output = settled(&mut app, &ctx, SMALL);
    let controls = heights(
        &output,
        &["Search videos…", "Choose quality", "Reload destinations"],
    );
    assert_height(&controls, view::CONTROL_HEIGHT, "Video downloads");

    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::English);
    app.view = View::Queues;
    let output = settled(&mut app, &ctx, SMALL);
    assert_height(
        &heights(&output, &["Schedule", "View downloads", "⋯"]),
        view::CONTROL_HEIGHT,
        "Queue card",
    );
    assert_height(
        &heights(&output, &["+ New queue"]),
        view::PRIMARY_HEIGHT,
        "Queues header",
    );

    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::English);
    app.open_preferences_window();
    let output = settled(&mut app, &ctx, SMALL);
    assert_height(
        &heights(&output, &["English", "Dark"]),
        view::CONTROL_HEIGHT,
        "Preferences",
    );
}

#[test]
fn checkboxes_are_small_squares_not_circles() {
    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::English);
    app.open_download_folders();
    let output = settled(&mut app, &ctx, SMALL);
    let boxes: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Rect(shape)
                if (shape.rect.width() - view::CHECKBOX_SIZE).abs() < 0.5
                    && (shape.rect.height() - view::CHECKBOX_SIZE).abs() < 0.5 =>
            {
                Some(shape.corner_radius)
            }
            _ => None,
        })
        .collect();
    assert!(!boxes.is_empty(), "Expected the folder checkbox");
    for radius in boxes {
        assert_eq!(radius, egui::CornerRadius::same(4), "Checkbox corners");
    }
}

#[test]
fn toolbar_labels_share_one_size_and_top_line() {
    for language in [
        Language::English,
        Language::German,
        Language::Arabic,
        Language::Japanese,
    ] {
        for size in [SMALL, WIDE] {
            let (_temp, mut app, ctx) = fixture();
            dark(&mut app, &ctx, language);
            let output = settled(&mut app, &ctx, size);
            // Toolbar labels sit between the menu bar and the content.
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    // Labels only: icons are 22px glyphs, labels at most 11px.
                    Shape::Text(text)
                        if (30.0..108.0).contains(&text.pos.y)
                            && !text.galley.job.text.trim().is_empty()
                            && text.galley.job.sections[0].format.font_id.size <= 11.0 =>
                    {
                        Some((
                            text.galley.job.sections[0].format.font_id.size,
                            text.pos.y + text.galley.rect.top(),
                        ))
                    }
                    _ => None,
                })
                .collect();
            assert!(
                labels.len() >= 15,
                "{language:?}: expected 15 toolbar labels"
            );
            let what = format!("{language:?} {size:?}");
            same(
                &labels.iter().map(|(size, _)| *size).collect::<Vec<_>>(),
                &format!("{what} label sizes"),
            );
            same(
                &labels.iter().map(|(_, top)| *top).collect::<Vec<_>>(),
                &format!("{what} label tops"),
            );
        }
    }
}

#[test]
fn download_list_scrollbar_strip_is_painted_like_the_header() {
    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::English);
    let output = settled(&mut app, &ctx, WIDE);
    let header = Palette::new(true).background;
    let strip = output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            Shape::Rect(shape)
                if (shape.rect.width() - 14.0).abs() < 0.5 && shape.rect.height() > 100.0 =>
            {
                Some(shape.fill)
            }
            _ => None,
        })
        .expect("Painted scrollbar strip beside the download list");
    assert_eq!(strip, header, "The strip uses the header colour");
}

#[test]
fn checkbox_labels_wrap_as_a_whole_in_wrapping_rows() {
    let (_temp, mut app, ctx) = fixture();
    dark(&mut app, &ctx, Language::English);
    app.view = View::Automation;
    click(&mut app, &ctx, "Scripts");
    click(&mut app, &ctx, "+ New script");
    // A queue choice fills the row, so the checkbox has to move to the next one.
    click(&mut app, &ctx, "Manual only");
    click(&mut app, &ctx, "Download completed");
    let output = settled(&mut app, &ctx, SMALL);
    let label = output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            Shape::Text(text) if text.galley.job.text == "Automatic trigger enabled" => {
                Some(text.galley.rows.len())
            }
            _ => None,
        })
        .expect("Automatic trigger checkbox");
    assert_eq!(
        label, 1,
        "The checkbox moves to the next row instead of splitting its label"
    );
}
