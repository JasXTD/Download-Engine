use super::*;
use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

#[test]
fn hiding_status_never_changes_the_download() {
    let (_temp, mut app, _ctx) = fixture();
    let job = app.jobs[0].clone();
    app.store.enqueue(&job).unwrap();
    app.open_download_status(&job);
    let mut window = app.download_status.windows.remove(&job.id).unwrap();
    assert!(!app.status_action(&job.id, &mut window, Some(Action::Hide)));
    assert_eq!(app.store.get(&job.id).unwrap().unwrap().status, job.status);
}

#[test]
fn status_layout_shows_real_values_in_both_themes() {
    for dark in [true, false] {
        let (_temp, mut app, ctx) = fixture();
        app.interface.appearance = if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        app.configure_appearance(&ctx);
        let job = app.jobs[0].clone();
        app.store.enqueue(&job).unwrap();
        app.open_download_status(&job);
        for _ in 0..3 {
            frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
        }
        let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
        for label in [
            "Status",
            "Speed limit",
            "When done",
            "25%",
            "Size",
            "Downloaded",
            "Speed",
            "Time left",
            "Hide details",
            "Resume",
            "×",
        ] {
            text_rect(&output, label);
        }
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.job.text.starts_with("Connections · "))),
            "The connections heading is missing"
        );
        for old in [
            "Download status",
            "Speed limiter",
            "Options on completion",
            "Start positions and download progress by connections",
            "<< Hide details",
            "Resume capability",
            "Transfer rate",
        ] {
            assert!(!rendered(&output, old), "Old status-window wording: {old}");
        }
        assert!(
            !rendered(&output, "Cancel"),
            "The header × is the only close control"
        );
    }
}

fn rendered(output: &egui::FullOutput, label: &str) -> bool {
    output.shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.trim() == label)
    })
}

fn draw_window(
    app: &mut Desktop,
    ctx: &egui::Context,
    id: &str,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut window = app.download_status.windows.remove(id).unwrap();
    let current_watch = app.extras.watches.get(&window.queue_id).map(|w| w.action);
    let power_states = app.extras.power_states;
    let mut action = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| {
            let p = Palette::new(ui.visuals().dark_mode);
            egui::CentralPanel::default()
                .frame(form_dialog::frame(p))
                .show(ui, |ui| {
                    action = window.body(ui, p, false, current_watch, power_states)
                });
        },
    );
    output.textures_delta.clear();
    if app.status_action(id, &mut window, action) {
        app.download_status.windows.insert(id.into(), window);
    }
    output
}
fn status_click(app: &mut Desktop, ctx: &egui::Context, id: &str, label: &str) {
    app.refresh_download_status();
    let size = egui::vec2(480.0, 390.0);
    draw_window(app, ctx, id, size, vec![]);
    let output = draw_window(app, ctx, id, size, vec![]);
    // Controls are drawn after the summary, whose labels can share a word ("Resume").
    let pos = output
        .shapes
        .iter()
        .rev()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text.trim() == label => {
                Some(text.galley.rect.translate(text.pos.to_vec2()).center())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("No rendered control: {label}"));
    for pressed in [true, false] {
        draw_window(
            app,
            ctx,
            id,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

#[test]
fn small_status_windows_keep_controls_visible_for_long_names_errors_and_unknown_sizes() {
    for dark in [true, false] {
        let (_temp, mut app, ctx) = fixture();
        app.interface.appearance = if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        app.configure_appearance(&ctx);
        for status in [
            Status::Downloading,
            Status::Queued,
            Status::Paused,
            Status::Failed,
            Status::Complete,
        ] {
            let mut job = app.jobs[0].clone();
            job.name = "Long filename 日本語 ".repeat(100);
            job.url = format!("https://example.com/{}", "path".repeat(300));
            job.status = status;
            job.total = None;
            job.error = (status == Status::Failed).then(|| "A long server error. ".repeat(100));
            app.download_status
                .windows
                .insert(job.id.clone(), Window::new(job.clone()));
            app.download_status
                .windows
                .get_mut(&job.id)
                .unwrap()
                .details = true;
            let size = egui::vec2(400.0, 340.0);
            let output = draw_window(&mut app, &ctx, &job.id, size, vec![]);
            let output = draw_window(&mut app, &ctx, &job.id, size, vec![])
                .shapes
                .into_iter()
                .chain(output.shapes)
                .collect::<Vec<_>>();
            let action_label = match status {
                Status::Complete => "Open folder",
                Status::Downloading => "Pause",
                Status::Queued => "Start immediately",
                _ => resume_action(&job),
            };
            for target_label in [action_label, "×"] {
                let btn = output
                    .iter()
                    .rev()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == target_label => {
                            Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                        }
                        _ => None,
                    })
                    .unwrap();
                assert!(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(btn),
                    "{target_label} clipped for {status:?}: {btn:?}"
                );
            }
        }
    }
}

#[test]
fn status_tracks_its_job_across_queue_changes_and_removal() {
    let (_temp, mut app, _ctx) = fixture();
    let mut job = app.jobs[0].clone();
    app.store.enqueue(&job).unwrap();
    app.open_download_status(&job);
    app.view = View::Queues;
    app.jobs.clear();
    app.selected = None;
    job.downloaded = 2048;
    app.store.save(&job).unwrap();
    app.refresh_download_status();
    assert_eq!(
        app.download_status.windows[&job.id]
            .job
            .as_ref()
            .unwrap()
            .downloaded,
        2048
    );
    app.store.remove(&job.id).unwrap();
    app.refresh_download_status();
    assert!(app.download_status.windows[&job.id].job.is_none());
}

#[test]
fn automatic_opening_ignores_background_jobs_disabled_preferences_and_repeated_refreshes() {
    let (_temp, mut app, _ctx) = fixture();
    let job = app.jobs[1].clone();
    app.store.enqueue(&job).unwrap();
    app.refresh_download_status();
    assert!(app.download_status.windows.is_empty());
    app.interface.auto_download_status = false;
    app.expect_download_status(&job.id);
    app.refresh_download_status();
    assert!(app.download_status.windows.is_empty());
    app.interface.auto_download_status = true;
    app.expect_download_status(&job.id);
    app.refresh_download_status();
    assert_eq!(app.download_status.windows.len(), 1);
    app.download_status.windows.clear();
    app.refresh_download_status();
    assert!(app.download_status.windows.is_empty());
}

#[test]
fn manual_open_is_idempotent_and_paused_queue_additions_do_not_open() {
    let (_temp, mut app, _ctx) = fixture();
    let job = app.jobs[0].clone();
    app.store.enqueue(&job).unwrap();
    app.store.set_queue_running(MAIN_QUEUE, false).unwrap();
    app.expect_added_status(&job.id);
    assert!(app.download_status.pending.is_empty());
    app.open_download_status(&job);
    app.open_download_status(&job);
    assert_eq!(app.download_status.windows.len(), 1);
    app.expect_download_status(&job.id);
    app.command(Command::Pause(job.id.clone()));
    assert!(app.download_status.pending.is_empty());
}

#[test]
fn keyboard_and_download_menu_open_the_same_status_window() {
    use crate::desktop::interaction_tests::click;
    let (_temp, mut app, ctx) = fixture();
    // Choose a different row so its name is not also in the selected inspector.
    let job = app.jobs[1].clone();
    app.store.enqueue(&job).unwrap();
    click(&mut app, &ctx, &job.name);
    assert_eq!(app.selected.as_deref(), Some(job.id.as_str()));
    assert!(
        ctx.memory(|m| m.has_focus(egui::Id::new("download-list"))),
        "Focus after filename click: {:?}",
        ctx.memory(|m| m.focused())
    );
    frame(
        &mut app,
        &ctx,
        egui::vec2(1080.0, 800.0),
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(app.download_status.windows.len(), 1);
    app.download_status.windows.clear();
    click(&mut app, &ctx, "File actions");
    click(&mut app, &ctx, "Download status");
    assert_eq!(app.download_status.windows.len(), 1);
    assert!(app.download_status.windows.contains_key(&job.id));
}

#[test]
fn unknown_size_and_finalization_never_claim_a_finished_download() {
    let (_temp, mut app, ctx) = fixture();
    let mut job = app.jobs[0].clone();
    job.status = Status::Downloading;
    job.total = None;
    app.download_status
        .windows
        .insert(job.id.clone(), Window::new(job.clone()));
    let output = draw_window(&mut app, &ctx, &job.id, egui::vec2(480.0, 390.0), vec![]);
    text_rect(&output, "Size unknown");
    text_rect(&output, "Pause");
    let window = app.download_status.windows.get_mut(&job.id).unwrap();
    window.job.as_mut().unwrap().total = Some(job.downloaded);
    window.phase = Some(Phase::Merging);
    let output = draw_window(&mut app, &ctx, &job.id, egui::vec2(480.0, 390.0), vec![]);
    text_rect(&output, "100%");
    text_rect(&output, "Merging video");
    text_rect(&output, "Pause");
    assert!(!output.shapes.iter().any(
        |shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="Completed")
    ));
}

#[test]
fn unreadable_status_is_reported_and_does_not_replace_its_last_snapshot() {
    let (_temp, mut app, _ctx) = fixture();
    let job = app.jobs[0].clone();
    app.store.enqueue(&job).unwrap();
    app.open_download_status(&job);
    let database = rusqlite::Connection::open(app.store.paths.data.join("queue.sqlite3")).unwrap();
    database
        .execute(
            "UPDATE jobs SET data=json_set(data,'$.total','broken') WHERE id=?1",
            [&job.id],
        )
        .unwrap();
    app.refresh_download_status();
    let window = &app.download_status.windows[&job.id];
    assert!(window.error.as_ref().unwrap().contains("Cannot refresh"));
    assert_eq!(window.job.as_ref().unwrap().id, job.id);
    assert_eq!(window.rate, 0.0);
}

struct Server {
    url: String,
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
    ranges: Arc<Mutex<Vec<u64>>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        Self::with_release(true)
    }

    fn with_release(released: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let released = Arc::new(AtomicBool::new(released));
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let (flag, seen) = (stop.clone(), ranges.clone());
        let release = released.clone();
        let thread = thread::spawn(move || {
            let mut workers = Vec::new();
            for stream in listener.incoming() {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                let (flag, seen) = (flag.clone(), seen.clone());
                let release = release.clone();
                workers.push(thread::spawn(move || {
                    let mut stream = stream.unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    stream.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut request = Vec::new();
                    let mut byte = [0];
                    while request.len() < 16384 && !request.ends_with(b"\r\n\r\n") {
                        if stream.read_exact(&mut byte).is_err() { return; }
                        request.push(byte[0]);
                    }
                    let text = String::from_utf8(request).unwrap();
                    if text.starts_with("GET /failed") { let _=stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"); return; }
                    const SIZE: u64 = 4*1024*1024;
                    let range = text.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_owned));
                    let (start,end) = range.as_ref().and_then(|r| r.split_once('-')).map(|(a,b)| (a.parse::<u64>().unwrap(),b.parse::<u64>().unwrap_or(SIZE-1))).unwrap_or((0,SIZE-1));
                    seen.lock().unwrap().push(start);
                    let status = if range.is_some() { "206 Partial Content" } else { "200 OK" };
                    let header = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{SIZE}\r\nETag: \"status-fixture\"\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n", end-start+1);
                    if stream.write_all(header.as_bytes()).is_err() { return; }
                    let chunk = vec![42;16*1024];
                    let mut left = end-start+1;
                    while left > 0 && !flag.load(Ordering::Relaxed) {
                        // Keep a real partial transfer alive until the UI assertions
                        // finish, independent of render speed or machine load.
                        if !release.load(Ordering::Relaxed) && end-start+1-left >= 64*1024 {
                            thread::sleep(Duration::from_millis(8));
                            continue;
                        }
                        let n=left.min(chunk.len() as u64) as usize;
                        if stream.write_all(&chunk[..n]).is_err() { break; }
                        left-=n as u64;
                        thread::sleep(Duration::from_millis(8));
                    }
                }));
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            url: format!("http://{address}/file.bin"),
            address,
            stop,
            released,
            ranges,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn until(app: &mut Desktop, condition: impl Fn(&Desktop) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.last_refresh = Instant::now() - Duration::from_millis(501);
        app.refresh();
        if condition(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out: {:?}; jobs: {:?}",
            app.notice.as_ref().map(|n| &n.message),
            app.store
                .list()
                .unwrap()
                .iter()
                .map(|j| (&j.name, j.status, j.downloaded))
                .collect::<Vec<_>>()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn separate_status_input(ctx: &egui::Context) {
    // A native status window cannot intercept input aimed at the main window.
    // Render its real UI on a separate surface in this headless backend too.
    ctx.set_embed_viewports(false);
    egui::Context::set_immediate_viewport_renderer(|_, mut viewport| {
        let child = egui::Context::default();
        let mut output = child.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(480.0, 390.0),
                )),
                ..Default::default()
            },
            |ui| (viewport.viewport_ui_cb)(ui),
        );
        output.textures_delta.clear();
    });
}

#[test]
fn toolbar_and_tasks_stop_all_stop_other_queues_and_immediate_transfers_without_losing_bytes() {
    for tasks_menu in [false, true] {
        let server = Server::with_release(false);
        let (_temp, mut app, ctx) = fixture();
        separate_status_input(&ctx);
        app.store.set_queue_limit(MAIN_QUEUE, 1).unwrap();
        let other = app.store.create_queue("Other downloads").unwrap();
        app.url = server.url.clone();
        app.add();
        let first = app.selected.clone().unwrap();
        let waiting = Job::new(
            &server.url,
            Some("waiting.bin"),
            app.store.paths.downloads.clone(),
            Uuid::new_v4().to_string(),
        )
        .unwrap();
        app.store.enqueue(&waiting).unwrap();
        let immediate = Job::new(
            &server.url,
            Some("immediate.bin"),
            app.store.paths.downloads.clone(),
            Uuid::new_v4().to_string(),
        )
        .unwrap();
        app.store
            .enqueue_batch(std::slice::from_ref(&immediate), &other.id)
            .unwrap();
        app.command(Command::StartImmediately(immediate.id.clone()));
        app.expect_download_status(&immediate.id);
        until(&mut app, |app| {
            [&first, &immediate.id].iter().all(|id| {
                app.store
                    .get(id)
                    .unwrap()
                    .is_some_and(|j| j.status == Status::Downloading && j.downloaded > 0)
            })
        });
        let saved = app.store.get(&first).unwrap().unwrap().downloaded;
        assert_eq!(
            app.store.get(&waiting.id).unwrap().unwrap().status,
            Status::Queued
        );
        app.open_download_status(&waiting);
        app.expect_download_status(&waiting.id);
        let mut complete = waiting.clone();
        complete.id = Uuid::new_v4().to_string();
        complete.request_id = Uuid::new_v4().to_string();
        complete.status = Status::Complete;
        app.store.enqueue(&complete).unwrap();
        app.open_download_status(&complete);
        let queues = app.store.queues().unwrap();
        // Stop all must work while a different queue and a filter hide the running files.
        app.queue_id = other.id.clone();
        app.filter = Filter::Complete;
        app.force_refresh();
        if tasks_menu {
            click(&mut app, &ctx, "Tasks");
        }
        click(&mut app, &ctx, "Stop all");
        until(&mut app, |app| {
            [&first, &waiting.id, &immediate.id]
                .iter()
                .all(|id| app.store.get(id).unwrap().unwrap().status == Status::Paused)
        });
        assert!(app.store.immediate_downloads().unwrap().is_empty());
        assert!(app.download_status.pending.is_empty());
        assert_eq!(
            app.download_status.windows.keys().collect::<Vec<_>>(),
            vec![&complete.id]
        );
        assert!(app.store.get(&first).unwrap().unwrap().downloaded >= saved);
        assert_eq!(app.store.queues().unwrap(), queues);
        // Starting a queue must not undo individual Stop ownership.
        app.command(Command::StartQueue(other.id.clone()));
        until(&mut app, |app| {
            app.store
                .queues()
                .unwrap()
                .iter()
                .find(|q| q.id == other.id)
                .unwrap()
                .running
        });
        assert_eq!(
            app.store.get(&immediate.id).unwrap().unwrap().status,
            Status::Paused
        );
        let original_ranges = server.ranges.lock().unwrap().clone();
        app.command(Command::Resume(first.clone()));
        server.released.store(true, Ordering::Relaxed);
        until(&mut app, |app| {
            app.store.get(&first).unwrap().unwrap().status == Status::Complete
        });
        let finished = app.store.get(&first).unwrap().unwrap();
        assert_eq!(
            std::fs::read(finished.saved_path.unwrap()).unwrap(),
            vec![42; 4 * 1024 * 1024]
        );
        assert_eq!(app.store.list().unwrap().len(), 4);
        assert!(
            server
                .ranges
                .lock()
                .unwrap()
                .iter()
                .skip(original_ranges.len())
                .any(|start| *start > 0 && !original_ranges.contains(start))
        );
        assert_eq!(
            app.store.get(&waiting.id).unwrap().unwrap().status,
            Status::Paused
        );
        assert_eq!(
            app.store.get(&immediate.id).unwrap().unwrap().status,
            Status::Paused
        );
    }
}

#[test]
fn stop_selected_controls_keep_other_files_and_close_only_selected_status_windows() {
    for entry in ["toolbar", "tasks", "actions", "context", "multiple"] {
        let (_temp, mut app, ctx) = fixture();
        separate_status_input(&ctx);
        let now = chrono::Utc::now().timestamp();
        app.store
            .schedule_queue(MAIN_QUEUE, now + 3600, now)
            .unwrap();
        let mut jobs = app.jobs.clone();
        jobs[0].status = Status::Queued;
        jobs[2].status = Status::Queued;
        app.store.enqueue_batch(&jobs, MAIN_QUEUE).unwrap();
        app.force_refresh();
        app.set_selection(Some(jobs[0].id.clone()));
        app.interface.show_details = false;
        if entry == "multiple" {
            app.selection.additional.insert(jobs[1].id.clone());
            app.selection.additional.insert(jobs[2].id.clone());
        }
        for job in &jobs {
            app.open_download_status(job);
        }
        app.expect_download_status(&jobs[0].id);
        if matches!(entry, "actions" | "multiple") {
            click(&mut app, &ctx, "⋮");
        }
        if entry == "tasks" {
            click(&mut app, &ctx, "Tasks");
        }
        if entry == "context" {
            let size = egui::vec2(1080.0, 800.0);
            let output = frame(&mut app, &ctx, size, vec![]);
            let pos = text_rect(&output, &jobs[0].name).center();
            for pressed in [true, false] {
                frame(
                    &mut app,
                    &ctx,
                    size,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        }
        click(
            &mut app,
            &ctx,
            if matches!(entry, "tasks" | "multiple") {
                "Stop selected"
            } else {
                "Stop"
            },
        );
        until(&mut app, |app| {
            app.store.get(&jobs[0].id).unwrap().unwrap().status == Status::Paused
                && (entry != "multiple"
                    || app.store.get(&jobs[2].id).unwrap().unwrap().status == Status::Paused)
        });
        assert!(
            !app.download_status.windows.contains_key(&jobs[0].id),
            "{entry}"
        );
        assert!(!app.download_status.pending.contains(&jobs[0].id));
        assert!(app.download_status.windows.contains_key(&jobs[1].id));
        assert_eq!(
            app.download_status.windows.contains_key(&jobs[2].id),
            entry != "multiple"
        );
        assert_eq!(
            app.store.get(&jobs[0].id).unwrap().unwrap().downloaded,
            jobs[0].downloaded
        );
        assert_eq!(
            app.store.get(&jobs[1].id).unwrap().unwrap().status,
            Status::Complete
        );
        assert_eq!(
            app.store.get(&jobs[2].id).unwrap().unwrap().status,
            if entry == "multiple" {
                Status::Paused
            } else {
                Status::Queued
            }
        );
        assert_eq!(app.store.list().unwrap().len(), 3);
    }
}

#[test]
fn live_status_controls_pause_resume_close_and_hide_without_duplicate_or_lost_progress() {
    let server = Server::with_release(false);
    let (_temp, mut app, ctx) = fixture();
    app.url = server.url.clone();
    app.add();
    let id = app.selected.clone().unwrap();
    until(&mut app, |app| {
        app.store
            .get(&id)
            .unwrap()
            .is_some_and(|j| j.downloaded > 0 && j.status == Status::Downloading)
    });
    assert!(app.download_status.windows.contains_key(&id));
    assert_eq!(app.store.activity.get(&id), Some(Phase::Downloading));
    status_click(&mut app, &ctx, &id, "Pause");
    until(&mut app, |app| {
        app.store.get(&id).unwrap().unwrap().status == Status::Paused
            && app.store.activity.get(&id).is_none()
    });
    let saved = app.store.get(&id).unwrap().unwrap().downloaded;
    assert!(saved > 0 && saved < 4 * 1024 * 1024);
    assert_eq!(app.store.activity.get(&id), None);
    status_click(&mut app, &ctx, &id, "Resume");
    until(&mut app, |app| {
        app.store.get(&id).unwrap().unwrap().downloaded > saved
    });
    status_click(&mut app, &ctx, &id, "Pause");
    until(&mut app, |app| {
        app.store.get(&id).unwrap().unwrap().status == Status::Paused
    });
    status_click(&mut app, &ctx, &id, "×");
    assert!(!app.download_status.windows.contains_key(&id));
    let job = app.store.get(&id).unwrap().unwrap();
    assert_eq!(job.status, Status::Paused);
    assert!(job.downloaded >= saved);
    app.open_download_status(&job);
    status_click(&mut app, &ctx, &id, "Resume");
    until(&mut app, |app| {
        app.store.get(&id).unwrap().unwrap().status == Status::Downloading
    });
    status_click(&mut app, &ctx, &id, "×");
    server.released.store(true, Ordering::Relaxed);
    until(&mut app, |app| {
        app.store.get(&id).unwrap().unwrap().status == Status::Complete
            && app.store.activity.get(&id).is_none()
    });
    assert!(app.download_status.windows.is_empty());
    let job = app.store.get(&id).unwrap().unwrap();
    assert_eq!(app.store.list().unwrap().len(), 1);
    assert_eq!(
        std::fs::read(job.saved_path.as_ref().unwrap()).unwrap(),
        vec![42; 4 * 1024 * 1024]
    );
    assert!(
        server
            .ranges
            .lock()
            .unwrap()
            .iter()
            .any(|start| *start >= saved)
    );
    assert_eq!(app.store.activity.get(&id), None);
    app.open_download_status(&job);
    let output = draw_window(&mut app, &ctx, &id, egui::vec2(480.0, 390.0), vec![]);
    text_rect(&output, "Download complete");
    text_rect(&output, "Open");
    text_rect(&output, "Source");
    text_rect(&output, "Saved to");
    assert!(!rendered(&output, "Download complete · Completed") && !rendered(&output, "100%"));
    text_rect(&output, "Open with…");
    text_rect(&output, "Open folder");
    text_rect(&output, "×");
    assert!(!rendered(&output, "Close"));
}

#[test]
fn failed_download_has_actionable_status_and_no_stale_activity() {
    let server = Server::new();
    let (_temp, mut app, ctx) = fixture();
    app.url = server.url.replace("file.bin", "failed");
    app.add();
    let id = app.selected.clone().unwrap();
    until(&mut app, |app| {
        app.store.get(&id).unwrap().unwrap().status == Status::Failed
    });
    assert!(
        app.download_status.windows[&id]
            .job
            .as_ref()
            .unwrap()
            .error
            .is_some()
    );
    assert_eq!(app.store.activity.get(&id), None);
    let output = draw_window(&mut app, &ctx, &id, egui::vec2(480.0, 390.0), vec![]);
    text_rect(&output, "Needs attention");
    text_rect(&output, "Start again");
}

#[test]
fn ui_stop_and_external_commands_share_ownership_without_losing_saved_bytes() {
    use download_engine::control::Action as ControlAction;
    let server = Server::with_release(false);
    let (_temp, mut app, ctx) = fixture();
    app.url = server.url.clone();
    app.add();
    let id = app.selected.clone().unwrap();
    until(&mut app, |app| {
        let job = app.store.get(&id).unwrap().unwrap();
        job.downloaded > 0 && job.status == Status::Downloading
    });
    let mut previous = 0;
    for _ in 0..4 {
        let store = app.store.clone();
        let other_id = id.clone();
        let requests = thread::spawn(move || {
            let mut ids = Vec::new();
            for action in [
                ControlAction::Pause {
                    id: other_id.clone(),
                },
                ControlAction::Resume {
                    id: other_id.clone(),
                },
                ControlAction::Pause { id: other_id },
            ] {
                let request = uuid::Uuid::new_v4().to_string();
                store.submit_control(&request, &action).unwrap();
                ids.push(request);
            }
            ids
        });
        // A status-window Pause requests a stop even while an external Pause
        // is settling; closing the window afterwards keeps the download state.
        app.refresh_download_status();
        let mut window = app.download_status.windows.remove(&id).unwrap();
        assert!(app.status_action(&id, &mut window, Some(Action::Pause)));
        assert!(!app.status_action(&id, &mut window, Some(Action::Hide)));
        assert!(!app.download_status.windows.contains_key(&id));
        let requests = requests.join().unwrap();
        until(&mut app, |app| {
            requests.iter().all(|request| {
                !matches!(
                    app.store.control_receipt(request).unwrap().state.as_str(),
                    "pending" | "running"
                )
            })
        });
        for request in requests {
            let receipt = app.store.control_receipt(&request).unwrap();
            if receipt.state == "failed" {
                assert!(
                    receipt
                        .error
                        .unwrap()
                        .contains("Pause the download before changing it")
                );
            } else {
                assert_eq!(receipt.state, "done");
            }
        }
        app.scheduler.send(Command::Pause(id.clone())).unwrap();
        until(&mut app, |app| {
            app.store.get(&id).unwrap().unwrap().status == Status::Paused
                && app.store.activity.get(&id).is_none()
        });
        let job = app.store.get(&id).unwrap().unwrap();
        assert!(job.downloaded >= previous);
        previous = job.downloaded;
        assert_eq!(app.store.list().unwrap().len(), 1);
        app.open_download_status(&job);
        status_click(&mut app, &ctx, &id, "Resume");
        until(&mut app, |app| {
            app.store.get(&id).unwrap().unwrap().status == Status::Downloading
        });
    }
    server.released.store(true, Ordering::Relaxed);
    until(&mut app, |app| {
        app.store.get(&id).unwrap().unwrap().status == Status::Complete
            && app.store.activity.get(&id).is_none()
    });
    let job = app.store.get(&id).unwrap().unwrap();
    assert_eq!(
        std::fs::read(job.saved_path.as_ref().unwrap()).unwrap(),
        vec![42; 4 * 1024 * 1024]
    );
    assert_eq!(app.store.list().unwrap().len(), 1);
}

#[test]
fn status_tabs_and_multi_segment_connections_work() {
    let (_temp, mut app, ctx) = fixture();
    let mut job = app.jobs[0].clone();
    job.status = Status::Downloading;
    job.total = Some(2 * 1024 * 1024);
    job.downloaded = 768 * 1024;
    job.plan = Some(download_engine::model::Plan {
        size: 2 * 1024 * 1024,
        etag: "\"etag\"".into(),
        final_url: job.url.clone(),
        segments: vec![
            download_engine::model::Segment {
                start: 0,
                end: 1024 * 1024 - 1,
            },
            download_engine::model::Segment {
                start: 1024 * 1024,
                end: 2 * 1024 * 1024 - 1,
            },
        ],
    });
    app.store.enqueue(&job).unwrap();
    app.store.activity.seed_preview(
        &job.id,
        Phase::Downloading,
        vec![
            SegmentSnapshot {
                index: 0,
                start: 0,
                end: 1024 * 1024 - 1,
                downloaded: 512 * 1024,
                state: SegmentState::ReceivingData,
            },
            SegmentSnapshot {
                index: 1,
                start: 1024 * 1024,
                end: 2 * 1024 * 1024 - 1,
                downloaded: 256 * 1024,
                state: SegmentState::SendingGet,
            },
        ],
    );
    app.open_download_status(&job);
    let size = egui::vec2(540.0, 440.0);
    draw_window(&mut app, &ctx, &job.id, size, vec![]);
    let output = draw_window(&mut app, &ctx, &job.id, size, vec![]);
    text_rect(&output, "Downloading");
    text_rect(&output, "Connecting");
    text_rect(&output, "#");
    assert!(!rendered(&output, "Send GET…") && !rendered(&output, "Receiving data…"));

    status_click(&mut app, &ctx, &job.id, "Speed limit");
    let output = draw_window(&mut app, &ctx, &job.id, size, vec![]);
    text_rect(&output, "Limit speed for this download");
    text_rect(&output, "Apply speed limits");
    status_click(&mut app, &ctx, &job.id, "1 MiB/s");
    status_click(&mut app, &ctx, &job.id, "Apply speed limits");
    until(&mut app, |app| {
        app.limits.downloads.get(&job.id) == Some(&(1024 * 1024))
    });

    status_click(&mut app, &ctx, &job.id, "When done");
    let output = draw_window(&mut app, &ctx, &job.id, size, vec![]);
    text_rect(&output, "Close this status window automatically");
    text_rect(&output, "Open file when download finishes");
    text_rect(&output, "Exit");
    status_click(&mut app, &ctx, &job.id, "Exit");
    assert_eq!(
        app.extras.watches.get(MAIN_QUEUE).map(|w| w.action),
        Some(completion::Action::Exit)
    );
}

#[test]
fn when_done_offers_sleep_and_explains_why_hibernate_is_unavailable() {
    let (_temp, mut app, ctx) = fixture();
    app.extras.power_states = || PowerStates {
        sleep: true,
        hibernate: false,
    };
    let job = app.store.enqueue(&app.jobs[0]).unwrap();
    app.open_download_status(&job);
    status_click(&mut app, &ctx, &job.id, "When done");
    // Tall enough to show every choice without scrolling.
    let size = egui::vec2(540.0, 640.0);
    let click = |app: &mut Desktop, label: &str| {
        let output = draw_window(app, &ctx, &job.id, size, vec![]);
        let pos = text_rect(&output, label).center();
        for pressed in [true, false] {
            draw_window(
                app,
                &ctx,
                &job.id,
                size,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    };
    draw_window(&mut app, &ctx, &job.id, size, vec![]);
    let output = draw_window(&mut app, &ctx, &job.id, size, vec![]);
    text_rect(&output, "Shut down computer");
    text_rect(
        &output,
        "Hibernate is off or not supported on this computer. An administrator can turn it on with powercfg /hibernate on.",
    );
    assert!(!rendered(
        &output,
        "Sleep is unavailable: Windows doesn't let apps put this computer to sleep, for example on Modern Standby PCs."
    ));
    click(&mut app, "Hibernate computer");
    assert!(
        app.extras.watches.is_empty(),
        "A disabled choice is never armed"
    );
    click(&mut app, "Sleep computer");
    assert_eq!(
        app.extras.watches.get(MAIN_QUEUE).map(|w| w.action),
        Some(completion::Action::Sleep)
    );
    click(&mut app, "Do nothing");
    assert!(app.extras.watches.is_empty());
}

#[test]
fn video_status_shows_media_parts_instead_of_a_single_file_range() {
    for dark in [true, false] {
        let (_temp, mut app, ctx) = fixture();
        app.interface.appearance = if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        app.configure_appearance(&ctx);
        let mut job = app.jobs[0].clone();
        job.status = Status::Paused;
        job.plan = None;
        job.total = None;
        let part = |n| download_engine::hls::Part {
            url: format!("https://example.com/{n}.ts"),
            range: None,
            initialization: false,
        };
        let mut download = download_engine::hls::Download::new(download_engine::hls::Playlist {
            public_only: false,
            source: "https://example.com/master.m3u8".into(),
            url: "https://example.com/video.m3u8".into(),
            quality: "720p".into(),
            bandwidth: None,
            duration_ms: 18_000,
            sequence: 0,
            format: download_engine::hls::Format::TransportStream,
            parts: (0..3).map(part).collect(),
            audio: Some(download_engine::hls::AudioTrack {
                name: "Audio".into(),
                language: None,
                url: "https://example.com/audio.m3u8".into(),
                duration_ms: 18_000,
                sequence: 0,
                format: download_engine::hls::Format::TransportStream,
                parts: (3..5).map(part).collect(),
            }),
            dash: None,
        });
        download
            .completed
            .push(download_engine::hls::CompletedPart {
                size: 18800,
                sha256: "0".repeat(64),
            });
        job.downloaded = 18800;
        job.url = download.playlist.source.clone();
        job.hls = Some(download);
        app.store.enqueue(&job).unwrap();
        app.open_download_status(&job);
        let size = egui::vec2(540.0, 495.0);
        draw_window(&mut app, &ctx, &job.id, size, vec![]);
        let output = draw_window(&mut app, &ctx, &job.id, size, vec![]);
        text_rect(&output, "Video segments (1 / 5 complete)");
        text_rect(&output, "Video · Completed");
        text_rect(&output, "Audio · Waiting");
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        assert!(viewport.contains_rect(text_rect(&output, "Video segments (1 / 5 complete)")));
        let window = app.download_status.windows.get_mut(&job.id).unwrap();
        window.job.as_mut().unwrap().status = Status::Downloading;
        window.live_media = Some(download_engine::transfer_activity::MediaSnapshot {
            index: 1,
            downloaded: 262_144,
            total: Some(1_048_576),
            state: SegmentState::ReceivingData,
        });
        let output = draw_window(&mut app, &ctx, &job.id, size, vec![]);
        text_rect(&output, "Video · Downloading");
        text_rect(&output, "1 connection · Segment 2 / 5 · 256.0 KB");
        let window = app.download_status.windows.get_mut(&job.id).unwrap();
        window.live_media.as_mut().unwrap().total = None;
        let current = window.job.as_ref().unwrap();
        assert_eq!(
            window.media_part(current, current.hls.as_ref().unwrap(), 1),
            (262_144, None, "Downloading")
        );
        let output = draw_window(&mut app, &ctx, &job.id, size, vec![]);
        text_rect(&output, "1 connection · Segment 2 / 5 · 256.0 KB");
        // Long playlists must not create thousands of painted rows or dividers.
        let window = app.download_status.windows.get_mut(&job.id).unwrap();
        let download = window.job.as_mut().unwrap().hls.as_mut().unwrap();
        download
            .playlist
            .parts
            .resize(10_000 - 2, download.playlist.parts[0].clone());
        let output = draw_window(&mut app, &ctx, &job.id, size, vec![]);
        assert!(
            output.shapes.len() < 1000,
            "Media details must virtualize long playlists"
        );
    }
}
