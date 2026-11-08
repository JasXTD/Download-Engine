//! The torrent section of the status window: what it shows, and that its controls only ask
//! the scheduler. No test here starts the torrent helper or reaches a network.
use super::torrent::TorrentTab;
use super::*;
use crate::desktop::interaction_tests::{fixture, text_rect};
use download_engine::torrent::{
    Details, File, Peer, Pieces, RECHECK_LATER, Settings, Source, SourceState, TorrentDetails,
    Tracker, TrackerState,
};
use download_engine::transfer_activity::TorrentSnapshot;
use std::sync::Arc;

const HASH: &str = "0123456789012345678901234567890123456789";
const TRACKER: &str = "udp://tracker.example.org:1337/announce";
const ADDED: &str = "udp://open.example.org:6969/announce";
const SIZE: egui::Vec2 = egui::vec2(702.0, 640.0);

#[test]
fn peer_connection_labels_follow_negotiation_and_keep_unknown_distinct() {
    use de_torrent::details::{ConnectionInfo, PeerTransport};
    let _language = download_engine::i18n::scope(download_engine::i18n::Language::English);
    let mut peer = details(false).peers.remove(0);
    for (transport, encrypted, expected) in [
        (PeerTransport::Tcp, Some(true), "TCP · Encrypted"),
        (PeerTransport::Tcp, Some(false), "TCP · Plaintext"),
        (PeerTransport::Utp, Some(true), "uTP · Encrypted"),
        (PeerTransport::Utp, Some(false), "uTP · Plaintext"),
        (PeerTransport::Unknown, None, "Unknown"),
    ] {
        peer.connection = ConnectionInfo {
            transport,
            encrypted,
        };
        assert_eq!(super::torrent::peer_connection(&peer), expected);
    }
}

/// Draws one status window the way `download_status_windows` does, then lets it act.
fn draw(
    app: &mut Desktop,
    ctx: &egui::Context,
    id: &str,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut window = app.download_status.windows.remove(id).unwrap();
    window.torrents_on = app.torrents.settings.enabled;
    let current_watch = app.extras.watches.get(&window.queue_id).map(|w| w.action);
    let power_states = app.extras.power_states;
    let mut action = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)),
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

fn shown(output: &egui::FullOutput, label: &str) -> bool {
    output.shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.trim() == label)
    })
}

/// Presses and releases at `pos`, and returns the frame of the release.
fn click_at(app: &mut Desktop, ctx: &egui::Context, id: &str, pos: egui::Pos2) -> egui::FullOutput {
    let mut last = None;
    for pressed in [true, false] {
        last = Some(draw(
            app,
            ctx,
            id,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        ));
    }
    last.unwrap()
}

fn click_text(app: &mut Desktop, ctx: &egui::Context, id: &str, label: &str) -> egui::FullOutput {
    draw(app, ctx, id, vec![]);
    let output = draw(app, ctx, id, vec![]);
    let pos = text_rect(&output, label).center();
    click_at(app, ctx, id, pos)
}

/// A paused torrent with three files, two of them chosen. Its queue is paused, so nothing
/// can start it, and tests never build the helper next to this binary.
fn paused_torrent(app: &mut Desktop) -> Job {
    app.store.set_queue_running("main", false).unwrap();
    app.store
        .save_torrent_settings(&Settings {
            enabled: true,
            dht: false,
            ..Default::default()
        })
        .unwrap();
    let file = |index, path: &str, len| File {
        index,
        path: path.into(),
        saved_as: path.into(),
        len,
    };
    let details = Details {
        info_hash: HASH.into(),
        name: "Sample pack".into(),
        magnet: format!(
            "magnet:?xt=urn:btih:{HASH}&tr=udp%3A%2F%2Ftracker.example.org%3A1337%2Fannounce"
        ),
        files: vec![
            file(0, "a.bin", 1000),
            file(1, "sub/b.bin", 2000),
            file(2, "c.bin", 3000),
        ],
    };
    let mut job = app
        .store
        .add_torrent(
            &details,
            vec![0, 1],
            None,
            "main",
            Uuid::new_v4().to_string(),
        )
        .unwrap();
    job.status = Status::Paused;
    app.store.save(&job).unwrap();
    app.torrents.settings.enabled = true;
    job
}

fn details(private: bool) -> TorrentDetails {
    TorrentDetails {
        private: Some(private),
        pieces: Some(Pieces {
            total: 3,
            wanted: 3,
            have: 2,
            in_progress: 1,
            checked: None,
            map: "99*".into(),
        }),
        peers: vec![Peer {
            connection: de_torrent::details::ConnectionInfo {
                transport: de_torrent::details::PeerTransport::Tcp,
                encrypted: Some(true),
            },
            address: Some("192.0.2.7:6881".into()),
            client: Some("qBittorrent/5.0.3".into()),
            has_pieces: 3,
            download_bps: 2048,
            upload_bps: 0,
            downloaded_bytes: 4096,
            uploaded_bytes: 0,
            incoming: false,
            receiving: true,
            sending: false,
        }],
        trackers: vec![
            Tracker {
                url: TRACKER.into(),
                added: false,
                state: TrackerState::Working,
                message: None,
                peers: Some(50),
                seeds: Some(212),
                leechers: Some(37),
                next_in: Some(1700),
            },
            Tracker {
                url: ADDED.into(),
                added: true,
                state: TrackerState::Failed,
                message: Some("timed out".into()),
                peers: None,
                seeds: None,
                leechers: None,
                next_in: Some(100),
            },
        ],
        dht: Source {
            state: SourceState::On,
            peers: 41,
        },
        pex: Source {
            state: SourceState::On,
            peers: 0,
        },
    }
}

/// The window as a running torrent's worker leaves it.
fn running_window(app: &mut Desktop, private: bool) -> Job {
    let mut job = paused_torrent(app);
    job.status = Status::Downloading;
    let mut window = Window::new(job.clone());
    window.phase = Some(Phase::Downloading);
    window.live_torrent = Some(TorrentSnapshot {
        peers: 1,
        download_speed: 2048,
        upload_speed: 0,
        uploaded: 0,
        file_progress: vec![1000, 1000, 0].into(),
        sharing: None,
    });
    window.torrent.apply(
        Some(Arc::new(details(private))),
        Some(app.store.torrent_file_list(&job.id).unwrap()),
        vec![ADDED.into()],
    );
    app.download_status.windows.insert(job.id.clone(), window);
    job
}

#[test]
fn a_running_torrent_shows_its_pieces_files_peers_and_trackers() {
    let (_temp, mut app, ctx) = fixture();
    let job = running_window(&mut app, false);
    draw(&mut app, &ctx, &job.id, vec![]);
    let output = draw(&mut app, &ctx, &job.id, vec![]);
    for label in [
        "Pieces · 2 of 3 pieces · 1 in progress",
        "Check files again",
        "Files  3",
        "Peers  1",
        "Trackers  2",
        "a.bin",
        "sub/",
        "b.bin",
        "c.bin",
        "Copy magnet link",
        "Supported",
    ] {
        text_rect(&output, label);
    }
    assert!(
        !shown(&output, "Connections · 1"),
        "the HTTP layout is gone"
    );
    assert!(
        !shown(&output, "Starts from the beginning"),
        "a torrent with progress resumes"
    );

    let output = click_text(&mut app, &ctx, &job.id, "Peers  1");
    assert_eq!(
        app.download_status.windows[&job.id].torrent.tab,
        TorrentTab::Peers
    );
    let output = if shown(&output, "192.0.2.7:6881") {
        output
    } else {
        draw(&mut app, &ctx, &job.id, vec![])
    };
    for label in ["192.0.2.7:6881", "qBittorrent/5.0.3", "Receiving", "100%"] {
        text_rect(&output, label);
    }

    click_text(&mut app, &ctx, &job.id, "Trackers  2");
    let output = draw(&mut app, &ctx, &job.id, vec![]);
    for label in [
        TRACKER,
        "Working",
        "No answer",
        "added by you",
        "212",
        "DHT",
        "Peer exchange",
        "Add tracker",
    ] {
        text_rect(&output, label);
    }
    assert!(!app.store.torrents.is_running());
}

#[test]
fn a_private_torrent_offers_no_way_to_add_a_tracker() {
    let (_temp, mut app, ctx) = fixture();
    let job = running_window(&mut app, true);
    app.download_status
        .windows
        .get_mut(&job.id)
        .unwrap()
        .torrent
        .tab = TorrentTab::Trackers;
    draw(&mut app, &ctx, &job.id, vec![]);
    let output = draw(&mut app, &ctx, &job.id, vec![]);
    text_rect(&output, download_engine::torrent::PRIVATE_TRACKERS_TEXT);
    // The button is off, so pressing it asks nothing.
    click_text(&mut app, &ctx, &job.id, "Add tracker");
    let window = &app.download_status.windows[&job.id];
    assert!(window.torrent.pending.is_none());
    assert!(window.torrent.outbox.is_empty());
}

#[test]
fn a_torrent_stopped_by_its_dht_search_says_so_in_the_chosen_language() {
    use download_engine::i18n::Language;
    let (_temp, mut app, ctx) = fixture();
    app.interface.language = Language::German;
    let _german = download_engine::i18n::scope(Language::German);
    let mut job = paused_torrent(&mut app);
    job.status = Status::Failed;
    job.error = Some(
        download_engine::torrent::MagnetStop::NothingFound
            .download_error()
            .into(),
    );
    app.store.save(&job).unwrap();
    let german = "Niemand hat die Dateiliste dieses Torrents geteilt. Die DHT-Suche wurde nach 10 Minuten beendet. Versuchen Sie es später erneut oder öffnen Sie die .torrent-Datei.";

    app.download_status
        .windows
        .insert(job.id.clone(), Window::new(job.clone()));
    draw(&mut app, &ctx, &job.id, vec![]);
    text_rect(&draw(&mut app, &ctx, &job.id, vec![]), german);

    app.jobs = vec![job];
    let size = egui::vec2(860.0, 600.0);
    crate::desktop::interaction_tests::frame(&mut app, &ctx, size, vec![]);
    text_rect(
        &crate::desktop::interaction_tests::frame(&mut app, &ctx, size, vec![]),
        german,
    );
}

#[test]
fn ticking_a_file_of_a_paused_torrent_saves_the_choice_and_keeps_it_paused() {
    let (_temp, mut app, ctx) = fixture();
    let job = paused_torrent(&mut app);
    app.open_status_and_wait(&job);
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.download_status.windows[&job.id]
        .torrent
        .picker
        .is_none()
    {
        assert!(Instant::now() < deadline, "the file list was not read");
        app.refresh_status_and_wait();
    }
    draw(&mut app, &ctx, &job.id, vec![]);
    let output = draw(&mut app, &ctx, &job.id, vec![]);
    let name = text_rect(&output, "c.bin");
    // The tick sits just left of the name.
    click_at(
        &mut app,
        &ctx,
        &job.id,
        egui::pos2(name.left() - 12.0, name.center().y),
    );
    // The scheduler saves the file list first and the download's new size right after.
    let deadline = Instant::now() + Duration::from_secs(10);
    let saved = loop {
        let mut chosen = app.store.torrent_files(&job.id).unwrap().chosen;
        chosen.sort_unstable();
        let saved = app.store.get(&job.id).unwrap().unwrap();
        if chosen == [0, 1, 2] && saved.total == Some(6000) {
            break saved;
        }
        assert!(
            Instant::now() < deadline,
            "the choice was not saved: {chosen:?} {:?}",
            saved.total
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        saved.status,
        Status::Paused,
        "a tick never starts a torrent"
    );
    assert!(!app.store.torrents.is_running());
}

#[test]
fn checking_a_paused_torrent_again_clears_saved_progress_and_starts_nothing() {
    let (_temp, mut app, ctx) = fixture();
    let job = paused_torrent(&mut app);
    let saved = app
        .store
        .paths
        .data
        .join("torrents")
        .join(format!("{HASH}.resume.json"));
    std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
    std::fs::write(&saved, b"{}").unwrap();
    app.open_status_and_wait(&job);
    click_text(&mut app, &ctx, &job.id, "Check files again");
    assert!(
        app.download_status.windows[&job.id]
            .torrent
            .pending
            .is_some(),
        "the request went to the scheduler"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.download_status.windows[&job.id]
        .torrent
        .message
        .is_none()
    {
        assert!(Instant::now() < deadline, "no answer to the request");
        app.refresh_status_and_wait();
    }
    assert_eq!(
        app.download_status.windows[&job.id].torrent.message,
        Some((true, RECHECK_LATER.to_owned()))
    );
    assert!(!saved.exists(), "the next start reads every file");
    assert_eq!(
        app.store.get(&job.id).unwrap().unwrap().status,
        Status::Paused
    );
    assert!(!app.store.torrents.is_running());
}

#[test]
fn copying_the_magnet_link_puts_the_whole_link_on_the_clipboard() {
    let (_temp, mut app, ctx) = fixture();
    let job = running_window(&mut app, false);
    let output = click_text(&mut app, &ctx, &job.id, "Copy magnet link");
    assert!(
        output
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(
                command,
                egui::OutputCommand::CopyText(text) if *text == job.url
            )),
        "{:?}",
        output.platform_output.commands
    );
    let output = draw(&mut app, &ctx, &job.id, vec![]);
    text_rect(&output, "Magnet link copied.");
}

#[test]
fn a_paused_torrent_keeps_the_last_details_it_showed() {
    let (_temp, mut app, ctx) = fixture();
    let job = running_window(&mut app, false);
    let window = app.download_status.windows.get_mut(&job.id).unwrap();
    // A refresh keeps each file's progress while the worker runs, as `Window::apply` does.
    window.torrent.progress = window
        .live_torrent
        .as_ref()
        .map(|live| live.file_progress.clone());
    // The worker has stopped: its live data is gone, and only what was seen stays.
    window.phase = None;
    window.live_torrent = None;
    window.job.as_mut().unwrap().status = Status::Paused;
    window.torrent.apply(None, None, vec![ADDED.into()]);
    draw(&mut app, &ctx, &job.id, vec![]);
    let output = draw(&mut app, &ctx, &job.id, vec![]);
    text_rect(&output, "Pieces · 2 of 3 pieces · 1 in progress");
    text_rect(
        &output,
        "Last seen less than a minute ago, while the torrent ran.",
    );
    assert!(shown(&output, "100%"), "file progress as last seen");
    click_text(&mut app, &ctx, &job.id, "Peers");
    let output = draw(&mut app, &ctx, &job.id, vec![]);
    text_rect(&output, "Peers show while the torrent runs.");
    click_text(&mut app, &ctx, &job.id, "Trackers  2");
    let output = draw(&mut app, &ctx, &job.id, vec![]);
    text_rect(&output, download_engine::torrent::ADD_WHILE_RUNNING);
}
