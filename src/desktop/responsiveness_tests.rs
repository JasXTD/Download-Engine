use super::interaction_tests::fixture;

impl Desktop {
    /// Existing outcome tests wait for the actual workers. Latency tests below
    /// deliberately use the nonblocking production entry points instead.
    pub(super) fn wait_for_io(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        let ctx = self.background_io.context.clone().unwrap_or_default();
        loop {
            let refresh = self.background_io.pending();
            self.poll_background_io(&ctx);
            if refresh {
                self.refresh_snapshot();
            }
            if self.hls_requests_pending() {
                self.poll_hls_requests(&ctx);
            }
            if self.browser_requests_pending() {
                self.poll_browser_video_requests(&ctx);
            }
            let pending = self.background_io.pending()
                || self.status_refresh_pending()
                || self.add_request.is_some()
                || self.hls.as_ref().is_some_and(hls::State::saving)
                || self
                    .browser_video
                    .as_ref()
                    .is_some_and(browser_video::State::saving)
                || self.hls_requests_pending()
                || self.browser_requests_pending();
            let pending = pending || self.antivirus_report_pending();
            if !pending {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Background UI work did not complete"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub(super) fn refresh_and_wait(&mut self) {
        self.force_refresh();
        self.wait_for_io();
    }

    pub(super) fn refresh_status_and_wait(&mut self) {
        self.refresh_download_status();
        self.wait_for_io();
    }

    pub(super) fn open_status_and_wait(&mut self, job: &Job) {
        self.open_download_status(job);
        self.refresh_status_and_wait();
    }
}
use super::*;

#[test]
fn refresh_returns_while_database_writer_is_busy() {
    let (_temp, mut app, _ctx) = fixture();
    let job = app.jobs[0].clone();
    app.store.enqueue(&job).unwrap();
    app.store.request_download_status(&job.id).unwrap();
    let database = app.store.paths.data.join("queue.sqlite3");
    let (ready, acquired) = std::sync::mpsc::channel();
    let lock = std::thread::spawn(move || {
        let connection = crate::desktop::fixture_database::open(database).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        ready.send(()).unwrap();
        std::thread::sleep(Duration::from_secs(2));
        connection.execute_batch("ROLLBACK").unwrap();
    });
    acquired.recv_timeout(Duration::from_secs(5)).unwrap();
    let started = Instant::now();
    app.force_refresh();
    let elapsed = started.elapsed();
    lock.join().unwrap();
    eprintln!("refresh while SQLite is busy: {elapsed:?}");
    app.wait_for_io();
    assert!(
        elapsed < Duration::from_millis(500),
        "UI refresh blocked for {elapsed:?}"
    );
}

fn raw_click(app: &mut Desktop, ctx: &egui::Context, position: egui::Pos2) -> Duration {
    let started = Instant::now();
    for pressed in [true, false] {
        app.refresh();
        interaction_tests::frame(
            app,
            ctx,
            egui::vec2(1080.0, 800.0),
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    started.elapsed()
}

#[test]
fn video_click_is_acknowledged_once_while_a_two_second_write_is_pending() {
    let (_temp, mut app, ctx) = fixture();
    let now = chrono::Utc::now().timestamp();
    app.store
        .schedule_queue(MAIN_QUEUE, now + 3600, now)
        .unwrap();
    app.refresh_and_wait();
    app.hls_sample(true);
    let size = egui::vec2(1080.0, 800.0);
    interaction_tests::frame(&mut app, &ctx, size, vec![]);
    let output = interaction_tests::frame(&mut app, &ctx, size, vec![]);
    let button = interaction_tests::text_rect(&output, "Add video to queue").center();
    let database =
        crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3")).unwrap();
    database.execute_batch("BEGIN IMMEDIATE").unwrap();
    let clicked = raw_click(&mut app, &ctx, button);
    assert!(
        clicked < Duration::from_millis(500),
        "Video click blocked for {clicked:?}"
    );
    assert!(app.hls.as_ref().unwrap().saving());
    let output = interaction_tests::frame(&mut app, &ctx, size, vec![]);
    interaction_tests::text_rect(&output, "Adding…");
    let busy_until = Instant::now() + Duration::from_secs(2);
    let mut slowest = Duration::ZERO;
    while Instant::now() < busy_until {
        slowest = slowest.max(raw_click(&mut app, &ctx, button));
        assert!(app.hls.as_ref().unwrap().saving());
        std::thread::sleep(Duration::from_millis(16));
    }
    assert!(
        slowest < Duration::from_millis(500),
        "An input frame blocked for {slowest:?}"
    );
    // Escape cannot abandon a transaction that is already being committed.
    interaction_tests::frame(
        &mut app,
        &ctx,
        size,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(app.dialog, Some(Dialog::Hls));
    assert!(app.store.list().unwrap().is_empty());
    database.execute_batch("ROLLBACK").unwrap();
    app.wait_for_io();
    assert_eq!(app.dialog, None);
    let jobs = app.store.list().unwrap();
    assert_eq!(jobs.len(), 1, "Repeated clicks created duplicate downloads");
    assert!(jobs[0].hls.is_some());
    assert_eq!(jobs[0].status, Status::Queued);
    eprintln!(
        "video click: {clicked:?}; slowest press/release pair during 2s contention: {slowest:?}"
    );
}

#[test]
fn failed_async_add_keeps_the_form_and_retry_saves_exactly_once() {
    let (_temp, mut app, _ctx) = fixture();
    let now = chrono::Utc::now().timestamp();
    app.store
        .schedule_queue(MAIN_QUEUE, now + 3600, now)
        .unwrap();
    app.open_add();
    app.wait_for_io();
    app.url = "https://example.invalid/retry.zip".into();
    app.name = "Keep this name.zip".into();
    app.add_queue = Some("removed-queue".into());
    app.add();
    assert!(app.add_request.is_some());
    app.add();
    app.wait_for_io();
    assert_eq!(app.dialog, Some(Dialog::Add));
    assert_eq!(app.url, "https://example.invalid/retry.zip");
    assert_eq!(app.name, "Keep this name.zip");
    assert!(
        app.form_error
            .as_ref()
            .unwrap()
            .contains("Queue no longer exists")
    );
    assert!(app.store.list().unwrap().is_empty());
    app.add_queue = Some(MAIN_QUEUE.into());
    app.add();
    app.add();
    app.wait_for_io();
    assert!(app.dialog.is_none());
    assert_eq!(app.store.list().unwrap().len(), 1);
}

#[test]
fn start_immediately_dispatches_without_waiting_for_database() {
    let (_temp, mut app, _ctx) = fixture();
    // No network is needed: the database lock holds the scheduler until after
    // the click latency and its immediate feedback have both been checked.
    let mut job = app.jobs[0].clone();
    job.url = "http://127.0.0.1:1/no-network-fixture.zip".into();
    app.store.enqueue(&job).unwrap();
    app.refresh_and_wait();
    let database =
        crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3")).unwrap();
    database.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    app.command(Command::StartImmediately(job.id));
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "Start click blocked for {elapsed:?}"
    );
    assert!(app.notice.as_ref().unwrap().contains("Start requested."));
    // A fast read must not erase feedback while the scheduler is still waiting
    // for this writer. The snapshot can finish before the first post-click paint.
    app.wait_for_io();
    assert_eq!(
        app.notice.as_ref().map(|notice| notice.message.as_str()),
        Some(tr("Start requested.")),
        "Refreshing the previous queue state erased the start acknowledgment"
    );
    database.execute_batch("ROLLBACK").unwrap();
    app.wait_for_io();
    eprintln!("Start immediately dispatch while SQLite is busy: {elapsed:?}");
}

#[test]
fn idle_background_keeps_processing_while_browser_storage_is_busy() {
    let (_temp, mut app, ctx) = fixture();
    app.interface.clipboard_links = false;
    app.interface.drop_target = false;
    app.selected = None;
    let database =
        crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3")).unwrap();
    database.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    app.background(&ctx);
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "Full background frame blocked for {elapsed:?}"
    );
    database.execute_batch("ROLLBACK").unwrap();
    app.wait_for_io();
    eprintln!("full idle background with SQLite writer busy: {elapsed:?}");
}
