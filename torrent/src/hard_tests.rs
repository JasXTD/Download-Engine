//! The situations that break torrent engines: several peers at once, stopping halfway,
//! starting over, damaged data, and peers that lie or misbehave.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use buffers::ByteBuf;
use librqbit_core::hash_id::Id20;
use librqbit_peer_protocol::extended::ExtendedMessage;
use librqbit_peer_protocol::extended::ut_metadata::UtMetadata;
use librqbit_peer_protocol::{Message, Piece, Request};

use crate::meta::TorrentMeta;
use crate::metadata;
use crate::testkit::{
    FakePeer, PATIENCE, TempFolder, context, is_done, is_live, join, limited_context, meta_of,
    multi_file_torrent, multi_file_torrent_with, options_for, sample_bytes, seed_of,
    single_file_torrent, start_torrent, touch_later, wait_until, write_files,
};
use crate::torrent::{Torrent, TorrentOptions};

const PIECE: u32 = 32 * 1024;
const CHUNK: usize = 16 * 1024;

fn only_this_part(data: &[u8], part: std::ops::Range<usize>) -> Vec<u8> {
    let mut kept = vec![0u8; data.len()];
    kept[part.clone()].copy_from_slice(&data[part]);
    kept
}

async fn live_torrent(meta: &Arc<TorrentMeta>, folder: &TempFolder, id: u8) -> Arc<Torrent> {
    let torrent = start_torrent(meta, folder.path(), id);
    torrent.start().await;
    wait_until(&torrent, "the torrent to go live", is_live).await;
    torrent
}

#[tokio::test(flavor = "multi_thread")]
async fn test_two_peers_with_different_halves_complete_a_download() {
    // The split is on a piece boundary, so that each piece is whole on one of them.
    let half = 10 * PIECE as usize;
    let data = sample_bytes(2 * half, 11);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (one, two, three) = (TempFolder::new(), TempFolder::new(), TempFolder::new());
    write_files(&one, &[("a.bin", &only_this_part(&data, 0..half))]);
    write_files(&two, &[("a.bin", &only_this_part(&data, half..2 * half))]);
    let first = live_torrent(&meta, &one, 1).await;
    let second = live_torrent(&meta, &two, 2).await;
    assert_eq!(first.stats().progress_bytes, half as u64);
    assert_eq!(second.stats().progress_bytes, half as u64);

    let leech = live_torrent(&meta, &three, 3).await;
    join(&first, &leech);
    join(&second, &leech);
    wait_until(&leech, "the download to finish", is_done).await;
    leech.pause().await;
    assert!(std::fs::read(three.path().join("a.bin")).unwrap() == data);
    assert_eq!(first.stats().uploaded_bytes, half as u64);
    assert_eq!(second.stats().uploaded_bytes, half as u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_peer_uploads_pieces_while_still_downloading_them() {
    let data = sample_bytes(1_000_000, 12);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (one, two, three) = (TempFolder::new(), TempFolder::new(), TempFolder::new());
    let seed = seed_of(&meta, &one, &[("a.bin", &data)]).await;
    let relay = live_torrent(&meta, &two, 2).await;
    let leech = live_torrent(&meta, &three, 3).await;
    // The last peer can only get data through the one in the middle.
    join(&relay, &leech);
    join(&seed, &relay);
    wait_until(&leech, "the far peer to finish", is_done).await;
    leech.pause().await;
    assert!(std::fs::read(three.path().join("a.bin")).unwrap() == data);
    assert!(relay.stats().uploaded_bytes >= 1_000_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_pausing_stops_everything_and_resuming_finishes_without_refetching() {
    let data = sample_bytes(2_000_000, 13);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (one, two) = (TempFolder::new(), TempFolder::new());
    write_files(&one, &[("a.bin", &data)]);
    let seed = Torrent::from_meta(
        meta.clone(),
        options_for(one.path()),
        limited_context(1, Some(2_000_000)),
    )
    .unwrap();
    seed.start().await;
    wait_until(&seed, "the seed to check", is_done).await;
    let leech = live_torrent(&meta, &two, 2).await;
    join(&seed, &leech);
    wait_until(&leech, "some progress", |stats| {
        stats.progress_bytes > 300_000
    })
    .await;

    leech.pause().await;
    assert_eq!(leech.stats().state, "paused");
    wait_until(&seed, "the seed to see it go", |stats| stats.peers == 0).await;
    let paused_at = leech.stats().progress_bytes;
    let sent = seed.stats().uploaded_bytes;
    let on_disk = std::fs::read(two.path().join("a.bin")).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(leech.stats().progress_bytes, paused_at);
    assert_eq!(seed.stats().uploaded_bytes, sent);
    assert!(std::fs::read(two.path().join("a.bin")).unwrap() == on_disk);

    leech.start().await;
    wait_until(&leech, "the torrent to resume", is_live).await;
    join(&seed, &leech);
    wait_until(&leech, "the download to finish", is_done).await;
    leech.pause().await;
    assert!(std::fs::read(two.path().join("a.bin")).unwrap() == data);
    let allowed = data.len() as u64 + 300 * CHUNK as u64;
    assert!(
        seed.stats().uploaded_bytes <= allowed,
        "sent too much again"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_restart_keeps_finished_pieces() {
    let data = sample_bytes(2_000_000, 14);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (one, two) = (TempFolder::new(), TempFolder::new());
    write_files(&one, &[("a.bin", &data)]);
    let seed = Torrent::from_meta(
        meta.clone(),
        options_for(one.path()),
        limited_context(1, Some(2_000_000)),
    )
    .unwrap();
    seed.start().await;
    wait_until(&seed, "the seed to check", is_done).await;
    let first_try = live_torrent(&meta, &two, 2).await;
    join(&seed, &first_try);
    wait_until(&first_try, "some progress", |stats| {
        stats.progress_bytes > 500_000
    })
    .await;
    first_try.pause().await;
    let kept = first_try.stats().progress_bytes;
    drop(first_try);
    wait_until(&seed, "the seed to see it go", |stats| stats.peers == 0).await;

    let sent_before = seed.stats().uploaded_bytes;
    let second_try = live_torrent(&meta, &two, 3).await;
    assert!(
        second_try.stats().progress_bytes >= kept,
        "pieces were lost"
    );
    let needed = data.len() as u64 - second_try.stats().progress_bytes;
    join(&seed, &second_try);
    wait_until(&second_try, "the download to finish", is_done).await;
    second_try.pause().await;
    assert!(std::fs::read(two.path().join("a.bin")).unwrap() == data);
    let sent = seed.stats().uploaded_bytes - sent_before;
    assert!(sent <= needed + 300 * CHUNK as u64, "{sent} for {needed}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_damaged_file_loses_only_the_damaged_piece() {
    let data = sample_bytes(1_000_000, 15);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (one, two) = (TempFolder::new(), TempFolder::new());
    let seed = seed_of(&meta, &one, &[("a.bin", &data)]).await;
    let mut damaged = data.clone();
    damaged[500_000] ^= 0xff;
    write_files(&two, &[("a.bin", &damaged)]);

    let leech = live_torrent(&meta, &two, 2).await;
    assert_eq!(
        leech.stats().progress_bytes,
        data.len() as u64 - u64::from(PIECE)
    );
    join(&seed, &leech);
    wait_until(&leech, "the repair to finish", is_done).await;
    leech.pause().await;
    assert!(std::fs::read(two.path().join("a.bin")).unwrap() == data);
    assert_eq!(seed.stats().uploaded_bytes, u64::from(PIECE));
}

fn options_with_state(output: std::path::PathBuf, state: &TempFolder) -> TorrentOptions {
    TorrentOptions {
        state_dir: Some(state.path()),
        ..options_for(output)
    }
}

/// Downloads a whole torrent into `folder`, leaving a resume record in `state`.
async fn finished_download(
    meta: &Arc<TorrentMeta>,
    data: &[u8],
    folder: &TempFolder,
    state: &TempFolder,
) {
    let source = TempFolder::new();
    let seed = seed_of(meta, &source, &[("a.bin", data)]).await;
    let leech = Torrent::from_meta(
        meta.clone(),
        options_with_state(folder.path(), state),
        context(2),
    )
    .unwrap();
    leech.start().await;
    wait_until(&leech, "the torrent to go live", is_live).await;
    join(&seed, &leech);
    wait_until(&leech, "the download to finish", is_done).await;
    leech.pause().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_resume_record_avoids_hashing_files_that_did_not_change() {
    let data = sample_bytes(300_000, 16);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (folder, state) = (TempFolder::new(), TempFolder::new());
    finished_download(&meta, &data, &folder, &state).await;

    // Change a byte but keep the file's time. Only a real hash check could notice this,
    // so if the torrent still counts as complete, the check was skipped.
    let path = folder.path().join("a.bin");
    let modified: SystemTime = std::fs::metadata(&path).unwrap().modified().unwrap();
    let mut changed = data.clone();
    changed[0] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(modified).unwrap();
    drop(file);

    let again = Torrent::from_meta(
        meta.clone(),
        options_with_state(folder.path(), &state),
        context(3),
    )
    .unwrap();
    again.start().await;
    wait_until(&again, "the torrent to go live", is_live).await;
    assert!(again.stats().finished);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_file_changed_after_the_record_is_checked_again() {
    let data = sample_bytes(300_000, 17);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (folder, state) = (TempFolder::new(), TempFolder::new());
    finished_download(&meta, &data, &folder, &state).await;

    let path = folder.path().join("a.bin");
    let mut changed = data.clone();
    changed[0] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    touch_later(&path);

    let again = Torrent::from_meta(
        meta.clone(),
        options_with_state(folder.path(), &state),
        context(3),
    )
    .unwrap();
    again.start().await;
    wait_until(&again, "the torrent to go live", is_live).await;
    let stats = again.stats();
    assert!(!stats.finished);
    assert_eq!(stats.progress_bytes, data.len() as u64 - u64::from(PIECE));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_peer_sending_bad_data_is_dropped_and_the_download_still_finishes() {
    let data = sample_bytes(640_000, 18);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (one, two) = (TempFolder::new(), TempFolder::new());
    let seed = seed_of(&meta, &one, &[("a.bin", &data)]).await;
    let leech = live_torrent(&meta, &two, 2).await;

    let mut liar = FakePeer::join(&leech, 9).await;
    let everything = vec![0xffu8; 3];
    liar.send(Message::Bitfield(ByteBuf(&everything))).await;
    liar.send(Message::Unchoke).await;
    let junk = vec![0x55u8; CHUNK];
    while let Some(request) = liar.next_request().await {
        let block = &junk[..request.length as usize];
        let piece = Piece::from_data(request.index, request.begin, block);
        liar.send(Message::Piece(piece)).await;
    }
    assert_eq!(leech.stats().progress_bytes, 0, "bad data must not count");

    join(&seed, &leech);
    wait_until(&leech, "the download to finish", is_done).await;
    leech.pause().await;
    assert!(std::fs::read(two.path().join("a.bin")).unwrap() == data);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_peer_that_stops_answering_cannot_hold_up_the_end() {
    let data = sample_bytes(320_000, 19);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (one, two) = (TempFolder::new(), TempFolder::new());
    let seed = seed_of(&meta, &one, &[("a.bin", &data)]).await;
    let leech = live_torrent(&meta, &two, 2).await;

    let mut silent = FakePeer::join(&leech, 9).await;
    let everything = vec![0xffu8; 2];
    silent.send(Message::Bitfield(ByteBuf(&everything))).await;
    silent.send(Message::Unchoke).await;
    assert!(silent.next_request().await.is_some());

    // The silent peer holds requests for everything, and a real peer joins too late to
    // get new work. Asking twice must finish this long before any timeout would.
    let started = tokio::time::Instant::now();
    join(&seed, &leech);
    wait_until(&leech, "the download to finish", is_done).await;
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(
        silent
            .wait_for(|message| matches!(message, Message::Cancel(_)))
            .await,
        "the silent peer should be told it isn't needed"
    );
}

async fn dropped_for(meta: &Arc<TorrentMeta>, build: impl AsyncFnOnce(&mut FakePeer)) {
    let folder = TempFolder::new();
    let torrent = live_torrent(meta, &folder, 2).await;
    let mut peer = FakePeer::join(&torrent, 9).await;
    build(&mut peer).await;
    assert!(peer.is_dropped().await, "the peer should have been dropped");
    assert_eq!(torrent.stats().progress_bytes, 0);
}

fn small_meta() -> Arc<TorrentMeta> {
    meta_of(&single_file_torrent(
        "a.bin",
        &sample_bytes(100_000, 20),
        PIECE,
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_message_claiming_to_be_huge_drops_the_peer() {
    dropped_for(&small_meta(), async |peer| {
        peer.send_raw(&[0xff, 0xff, 0xff, 0xff]).await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_bitfield_of_the_wrong_size_drops_the_peer() {
    dropped_for(&small_meta(), async |peer| {
        peer.send(Message::Bitfield(ByteBuf(&[0xff; 99]))).await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_claiming_a_piece_that_does_not_exist_drops_the_peer() {
    dropped_for(&small_meta(), async |peer| {
        peer.send(Message::Have(u32::MAX)).await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_asking_for_a_piece_that_does_not_exist_drops_the_peer() {
    dropped_for(&small_meta(), async |peer| {
        peer.send(Message::Request(Request::new(999, 0, 16384)))
            .await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_asking_past_the_end_of_a_piece_drops_the_peer() {
    dropped_for(&small_meta(), async |peer| {
        let past = Request::new(0, u32::MAX, 16384);
        peer.send(Message::Request(past)).await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_block_for_a_piece_that_does_not_exist_drops_the_peer() {
    dropped_for(&small_meta(), async |peer| {
        let block = vec![0u8; CHUNK];
        peer.send(Message::Piece(Piece::from_data(999, 0, &block)))
            .await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_block_of_the_wrong_size_drops_the_peer() {
    dropped_for(&small_meta(), async |peer| {
        let block = vec![0u8; 100];
        peer.send(Message::Piece(Piece::from_data(0, 0, &block)))
            .await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_blocks_nobody_asked_for_are_refused_then_drop_the_peer() {
    let data = sample_bytes(100_000, 21);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let folder = TempFolder::new();
    let torrent = live_torrent(&meta, &folder, 2).await;
    let mut peer = FakePeer::join(&torrent, 9).await;
    let block = data[..CHUNK].to_vec();
    for _ in 0..100 {
        peer.send(Message::Piece(Piece::from_data(0, 0, &block)))
            .await;
    }
    assert!(peer.is_dropped().await);
    assert!(
        !folder.path().join("a.bin").exists(),
        "nothing may be written"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_an_unknown_message_is_skipped() {
    let data = sample_bytes(100_000, 22);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let folder = TempFolder::new();
    let torrent = live_torrent(&meta, &folder, 2).await;
    let mut peer = FakePeer::join(&torrent, 9).await;
    peer.send_raw(&[0, 0, 0, 2, 200, 1]).await;
    peer.send(Message::Bitfield(ByteBuf(&[0xff]))).await;
    assert!(
        peer.wait_for(|message| matches!(message, Message::Interested))
            .await,
        "the connection should carry on"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_peer_gets_the_exact_bytes_it_asks_for_once_unchoked() {
    let data = sample_bytes(100_000, 23);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let folder = TempFolder::new();
    let seed = seed_of(&meta, &folder, &[("a.bin", &data)]).await;
    let mut peer = FakePeer::join(&seed, 9).await;

    // Asking while choked gets nothing.
    let request = Request::new(1, 100, 1000);
    peer.send(Message::Request(request)).await;
    peer.send(Message::Interested).await;
    assert!(
        peer.wait_for(|message| matches!(message, Message::Unchoke))
            .await
    );
    peer.send(Message::Request(request)).await;
    let start = PIECE as usize + 100;
    let expected = data[start..start + 1000].to_vec();
    let matched = peer
        .wait_for(|message| match message {
            Message::Piece(piece) => {
                assert_eq!((piece.index, piece.begin), (1, 100));
                piece.data().0 == expected.as_slice()
            }
            _ => false,
        })
        .await;
    assert!(matched);
    assert_eq!(seed.stats().uploaded_bytes, 1000);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_cancelled_block_is_not_sent() {
    let data = sample_bytes(100_000, 24);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (one, two) = (TempFolder::new(), TempFolder::new());
    write_files(&one, &[("a.bin", &data)]);
    let seed = Torrent::from_meta(
        meta.clone(),
        options_for(one.path()),
        limited_context(1, Some(64 * 1024)),
    )
    .unwrap();
    seed.start().await;
    wait_until(&seed, "the seed to check", is_done).await;
    drop(two);
    let mut peer = FakePeer::join(&seed, 9).await;
    peer.send(Message::Interested).await;
    assert!(
        peer.wait_for(|message| matches!(message, Message::Unchoke))
            .await
    );
    // The limit holds up the first blocks long enough for the cancel to arrive.
    let requests: Vec<Request> = (0..6)
        .map(|index| Request::new(index, 0, CHUNK as u32))
        .collect();
    for request in &requests {
        peer.send(Message::Request(*request)).await;
    }
    peer.send(Message::Cancel(requests[5])).await;
    let mut received = Vec::new();
    while received.len() < 5 {
        let frame = peer.next_frame(Duration::from_secs(1)).await;
        let Some(frame) = frame else {
            break;
        };
        if let Ok((Message::Piece(piece), _)) = Message::deserialize(&frame, &[]) {
            received.push(piece.index);
        }
    }
    assert!(!received.contains(&5), "{received:?}");
    assert!(peer.next_frame(Duration::from_secs(1)).await.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_only_chosen_files_are_fetched_and_more_can_be_chosen_later() {
    let first = sample_bytes(100_000, 25);
    let second = sample_bytes(100_000, 26);
    let third = sample_bytes(100_000, 27);
    let torrent = multi_file_torrent(
        "pack",
        &[
            (&["a"], &first, false),
            (&["b"], &second, false),
            (&["c"], &third, false),
        ],
        PIECE,
    );
    let meta = meta_of(&torrent);
    let (one, two) = (TempFolder::new(), TempFolder::new());
    let seed = seed_of(&meta, &one, &[("a", &first), ("b", &second), ("c", &third)]).await;
    let options = TorrentOptions {
        only: Some(vec![1]),
        ..options_for(two.path())
    };
    let leech = Torrent::from_meta(meta.clone(), options, context(2)).unwrap();
    leech.start().await;
    wait_until(&leech, "the torrent to go live", is_live).await;
    join(&seed, &leech);
    wait_until(&leech, "the chosen file", is_done).await;

    let stats = leech.stats();
    assert_eq!(stats.file_progress[1], 100_000);
    assert!(stats.file_progress[0] <= u64::from(PIECE));
    assert!(stats.file_progress[2] <= u64::from(PIECE));
    let sent = seed.stats().uploaded_bytes;
    assert!(sent <= 100_000 + 2 * u64::from(PIECE), "sent {sent}");
    assert!(std::fs::read(two.path().join("b")).unwrap() == second);

    leech.select(Some(vec![1, 2])).unwrap();
    assert!(!leech.stats().finished);
    join(&seed, &leech);
    wait_until(&leech, "the file chosen later", is_done).await;
    leech.pause().await;
    assert!(std::fs::read(two.path().join("c")).unwrap() == third);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_bad_file_number_is_refused() {
    let meta = small_meta();
    let options = TorrentOptions {
        only: Some(vec![5]),
        ..options_for(TempFolder::new().path())
    };
    assert!(Torrent::from_meta(meta.clone(), options, context(2)).is_err());
    let folder = TempFolder::new();
    let torrent = live_torrent(&meta, &folder, 2).await;
    assert!(torrent.select(Some(vec![5])).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_torrent_that_cannot_write_reports_an_error() {
    let data = sample_bytes(100_000, 28);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (one, two) = (TempFolder::new(), TempFolder::new());
    let seed = seed_of(&meta, &one, &[("a.bin", &data)]).await;
    // A file where the download folder should be can't be written into.
    let blocked = two.path().join("not-a-folder");
    std::fs::write(&blocked, b"x").unwrap();
    let leech = start_torrent(&meta, blocked, 2);
    leech.start().await;
    wait_until(&leech, "the torrent to go live", is_live).await;
    join(&seed, &leech);
    let reported = tokio::time::timeout(PATIENCE, async {
        loop {
            if leech.stats().error.is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(reported.is_ok());
    assert_eq!(leech.stats().state, "error");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_without_seeding_a_finished_torrent_stops() {
    let data = sample_bytes(100_000, 29);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (one, two) = (TempFolder::new(), TempFolder::new());
    let seed = seed_of(&meta, &one, &[("a.bin", &data)]).await;
    let options = TorrentOptions {
        seed: false,
        ..options_for(two.path())
    };
    let leech = Torrent::from_meta(meta.clone(), options, context(2)).unwrap();
    leech.start().await;
    wait_until(&leech, "the torrent to go live", is_live).await;
    join(&seed, &leech);
    wait_until(&leech, "the torrent to stop", |stats| {
        stats.state == "finished"
    })
    .await;
    assert!(leech.stats().finished);
    assert_eq!(leech.stats().peers, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_magnet_link_gets_its_details_from_a_peer() {
    let data = sample_bytes(4_000_000, 30);
    // A small piece size makes the details bigger than one metadata piece.
    let meta = meta_of(&single_file_torrent("a.bin", &data, 1024));
    assert!(meta.info_bytes.len() > CHUNK * 3);
    let folder = TempFolder::new();
    let seed = seed_of(&meta, &folder, &[("a.bin", &data)]).await;

    let (one, two) = tokio::io::duplex(1 << 20);
    seed.connect_stream(Box::new(one)).unwrap();
    let our_id = Id20::new([9; 20]);
    let fetched = metadata::fetch(Box::new(two), meta.info_hash, our_id)
        .await
        .unwrap();
    assert!(fetched == meta.info_bytes);
    let rebuilt = TorrentMeta::from_info_bytes(meta.info_hash, fetched, Vec::new()).unwrap();
    assert_eq!(rebuilt.name, "a.bin");
    assert_eq!(rebuilt.lengths.total_length(), data.len() as u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_private_torrent_does_not_offer_peer_exchange() {
    let data = sample_bytes(1000, 31);
    let torrent = multi_file_torrent_with("pack", &[(&["a"], &data, false)], PIECE, None, true);
    let meta = meta_of(&torrent);
    assert!(meta.private);
    let folder = TempFolder::new();
    let live = live_torrent(&meta, &folder, 2).await;
    let mut peer = FakePeer::join(&live, 9).await;
    let told = peer
        .wait_for(|message| match message {
            Message::Extended(librqbit_peer_protocol::extended::ExtendedMessage::Handshake(
                handshake,
            )) => handshake.m.ut_pex.is_none(),
            _ => false,
        })
        .await;
    assert!(told);
}

/// A small repeatable random number source.
struct Dice(u64);

impl Dice {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, limit: u64) -> u64 {
        self.next() % limit
    }
}

fn frame(id: u8, body: &[u8]) -> Vec<u8> {
    let length = body.len() as u32 + 1;
    [&length.to_be_bytes()[..], &[id], body].concat()
}

/// Something shaped like a peer message, whose values are mostly wrong.
fn random_message(dice: &mut Dice) -> Vec<u8> {
    let index = dice.below(30) as u32;
    let begin = dice.below(3) as u32 * CHUNK as u32 + dice.below(2) as u32;
    let length = [0, 1, 1000, 16384, 16385, 1 << 20, u32::MAX][dice.below(7) as usize];
    let numbers = [index, begin, length].map(u32::to_be_bytes).concat();
    match dice.below(10) {
        0 => {
            let body = sample_bytes(dice.below(60) as usize, dice.next());
            frame(dice.below(25) as u8, &body)
        }
        1 => frame(4, &index.to_be_bytes()),
        2 => frame(5, &sample_bytes(dice.below(5) as usize, dice.next())),
        3 => frame(6, &numbers),
        4 => frame(8, &numbers),
        5 => {
            let size = [1, 100, CHUNK, CHUNK + 1][dice.below(4) as usize];
            let head = [index, begin].map(u32::to_be_bytes).concat();
            frame(7, &[head, sample_bytes(size, dice.next())].concat())
        }
        6 => {
            let size = dice.below(1 << 26);
            let queue = dice.below(100_000);
            let body = format!(
                "d1:md6:ut_pexi1e11:ut_metadatai3ee4:reqqi{queue}e13:metadata_sizei{size}ee"
            );
            frame(20, &[&[0], body.as_bytes()].concat())
        }
        7 => {
            let piece = dice.below(50);
            let body = format!("d8:msg_typei{}e5:piecei{piece}ee", dice.below(4));
            frame(20, &[&[3], body.as_bytes()].concat())
        }
        8 => {
            let addresses = sample_bytes(6 * dice.below(4) as usize, dice.next());
            let head = format!("d5:added{}:", addresses.len());
            frame(20, &[b"\x01", head.as_bytes(), &addresses, b"e"].concat())
        }
        _ => sample_bytes(dice.below(20) as usize, dice.next()),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_random_messages_cannot_crash_a_connection_or_spoil_a_download() {
    let data = sample_bytes(640_000, 32);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let (one, two) = (TempFolder::new(), TempFolder::new());
    let seed = seed_of(&meta, &one, &[("a.bin", &data)]).await;
    let leech = live_torrent(&meta, &two, 2).await;

    let mut dice = Dice(0x9E37_79B9_7F4A_7C15);
    for round in 0..600u32 {
        let target = if round % 2 == 0 { &seed } else { &leech };
        let mut peer = FakePeer::join(target, 100 + (round % 100) as u8).await;
        for _ in 0..=dice.below(30) {
            let message = random_message(&mut dice);
            peer.send_raw(&message).await;
        }
        drop(peer);
        while target.stats().peers > 20 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    assert_eq!(seed.crashed_connections(), 0);
    assert_eq!(leech.crashed_connections(), 0);
    join(&seed, &leech);
    wait_until(&leech, "the download to finish", is_done).await;
    leech.pause().await;
    assert_eq!(leech.crashed_connections(), 0);
    assert!(std::fs::read(two.path().join("a.bin")).unwrap() == data);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_uploading_under_a_limit_does_not_hold_up_requests() {
    let half = 10 * PIECE as usize;
    let data = sample_bytes(2 * half, 34);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let folder = TempFolder::new();
    write_files(&folder, &[("a.bin", &only_this_part(&data, 0..half))]);
    let relay = Torrent::from_meta(
        meta.clone(),
        options_for(folder.path()),
        limited_context(2, Some(1024)),
    )
    .unwrap();
    relay.start().await;
    wait_until(&relay, "the torrent to go live", is_live).await;

    let mut peer = FakePeer::join(&relay, 9).await;
    peer.send(Message::Interested).await;
    assert!(
        peer.wait_for(|message| matches!(message, Message::Unchoke))
            .await
    );
    // At this limit, these blocks use up the allowance for many seconds.
    for begin in [0, CHUNK as u32] {
        peer.send(Message::Request(Request::new(0, begin, CHUNK as u32)))
            .await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    // The same peer now offers what we need, and asking for it must not wait for that.
    peer.send(Message::Bitfield(ByteBuf(&[0xff, 0xff, 0xf0])))
        .await;
    peer.send(Message::Unchoke).await;
    let asked = tokio::time::timeout(Duration::from_secs(2), peer.next_request()).await;
    assert!(matches!(asked, Ok(Some(_))), "the request was held up");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_with_uploads_off_a_peer_gets_nothing() {
    let data = sample_bytes(100_000, 35);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let folder = TempFolder::new();
    write_files(&folder, &[("a.bin", &data)]);
    let options = TorrentOptions {
        no_upload: true,
        ..options_for(folder.path())
    };
    let seed = Torrent::from_meta(meta.clone(), options, context(1)).unwrap();
    seed.start().await;
    wait_until(&seed, "the seed to check", is_done).await;

    let mut peer = FakePeer::join(&seed, 9).await;
    peer.send(Message::Interested).await;
    peer.send(Message::Request(Request::new(0, 0, CHUNK as u32)))
        .await;
    let body = b"d1:md6:ut_pexi1e11:ut_metadatai3eee";
    peer.send_raw(&frame(20, &[&[0], &body[..]].concat())).await;
    peer.send_raw(&frame(20, b"\x03d8:msg_typei0e5:piecei0ee"))
        .await;
    assert!(
        peer.wait_for(|message| matches!(
            message,
            Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Reject(0)))
        ))
        .await,
        "the torrent's details count as uploading too"
    );
    let unchoked = tokio::time::timeout(
        Duration::from_secs(1),
        peer.wait_for(|message| matches!(message, Message::Unchoke | Message::Piece(_))),
    )
    .await;
    assert!(unchoked.is_err(), "a peer was offered data");
    assert_eq!(seed.stats().uploaded_bytes, 0);
}
