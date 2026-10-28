use super::interaction_tests::{click, fixture, frame, text_rect};
use super::*;

const SMALL: egui::Vec2 = egui::vec2(860.0, 600.0);

#[test]
fn download_focus_outline_follows_the_active_row_without_outlining_empty_space() {
    for dark in [false, true] {
        let (_temp, mut app, ctx) = fixture();
        app.interface.appearance = if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        app.configure_appearance(&ctx);
        click(&mut app, &ctx, "guide.pdf");
        for name in ["guide.pdf", "failed.zip"] {
            let output = settled(&mut app, &ctx);
            assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("download-list"))));
            let outlines: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Rect(rect)
                        if [Palette::new(false).blue, Palette::new(true).blue]
                            .contains(&rect.stroke.color)
                            && rect.stroke.width > 0.0 =>
                    {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(outlines.len(), 1);
            assert!(
                outlines[0].height() <= 48.0,
                "Focus outline covers the list's empty space: {:?}",
                outlines[0]
            );
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text)
                if text.galley.job.text == name && outlines[0].contains_rect(egui::Rect::from_min_size(text.pos,text.galley.size())))));
            key(&mut app, &ctx, egui::Key::ArrowDown, egui::Modifiers::NONE);
        }
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("download-list"))));
    }
}

fn settled(app: &mut Desktop, ctx: &egui::Context) -> egui::FullOutput {
    frame(app, ctx, SMALL, vec![]);
    frame(app, ctx, SMALL, vec![])
}

fn assert_visible(output: &egui::FullOutput, label: &str) {
    let rect = text_rect(output, label);
    assert!(
        egui::Rect::from_min_size(egui::Pos2::ZERO, SMALL).contains_rect(rect),
        "Outside minimum window: {label} {rect:?}"
    );
    assert!(
        output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::epaint::Shape::Text(text)
            if text.galley.job.text.trim() == label && shape.clip_rect.contains_rect(rect))
        }),
        "Clipped control: {label}"
    );
}

fn key(app: &mut Desktop, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            SMALL,
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            }],
        );
    }
}

#[test]
fn ux_review_enter_in_url_adds_exactly_one_download() {
    let (_temp, mut app, ctx) = fixture();
    let now = chrono::Utc::now().timestamp();
    app.store
        .schedule_queue(MAIN_QUEUE, now + 3600, now)
        .unwrap();
    app.refresh_and_wait();
    app.url = "https://example.invalid/archive.zip".into();
    app.open_add();
    settled(&mut app, &ctx);
    key(&mut app, &ctx, egui::Key::Enter, egui::Modifiers::NONE);
    app.wait_for_io();
    assert_eq!(app.dialog, None);
    let jobs = app.store.list().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].url, "https://example.invalid/archive.zip");
    assert_eq!(jobs[0].status, Status::Queued);
    assert_eq!(app.selected.as_deref(), Some(jobs[0].id.as_str()));
}

#[test]
fn ux_review_add_close_and_escape_preserve_draft_without_enqueueing() {
    let (_temp, mut app, ctx) = fixture();
    app.url = "https://example.invalid/archive.zip".into();
    app.name = "custom.zip".into();
    app.expected_sha256 = "a".repeat(64);
    app.open_add();
    click(&mut app, &ctx, "×");
    assert_eq!(app.dialog, None);
    app.open_add();
    settled(&mut app, &ctx);
    assert_eq!(app.url, "https://example.invalid/archive.zip");
    assert_eq!(app.name, "custom.zip");
    assert_eq!(app.expected_sha256, "a".repeat(64));
    key(&mut app, &ctx, egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(app.dialog, None);
    assert!(app.store.list().unwrap().is_empty());
}

#[test]
fn ux_review_shortcuts_focus_search_clear_it_and_respect_dialogs() {
    let (_temp, mut app, ctx) = fixture();
    let ctrl = egui::Modifiers {
        ctrl: true,
        command: true,
        ..egui::Modifiers::NONE
    };
    app.view = View::Queues;
    settled(&mut app, &ctx);
    key(&mut app, &ctx, egui::Key::F, ctrl);
    assert_eq!(app.view, View::Downloads);
    let search = egui::Id::new("filename-search");
    assert!(ctx.memory(|memory| memory.has_focus(search)));
    frame(
        &mut app,
        &ctx,
        SMALL,
        vec![egui::Event::Text("guide".into())],
    );
    assert_eq!(app.search, "guide");
    assert_eq!(app.selected.as_deref(), Some(app.jobs[1].id.as_str()));
    key(&mut app, &ctx, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(app.search.is_empty());
    assert!(ctx.memory(|memory| memory.has_focus(search)));
    key(&mut app, &ctx, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!ctx.memory(|memory| memory.has_focus(search)));
    key(&mut app, &ctx, egui::Key::N, ctrl);
    assert_eq!(app.dialog, Some(Dialog::Add));
    app.url = "draft URL".into();
    key(&mut app, &ctx, egui::Key::F, ctrl);
    assert_eq!(app.dialog, Some(Dialog::Add));
    assert_eq!(app.url, "draft URL");
    key(&mut app, &ctx, egui::Key::Escape, egui::Modifiers::NONE);
    app.open_preferences_window();
    settled(&mut app, &ctx);
    key(&mut app, &ctx, egui::Key::N, ctrl);
    key(&mut app, &ctx, egui::Key::F, ctrl);
    assert_eq!(app.dialog, Some(Dialog::PreferencesWindow));
    key(&mut app, &ctx, egui::Key::Escape, egui::Modifiers::NONE);
    click(&mut app, &ctx, "Tasks");
    assert!(egui::Popup::is_any_open(&ctx));
    key(&mut app, &ctx, egui::Key::N, ctrl);
    assert_eq!(app.dialog, None);
    assert!(app.store.list().unwrap().is_empty());
}

#[test]
fn ux_review_empty_states_offer_working_recovery_actions() {
    let (_temp, mut app, ctx) = fixture();
    app.filter = Filter::Complete;
    app.category = Some(app.jobs[0].category());
    app.search = "no matching filename".into();
    app.reconcile_selection();
    let output = settled(&mut app, &ctx);
    assert_visible(&output, "Clear filters");
    click(&mut app, &ctx, "Clear filters");
    assert_eq!(app.filter, Filter::All);
    assert_eq!(app.category, None);
    assert!(app.search.is_empty());
    assert_eq!(app.selected.as_deref(), Some(app.jobs[0].id.as_str()));
    app.jobs.clear();
    app.reconcile_selection();
    let output = settled(&mut app, &ctx);
    assert_visible(&output, "Add download");
    click(&mut app, &ctx, "Add download");
    assert_eq!(app.dialog, Some(Dialog::Add));
    assert!(app.store.list().unwrap().is_empty());
}

#[test]
fn ux_review_queue_controls_fit_minimum_window() {
    let (_temp, mut app, ctx) = fixture();
    app.view = View::Queues;
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(theme);
        for synchronize in [false, true] {
            app.queues[0].synchronize = synchronize;
            let output = settled(&mut app, &ctx);
            for label in ["Schedule", "View downloads", "⋯"] {
                assert_visible(&output, label);
            }
        }
    }
}

#[test]
fn ux_review_add_error_keeps_actions_visible() {
    let (_temp, mut app, ctx) = fixture();
    app.queues[0].name = "W".repeat(64);
    app.url = "https://example.com/archive.zip".into();
    app.open_add();
    app.form_error = Some(format!(
        "Cannot create download folder {}: access is denied. Choose another folder and try again.",
        "C:\\Downloads\\".repeat(12)
    ));
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(theme);
        let output = settled(&mut app, &ctx);
        assert_visible(&output, "Add download");
        assert_visible(&output, "Start download");
        assert_visible(&output, "×");
    }
}

#[test]
fn ux_review_enter_in_dropdown_does_not_submit_download() {
    let (_temp, mut app, ctx) = fixture();
    app.store.set_queue_running(MAIN_QUEUE, false).unwrap();
    app.refresh_and_wait();
    app.url = "ftps://example.com/archive.zip".into();
    app.open_add();
    click(
        &mut app,
        &ctx,
        download_engine::ftp::Security::Implicit.label(),
    );
    assert!(egui::Popup::is_any_open(&ctx));
    frame(
        &mut app,
        &ctx,
        SMALL,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(app.dialog, Some(Dialog::Add));
    assert!(app.store.list().unwrap().is_empty());
}

#[test]
fn ux_review_search_returns_to_first_result_after_scrolling() {
    let (_temp, mut app, ctx) = fixture();
    app.store.set_queue_running(MAIN_QUEUE, false).unwrap();
    app.interface.show_details = false;
    let template = app.jobs[0].clone();
    app.jobs = (0..120)
        .map(|index| {
            let mut job = template.clone();
            job.id = Uuid::new_v4().to_string();
            job.request_id = Uuid::new_v4().to_string();
            job.name = format!("file-{index:03}.zip");
            job
        })
        .collect();
    app.store.enqueue_batch(&app.jobs, MAIN_QUEUE).unwrap();
    app.refresh_and_wait();
    settled(&mut app, &ctx);
    let output = scroll_down(&mut app, &ctx);
    assert!(
        !has_text(&output, "file-000.zip"),
        "Fixture must be scrolled"
    );
    app.refresh_and_wait();
    let refreshed = settled(&mut app, &ctx);
    assert!(
        !has_text(&refreshed, "file-000.zip"),
        "Refresh must keep scroll position"
    );
    app.search = "file-0".into();
    app.reconcile_selection();
    let output = settled(&mut app, &ctx);
    assert_visible(&output, "file-000.zip");

    let other = app.store.create_queue("Other queue").unwrap();
    let jobs: Vec<_> = app
        .jobs
        .iter()
        .cloned()
        .map(|mut job| {
            job.id = Uuid::new_v4().to_string();
            job.request_id = Uuid::new_v4().to_string();
            job.name = job.name.replace(".zip", "-other.zip");
            job
        })
        .collect();
    app.store.enqueue_batch(&jobs, &other.id).unwrap();
    let output = scroll_down(&mut app, &ctx);
    assert!(!has_text(&output, "file-000.zip"));
    app.queue_id = other.id;
    app.refresh_and_wait();
    let output = settled(&mut app, &ctx);
    assert_visible(&output, "file-000-other.zip");
}

fn has_text(output: &egui::FullOutput, label: &str) -> bool {
    output.shapes.iter().any(|shape| {
        matches!(
            &shape.shape, egui::epaint::Shape::Text(text) if text.galley.job.text == label
        )
    })
}

fn scroll_down(app: &mut Desktop, ctx: &egui::Context) -> egui::FullOutput {
    frame(
        app,
        ctx,
        SMALL,
        vec![
            egui::Event::PointerMoved(egui::pos2(480.0, 300.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: egui::vec2(0.0, -1400.0),
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let mut output = settled(app, ctx);
    for _ in 0..20 {
        output = frame(app, ctx, SMALL, vec![]);
    }
    output
}
