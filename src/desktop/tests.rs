use super::*;
#[test]
fn synchronization_queue_creation_and_controls_fit_the_compact_interface() {
    use super::interaction_tests::{click, fixture, frame, text_rect};
    let (_temp, mut app, ctx) = fixture();
    app.open_queue_dialog(Dialog::NewQueue);
    let queue_count = app.store.queues().unwrap().len();
    click(&mut app, &ctx, "Create queue");
    assert_eq!(app.form_error.as_deref(), Some("Enter a queue name"));
    assert_eq!(app.store.queues().unwrap().len(), queue_count);
    app.queue_name = "Main queue".into();
    click(&mut app, &ctx, "Create queue");
    assert!(
        app.form_error
            .as_deref()
            .unwrap()
            .contains("choose a unique name")
    );
    assert_eq!(app.store.queues().unwrap().len(), queue_count);
    app.queue_name = "Current files".into();
    click(&mut app, &ctx, "Synchronization queue");
    assert!(app.queue_synchronize);
    // The custom switch keeps native checkbox keyboard behavior after a click.
    for expected in [false, true] {
        for pressed in [true, false] {
            frame(
                &mut app,
                &ctx,
                egui::vec2(1080.0, 800.0),
                vec![egui::Event::Key {
                    key: egui::Key::Space,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(app.queue_synchronize, expected);
    }
    click(&mut app, &ctx, "Create queue");
    assert!(app.dialog.is_none());
    let queue = app
        .store
        .queues()
        .unwrap()
        .into_iter()
        .find(|q| q.id == app.queue_id)
        .unwrap();
    assert!(queue.synchronize && !queue.running);
    click(&mut app, &ctx, "Manage queues");
    for size in [egui::vec2(1080.0, 640.0), egui::vec2(1360.0, 900.0)] {
        frame(&mut app, &ctx, size, vec![]);
        let output = frame(&mut app, &ctx, size, vec![]);
        for label in ["Check now", "Pause", "Schedule", "⋯"] {
            let rect = text_rect(&output, label);
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                "{label} is outside the window"
            );
        }
    }
    app.open_queue_dialog(Dialog::NewQueue);
    assert!(!app.queue_synchronize);
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none());
    assert_eq!(app.store.queues().unwrap().len(), queue_count + 1);
}
fn job(status: Status) -> Job {
    let mut job = Job::new(
        "https://example.com/file.zip",
        None,
        std::env::temp_dir(),
        "ui-test".into(),
    )
    .unwrap();
    job.status = status;
    job.downloaded = 100;
    job.total = Some(1000);
    job
}

#[test]
fn retry_labels_show_countdown_and_paused_queue_without_transfer_eta() {
    let mut job = job(Status::Queued);
    assert_eq!(status_label(&job, true, 1), "Queued");
    assert_eq!(status_label(&job, false, 1), "Queue paused");
    job.retry = download_engine::retry::State {
        attempts: 2,
        next_at: Some(5001),
    };
    assert_eq!(status_label(&job, true, 1), "Retry in 5s");
    assert_eq!(status_label(&job, true, 5000), "Retry in 1s");
    assert_eq!(status_label(&job, true, 5001), "Retry queued");
    assert_eq!(status_label(&job, false, 5001), "Retry paused");
    assert_eq!(eta(&job, 100.0), "—");
    assert!(retry_description(&job, true).unwrap().contains("2 of 3"));
    assert!(
        retry_description(&job, false)
            .unwrap()
            .contains("queue to start")
    );
    job.status = Status::Paused;
    assert_eq!(status_label(&job, true, 1), "Paused");
    assert!(retry_description(&job, true).is_none());
}

#[test]
fn status_filters_keep_active_paused_queued_and_failed_states_distinct() {
    for status in [
        Status::Queued,
        Status::Downloading,
        Status::Paused,
        Status::Failed,
        Status::Complete,
    ] {
        assert!(Filter::All.accepts(status));
        assert_eq!(
            Filter::Downloading.accepts(status),
            status == Status::Downloading
        );
        assert_eq!(
            Filter::Unfinished.accepts(status),
            status != Status::Complete
        );
        assert_eq!(Filter::Queued.accepts(status), status == Status::Queued);
        assert_eq!(Filter::Failed.accepts(status), status == Status::Failed);
        assert_eq!(Filter::Complete.accepts(status), status == Status::Complete);
    }
}

#[test]
fn eta_requires_an_active_transfer_known_size_and_measured_rate() {
    let mut job = job(Status::Downloading);
    assert_eq!(eta(&job, 100.0), "9 sec");
    assert_eq!(eta(&job, 1.0), "15 min 0 sec");
    for rate in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(eta(&job, rate), "—");
    }
    job.total = None;
    assert_eq!(eta(&job, 100.0), "—");
    job.total = Some(100);
    assert_eq!(eta(&job, 100.0), "—");
    job.total = Some(1000);
    job.status = Status::Paused;
    assert_eq!(eta(&job, 100.0), "—");
}

#[test]
fn progress_distinguishes_unknown_sizes_and_empty_completed_files() {
    let mut job = job(Status::Downloading);
    assert_eq!(progress(&job), Some(0.1));
    job.total = None;
    assert_eq!(progress(&job), None);
    job.total = Some(0);
    assert_eq!(progress(&job), None);
    job.status = Status::Complete;
    assert_eq!(progress(&job), Some(1.0));
}

#[test]
fn no_resume_plan_keeps_restart_semantics_explicit() {
    let job = job(Status::Paused);
    assert_eq!(resume_action(&job), "Start again");
    assert_eq!(resume_description(&job), "Starts from the beginning");
}
