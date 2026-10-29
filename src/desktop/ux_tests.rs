use super::interaction_tests::{click, fixture, frame, text_rect};
use super::*;

const SMALL: egui::Vec2 = egui::vec2(860.0, 600.0);

#[test]
fn filtered_list_preserves_order_unicode_matching_and_selection() {
    let (_temp, mut app, _ctx) = fixture();
    app.jobs[0].name = "RÉSUMÉ.ZIP".into();
    app.jobs[2].name = "résumé-final.zip".into();
    let ids: Vec<_> = app.jobs.iter().map(|job| job.id.clone()).collect();
    app.set_selection(Some(ids[0].clone()));
    app.selection.additional.extend(ids[1..].iter().cloned());
    app.search = "  RÉSUMÉ  ".into();
    app.reconcile_selection();
    assert_eq!(app.visible_job_indices(), [0, 2]);
    assert_eq!(app.visible_job_ids(), [ids[0].clone(), ids[2].clone()]);
    assert_eq!(app.selected_job_refs().count(), 2);
    app.search.clear();
    app.reconcile_selection();
    assert_eq!(app.visible_job_indices(), [0, 1, 2]);
    assert_eq!(
        app.selected_job_refs().count(),
        2,
        "Hidden selections must not return"
    );
    app.filter = Filter::Failed;
    app.reconcile_selection();
    assert_eq!(app.selected.as_deref(), Some(ids[2].as_str()));
    assert_eq!(app.selected_jobs()[0].name, "résumé-final.zip");
}

#[test]
fn compact_search_controls_and_queue_actions_fit_translated_headers() {
    use download_engine::i18n::Language;
    let (_temp, mut app, ctx) = fixture();
    for language in [
        Language::English,
        Language::German,
        Language::Arabic,
        Language::Malay,
    ] {
        app.interface.language = language;
        let _scope = download_engine::i18n::scope(language);
        for filter in [Filter::All, Filter::Unfinished, Filter::Downloading] {
            app.filter = filter;
            app.search = "zip".into();
            let output = settled(&mut app, &ctx);
            assert_visible(&output, tr("Clear"));
            assert_visible(&output, filter.title());
            let title = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if text.galley.job.text == filter.title() && text.pos.x >= 190.0 =>
                    {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .expect("Visible list heading, rather than the sidebar label");
            let search = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == "zip" => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .expect("Visible search text");
            assert!(
                !title.intersects(search),
                "Search overlaps the translated heading"
            );
        }
    }
}

#[test]
fn list_queue_controls_start_and_pause_an_empty_queue() {
    let (_temp, mut app, ctx) = fixture();
    // The fixture jobs are presentation-only: the store has no runnable downloads.
    assert!(app.store.list().unwrap().is_empty());
    app.command(Command::PauseQueue(MAIN_QUEUE.into()));
    let wait_for_queue = |app: &Desktop, running: bool| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app
            .store
            .queues()
            .unwrap()
            .iter()
            .find(|q| q.id == MAIN_QUEUE)
            .unwrap()
            .running
            != running
        {
            assert!(
                Instant::now() < deadline,
                "Queue command was not acknowledged"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    wait_for_queue(&app, false);
    app.refresh_and_wait();
    click(&mut app, &ctx, "Start queue");
    wait_for_queue(&app, true);
    app.refresh_and_wait();
    click(&mut app, &ctx, "Pause queue");
    wait_for_queue(&app, false);
    assert!(app.store.list().unwrap().is_empty());
}

#[test]
fn status_hover_explains_failure_without_opening_details_or_changing_selection() {
    let (_temp, mut app, ctx) = fixture();
    app.interface.show_details = false;
    let selected = app.selected.clone();
    let output = settled(&mut app, &ctx);
    let failed = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Failed" && text.pos.x >= 190.0 => {
                Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
        .expect("Failed status in the download row");
    frame(
        &mut app,
        &ctx,
        SMALL,
        vec![egui::Event::PointerMoved(failed.center())],
    );
    assert!((0..90).any(|_| {
        frame(&mut app, &ctx, SMALL, vec![]).shapes.iter().any(|shape|
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Server returned HTTP 403"))
    }), "The failure reason should be available directly from the status");
    assert_eq!(app.selected, selected);
    assert!(!app.interface.show_details);
    assert!(app.dialog.is_none());
}

/// Observe whole-frame cost with a large queue; never use workstation timing as
/// a pass/fail threshold. Run the same fixture against before/after binaries.
#[test]
#[ignore = "opt-in large download list benchmark"]
fn large_download_list_frame_time() {
    let (_temp, mut app, ctx) = fixture();
    let sample = app.jobs[0].clone();
    app.jobs = (0..5000)
        .map(|index| {
            let mut job = sample.clone();
            job.id = format!("fixture-{index:05}");
            job.name = format!("Archive-{index:05}.zip");
            job
        })
        .collect();
    app.set_selection(Some(app.jobs[0].id.clone()));
    let size = egui::vec2(1280.0, 860.0);
    for search in ["", "archive-04"] {
        app.search = search.into();
        app.reconcile_selection();
        for _ in 0..5 {
            drop(frame(&mut app, &ctx, size, vec![]));
        }
        let start = Instant::now();
        for _ in 0..60 {
            drop(frame(&mut app, &ctx, size, vec![]));
        }
        println!(
            "UI_FRAME_BENCH {}",
            serde_json::json!({"jobs": app.jobs.len(), "search": search,
                "frames": 60, "milliseconds_per_frame": start.elapsed().as_secs_f64() * 1000.0 / 60.0})
        );
    }
}

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
