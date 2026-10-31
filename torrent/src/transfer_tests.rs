//! Whole transfers between two engines, joined by in-memory pipes. No sockets are opened,
//! so these run anywhere and can't reach a network.

use std::sync::Arc;
use std::time::Duration;

use librqbit_core::hash_id::Id20;
use tokio::time::timeout;

use crate::meta::TorrentMeta;
use crate::rate::ByteLimiter;
use crate::testkit::{TempFolder, meta_of, multi_file_torrent, sample_bytes, single_file_torrent};
use crate::torrent::{Context, Torrent, TorrentOptions, TorrentStats, Totals};

const PIECE: u32 = 32 * 1024;
const PATIENCE: Duration = Duration::from_secs(30);

fn context(id: u8) -> Context {
    Context {
        peer_id: Id20::new([id; 20]),
        up: Arc::new(ByteLimiter::new(None)),
        down: Arc::new(ByteLimiter::new(None)),
        totals: Arc::new(Totals::default()),
        discovery: None,
    }
}

fn start_torrent(meta: &Arc<TorrentMeta>, output: std::path::PathBuf, id: u8) -> Arc<Torrent> {
    let options = TorrentOptions {
        output,
        seed: true,
        ..Default::default()
    };
    Torrent::from_meta(meta.clone(), options, context(id)).unwrap()
}

async fn wait_until(torrent: &Torrent, what: &str, done: impl Fn(&TorrentStats) -> bool) {
    let waited = timeout(PATIENCE, async {
        loop {
            let stats = torrent.stats();
            assert!(stats.error.is_none(), "{what}: {:?}", stats.error);
            if done(&stats) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "timed out: {what}: {:?}", torrent.stats());
}

fn is_live(stats: &TorrentStats) -> bool {
    stats.state == "live"
}

fn is_done(stats: &TorrentStats) -> bool {
    stats.state == "live" && stats.finished
}

fn join(left: &Torrent, right: &Torrent) {
    let (one, two) = tokio::io::duplex(256 * 1024);
    left.connect_stream(Box::new(one)).unwrap();
    right.connect_stream(Box::new(two)).unwrap();
}

/// Starts a torrent over complete files, and waits until it knows they are complete.
async fn seed_of(
    meta: &Arc<TorrentMeta>,
    folder: &TempFolder,
    files: &[(&str, &[u8])],
) -> Arc<Torrent> {
    for (name, data) in files {
        let path = folder.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, data).unwrap();
    }
    let seed = start_torrent(meta, folder.path(), 1);
    seed.start().await;
    wait_until(&seed, "the seed to check its files", is_done).await;
    seed
}

#[tokio::test(flavor = "multi_thread")]
async fn test_one_peer_downloads_one_file() {
    let data = sample_bytes(1_000_000, 1);
    let meta = meta_of(&single_file_torrent("movie.bin", &data, PIECE));
    let (seed_folder, leech_folder) = (TempFolder::new(), TempFolder::new());
    let seed = seed_of(&meta, &seed_folder, &[("movie.bin", &data)]).await;
    let leech = start_torrent(&meta, leech_folder.path(), 2);
    leech.start().await;
    wait_until(&leech, "the download to start", is_live).await;
    assert_eq!(leech.stats().progress_bytes, 0);

    join(&seed, &leech);
    wait_until(&leech, "the download to finish", is_done).await;

    assert_eq!(leech.stats().progress_bytes, 1_000_000);
    leech.pause().await;
    seed.pause().await;
    let saved = std::fs::read(leech_folder.path().join("movie.bin")).unwrap();
    assert!(saved == data, "the saved file differs");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_files_across_pieces_all_arrive_intact() {
    let first = sample_bytes(50_000, 2);
    let second = sample_bytes(70_001, 3);
    let third = sample_bytes(3, 4);
    let torrent = multi_file_torrent(
        "pack",
        &[
            (&["a.bin"], &first, false),
            (&["sub", "b.bin"], &second, false),
            (&["c.bin"], &third, false),
        ],
        PIECE,
    );
    let meta = meta_of(&torrent);
    let (seed_folder, leech_folder) = (TempFolder::new(), TempFolder::new());
    let seed = seed_of(
        &meta,
        &seed_folder,
        &[("a.bin", &first), ("sub/b.bin", &second), ("c.bin", &third)],
    )
    .await;
    let leech = start_torrent(&meta, leech_folder.path(), 2);
    leech.start().await;
    wait_until(&leech, "the download to start", is_live).await;
    join(&seed, &leech);
    wait_until(&leech, "the download to finish", is_done).await;
    leech.pause().await;
    let root = leech_folder.path();
    assert!(std::fs::read(root.join("a.bin")).unwrap() == first);
    assert!(std::fs::read(root.join("sub").join("b.bin")).unwrap() == second);
    assert!(std::fs::read(root.join("c.bin")).unwrap() == third);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_seed_sees_what_it_has_at_the_start() {
    let data = sample_bytes(100_000, 5);
    let meta = meta_of(&single_file_torrent("a.bin", &data, PIECE));
    let folder = TempFolder::new();
    let seed = seed_of(&meta, &folder, &[("a.bin", &data)]).await;
    assert_eq!(seed.stats().progress_bytes, 100_000);
    assert_eq!(seed.stats().downloaded_bytes, 0);
}
