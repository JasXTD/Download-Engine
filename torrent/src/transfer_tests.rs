//! Whole transfers between two engines, joined by in-memory pipes. No sockets are opened,
//! so these run anywhere and can't reach a network.

use crate::testkit::{
    TempFolder, is_done, is_live, join, meta_of, multi_file_torrent, sample_bytes, seed_of,
    single_file_torrent, start_torrent, wait_until,
};

const PIECE: u32 = 32 * 1024;

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
