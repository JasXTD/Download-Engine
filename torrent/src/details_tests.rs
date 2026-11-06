//! The status window's view of a torrent: peers, pieces, trackers and checking files again.
//! Peers are joined by in-memory pipes, and no discovery runs, so nothing reaches a network.

use std::time::Duration;

use buffers::ByteBuf;
use librqbit_core::hash_id::Id20;
use librqbit_peer_protocol::Message;
use url::Url;

use crate::details::{SourceState, TrackerState};
use crate::peer::{Hello, read_handshake};
use crate::session::{AddOptions, Session, SessionOptions};
use crate::testkit::{
    FakePeer, PATIENCE, TempFolder, context, is_done, is_live, join, meta_of,
    multi_file_torrent_with, options_for, sample_bytes, seed_of, single_file_torrent,
    start_torrent, wait_until,
};
use crate::torrent::{PRIVATE_TRACKERS, Torrent};

const PIECE: u32 = 16384;
const TRACKER: &str = "http://tracker.example.org/announce";
const ADDED: &str = "udp://open.example.org:6969/announce";

fn frame(id: u8, body: &[u8]) -> Vec<u8> {
    let length = body.len() as u32 + 1;
    [&length.to_be_bytes()[..], &[id], body].concat()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_shows_its_cleaned_name_what_it_has_and_that_it_sends() {
    let data = sample_bytes(4 * PIECE as usize, 50);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let folder = TempFolder::new();
    let torrent = start_torrent(&meta, folder.path(), 2);
    torrent.start().await;
    wait_until(&torrent, "the torrent to start", is_live).await;
    let mut peer = FakePeer::join(&torrent, 9).await;
    let name = "Fake\u{202E}Client\r\n1.0";
    let handshake = format!("d1:v{}:{name}e", name.len());
    peer.send_raw(&frame(20, &[&[0u8][..], handshake.as_bytes()].concat()))
        .await;
    peer.send(Message::Bitfield(ByteBuf(&[0xf0]))).await;
    peer.send(Message::Unchoke).await;
    assert!(
        peer.next_request().await.is_some(),
        "the torrent asks for a block"
    );

    let details = torrent.details();
    assert_eq!(details.peers.len(), 1);
    let seen = &details.peers[0];
    assert_eq!(seen.client.as_deref(), Some("Fake Client 1.0"));
    assert_eq!(seen.has_pieces, 4);
    assert!(seen.receiving, "the peer unchoked us and has what we want");
    assert!(!seen.sending, "the peer wants nothing from us");
    assert!(!seen.incoming, "we joined it");
    assert_eq!(seen.address, None, "a pipe has no address");
    assert_eq!(details.private, Some(false));
    assert_eq!(details.pex.state, SourceState::On);
    assert_eq!(
        details.dht.state,
        SourceState::Off,
        "tests run without the DHT"
    );

    torrent.pause().await;
    assert!(torrent.details().peers.is_empty());
    assert_eq!(torrent.details().pex.state, SourceState::Waiting);
}

#[tokio::test]
async fn a_peer_that_connected_to_us_shows_its_address_in_plain_form() {
    let meta = meta_of(&single_file_torrent("a", &[1u8; 100], PIECE));
    let folder = TempFolder::new();
    let torrent = start_torrent(&meta, folder.path(), 2);
    torrent.start().await;
    wait_until(&torrent, "the torrent to start", is_live).await;
    let (server, mut client) = tokio::io::duplex(1 << 16);
    torrent
        .offer(
            Box::new(server),
            "[::ffff:192.0.2.7]:6000".parse().unwrap(),
            Hello {
                peer_id: Id20::new([9; 20]),
                supports_extended: false,
            },
        )
        .unwrap();
    read_handshake(&mut client).await.unwrap();
    let seen = tokio::time::timeout(PATIENCE, async {
        loop {
            if let Some(peer) = torrent.details().peers.pop() {
                return peer;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the peer is listed");
    assert_eq!(seen.address.as_deref(), Some("192.0.2.7:6000"));
    assert!(seen.incoming);
    assert_eq!(seen.client, None);
    torrent.pause().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_piece_counts_and_map_follow_the_download() {
    let data = sample_bytes(300_000, 51);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (seed_folder, leech_folder) = (TempFolder::new(), TempFolder::new());
    let seed = seed_of(&meta, &seed_folder, &[("a.bin", &data)]).await;
    let leech = start_torrent(&meta, leech_folder.path(), 2);
    leech.start().await;
    wait_until(&leech, "the download to start", is_live).await;
    let before = leech.details().pieces.unwrap();
    assert_eq!((before.have, before.wanted, before.total), (0, 19, 19));
    assert_eq!(before.map, "0".repeat(19));
    assert_eq!(before.checked, None);

    join(&seed, &leech);
    wait_until(&leech, "the download to finish", is_done).await;
    let after = leech.details().pieces.unwrap();
    assert_eq!((after.have, after.in_progress), (19, 0));
    assert_eq!(after.map, "9".repeat(19));
    leech.pause().await;
    seed.pause().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn checking_again_ignores_saved_progress_and_fetches_a_damaged_piece() {
    let data = sample_bytes(200_000, 52);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (seed_folder, leech_folder, state) =
        (TempFolder::new(), TempFolder::new(), TempFolder::new());
    let seed = seed_of(&meta, &seed_folder, &[("a.bin", &data)]).await;
    let mut options = options_for(leech_folder.path());
    options.state_dir = Some(state.path());
    let leech = Torrent::from_meta(meta.clone(), options, context(2)).unwrap();
    leech.start().await;
    wait_until(&leech, "the download to start", is_live).await;
    join(&seed, &leech);
    wait_until(&leech, "the download to finish", is_done).await;
    // Stopping saves progress with the file's size and time.
    leech.pause().await;

    // Damage one byte without changing the file's size or time, as a failing disk might.
    let path = leech_folder.path().join("a.bin");
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[100_000] ^= 0xff;
    std::fs::write(&path, &bytes).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();

    leech.start().await;
    wait_until(&leech, "a start from saved progress", is_done).await;
    leech.recheck().await.unwrap();
    wait_until(&leech, "the check to find the damage", |stats| {
        stats.state == "live" && !stats.finished
    })
    .await;
    let pieces = leech.details().pieces.unwrap();
    assert_eq!(
        pieces.have,
        pieces.wanted - 1,
        "only the damaged piece is missing"
    );
    assert_eq!(
        pieces.map.chars().filter(|cell| *cell == '0').count(),
        1,
        "{}",
        pieces.map
    );

    join(&seed, &leech);
    wait_until(&leech, "the piece to arrive again", is_done).await;
    leech.pause().await;
    seed.pause().await;
    assert!(
        std::fs::read(&path).unwrap() == data,
        "the file is whole again"
    );

    // A finished check makes saved progress count again.
    leech.start().await;
    wait_until(&leech, "a quick start", is_done).await;
    leech.pause().await;
}

#[tokio::test]
async fn checking_again_needs_chosen_files_and_leaves_a_paused_torrent_paused() {
    let folder = TempFolder::new();
    let magnet = Torrent::from_magnet(
        Id20::new([3; 20]),
        Vec::new(),
        options_for(folder.path()),
        context(2),
    );
    assert!(magnet.recheck().await.is_err(), "nothing is known to check");

    let data = sample_bytes(40_000, 54);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let output = TempFolder::new();
    std::fs::write(output.path().join("a.bin"), &data).unwrap();
    let torrent = start_torrent(&meta, output.path(), 2);
    torrent.start().await;
    wait_until(&torrent, "the check of the files", is_done).await;
    torrent.pause().await;
    torrent.recheck().await.unwrap();
    assert!(torrent.is_paused());
    assert_eq!(torrent.details().pieces.unwrap().have, 0);
    torrent.start().await;
    wait_until(&torrent, "the files to be found whole", is_done).await;
    torrent.pause().await;
}

fn with_tracker(private: bool) -> std::sync::Arc<crate::meta::TorrentMeta> {
    let data = sample_bytes(1000, 53);
    meta_of(&multi_file_torrent_with(
        "pack",
        &[(&["a"], &data, false)],
        PIECE,
        Some(TRACKER),
        private,
    ))
}

#[tokio::test]
async fn trackers_the_user_adds_are_checked_listed_and_can_be_removed() {
    let folder = TempFolder::new();
    let torrent = start_torrent(&with_tracker(false), folder.path(), 2);
    let long = format!("http://{}.example.org/announce", "a".repeat(3000));
    for bad in [
        "ftp://tracker.example.org/announce",
        "udp://tracker.example.org/announce",
        "http://127.0.0.1:6969/announce",
        "http://[::1]/announce",
        "udp://192.168.1.1:6969/announce",
        "http://localhost/announce",
        TRACKER,
        "not an address",
        long.as_str(),
    ] {
        assert!(torrent.add_tracker(bad).is_err(), "{bad}");
    }
    torrent.add_tracker(&format!("  {ADDED} ")).unwrap();
    assert!(torrent.add_tracker(ADDED).is_err(), "it is there already");
    let trackers = torrent.details().trackers;
    assert_eq!(
        trackers
            .iter()
            .map(|tracker| (tracker.url.as_str(), tracker.added))
            .collect::<Vec<_>>(),
        [(TRACKER, false), (ADDED, true)]
    );
    assert!(
        trackers
            .iter()
            .all(|tracker| tracker.state == TrackerState::Off),
        "tests run without trackers"
    );
    assert!(
        torrent.remove_tracker(TRACKER).is_err(),
        "the torrent's own trackers stay"
    );
    torrent.remove_tracker(ADDED).unwrap();
    assert_eq!(torrent.details().trackers.len(), 1);
    assert!(torrent.remove_tracker(ADDED).is_err());
}

#[test]
fn a_private_torrent_keeps_to_its_own_trackers() {
    let folder = TempFolder::new();
    let mut options = options_for(folder.path());
    options.trackers = vec![Url::parse(ADDED).unwrap()];
    let torrent = Torrent::from_meta(with_tracker(true), options, context(2)).unwrap();
    let details = torrent.details();
    assert_eq!(
        details.trackers.len(),
        1,
        "a saved added tracker isn't used"
    );
    assert_eq!(details.private, Some(true));
    assert_eq!(details.pex.state, SourceState::Private);
    let error = torrent.add_tracker(ADDED).unwrap_err();
    assert_eq!(error.to_string(), PRIVATE_TRACKERS);
}

#[test]
fn a_magnet_link_without_details_lists_its_trackers_and_refuses_new_ones() {
    let folder = TempFolder::new();
    let torrent = Torrent::from_magnet(
        Id20::new([3; 20]),
        vec![Url::parse(TRACKER).unwrap()],
        options_for(folder.path()),
        context(2),
    );
    let details = torrent.details();
    assert_eq!(details.private, None);
    assert!(details.pieces.is_none());
    assert!(details.peers.is_empty());
    assert_eq!(details.trackers.len(), 1);
    assert_eq!(details.pex.state, SourceState::Waiting);
    assert!(
        torrent.add_tracker(ADDED).is_err(),
        "whether it is private isn't known yet"
    );
}

#[tokio::test]
async fn trackers_passed_with_add_are_listed_as_added_and_unusable_ones_are_left_out() {
    let (state, output) = (TempFolder::new(), TempFolder::new());
    let data = sample_bytes(1000, 55);
    let file = multi_file_torrent_with(
        "pack",
        &[(&["a"], &data, false)],
        PIECE,
        Some(TRACKER),
        false,
    );
    let session = Session::new(SessionOptions {
        state_dir: Some(state.path()),
        ..Default::default()
    })
    .await
    .unwrap();
    session
        .add_torrent_file(
            "one".into(),
            &file,
            AddOptions {
                output: output.path(),
                trackers: vec![
                    ADDED.into(),
                    "ftp://tracker.example.org/".into(),
                    TRACKER.into(),
                    ADDED.into(),
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let trackers = session.torrent("one").unwrap().details().trackers;
    assert_eq!(
        trackers
            .iter()
            .map(|tracker| (tracker.url.as_str(), tracker.added))
            .collect::<Vec<_>>(),
        [(TRACKER, false), (ADDED, true)]
    );
    session.shutdown().await;
}
