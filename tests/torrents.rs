//! Torrents through the real scheduler and the real `de-torrent` helper. The only peer is a
//! small seeder in this test, on 127.0.0.1, so nothing here reaches a network.
use download_engine::{
    bandwidth,
    model::{Job, Status},
    paths::AppPaths,
    quota,
    scheduler::{Command, Scheduler},
    store::Store,
    torrent::{Details, OFF_MESSAGE, Settings, Stage},
};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const PATIENCE: Duration = Duration::from_secs(60);
/// Every look at the store opens a database, and many tests run at once.
const POLL: Duration = Duration::from_millis(100);
const PIECE: usize = 16384;
const MAIN: &str = download_engine::queue::MAIN_QUEUE;

fn text(value: &str) -> Vec<u8> {
    format!("{}:{value}", value.len()).into_bytes()
}

/// One torrent, built by hand so that this test depends on nothing in the engine.
struct Sample {
    data: Vec<u8>,
    info: Vec<u8>,
    hash: [u8; 20],
    torrent: Vec<u8>,
}

fn sha1(data: &[u8]) -> [u8; 20] {
    use sha1::{Digest, Sha1};
    Sha1::digest(data).into()
}

fn bytes_of(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2654435761) | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

fn sample(name: &str, files: &[(&str, usize)], seed: u32) -> Sample {
    let total: usize = files.iter().map(|(_, len)| len).sum();
    let data = bytes_of(total, seed);
    let pieces: Vec<u8> = data.chunks(PIECE).flat_map(sha1).collect();
    let mut info = b"d".to_vec();
    if let [(_, len)] = files
        && files[0].0 == name
    {
        info.extend(format!("6:lengthi{len}e").bytes());
    } else {
        info.extend(b"5:filesl");
        for (path, len) in files {
            info.extend(format!("d6:lengthi{len}e4:pathl").bytes());
            for part in path.split('/') {
                info.extend(text(part));
            }
            info.extend(b"ee");
        }
        info.push(b'e');
    }
    info.extend(b"4:name");
    info.extend(text(name));
    info.extend(format!("12:piece lengthi{PIECE}e6:pieces{}:", pieces.len()).bytes());
    info.extend(&pieces);
    info.push(b'e');
    let hash = sha1(&info);
    let torrent = [b"d4:info".as_slice(), &info, b"e"].concat();
    Sample {
        data,
        info,
        hash,
        torrent,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn magnet(sample: &Sample) -> String {
    format!("magnet:?xt=urn:btih:{}&dn=sample", hex(&sample.hash))
}

#[derive(Clone)]
struct Seeder {
    sample: Arc<Sample>,
    sent: Arc<AtomicU64>,
    connections: Arc<AtomicUsize>,
}

impl Seeder {
    async fn serve(self, mut peer: TcpStream) -> std::io::Result<()> {
        let mut hello = [0u8; 68];
        peer.read_exact(&mut hello).await?;
        let mut answer = hello;
        answer[20..28].copy_from_slice(&[0, 0, 0, 0, 0, 0x10, 0, 0]);
        answer[28..48].copy_from_slice(&self.sample.hash);
        answer[48..68].copy_from_slice(b"-TS0001-seeder-peer1");
        peer.write_all(&answer).await?;
        let extended = hello[25] & 0x10 != 0;
        if extended {
            let body = format!(
                "d1:md11:ut_metadatai1ee13:metadata_sizei{}ee",
                self.sample.info.len()
            );
            send(&mut peer, 20, &[&[0], body.as_bytes()].concat()).await?;
        }
        let pieces = self.sample.data.len().div_ceil(PIECE);
        let mut bits = vec![0u8; pieces.div_ceil(8)];
        for piece in 0..pieces {
            bits[piece / 8] |= 0x80 >> (piece % 8);
        }
        send(&mut peer, 5, &bits).await?;
        send(&mut peer, 1, &[]).await?;
        let mut their_id = 0;
        loop {
            let mut size = [0u8; 4];
            peer.read_exact(&mut size).await?;
            let size = u32::from_be_bytes(size) as usize;
            if size == 0 {
                continue;
            }
            let mut message = vec![0u8; size];
            peer.read_exact(&mut message).await?;
            match message[0] {
                6 => {
                    let number = |at: usize| {
                        u32::from_be_bytes(message[at..at + 4].try_into().unwrap()) as usize
                    };
                    let (piece, begin, length) = (number(1), number(5), number(9));
                    let start = piece * PIECE + begin;
                    let block = &self.sample.data[start..start + length];
                    let body = [&message[1..9], block].concat();
                    send(&mut peer, 7, &body).await?;
                    self.sent.fetch_add(length as u64, Ordering::Relaxed);
                }
                20 if message[1] == 0 => their_id = ut_metadata_id(&message[2..]),
                20 if message[1] == 1 => {
                    let header = format!(
                        "d8:msg_typei1e5:piecei0e10:total_sizei{}ee",
                        self.sample.info.len()
                    );
                    let body = [&[their_id][..], header.as_bytes(), &self.sample.info[..]].concat();
                    send(&mut peer, 20, &body).await?;
                }
                _ => {}
            }
        }
    }
}

/// The number the other side wants its metadata messages to carry.
fn ut_metadata_id(handshake: &[u8]) -> u8 {
    let key = b"11:ut_metadatai";
    let at = handshake
        .windows(key.len())
        .position(|window| window == key)
        .expect("a metadata id");
    handshake[at + key.len()..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .fold(0, |id, byte| id * 10 + (byte - b'0'))
}

async fn send(peer: &mut TcpStream, id: u8, body: &[u8]) -> std::io::Result<()> {
    let size = (body.len() as u32 + 1).to_be_bytes();
    peer.write_all(&[&size[..], &[id][..], body].concat()).await
}

struct Bench {
    _temp: tempfile::TempDir,
    store: Store,
    scheduler: Scheduler,
    seeder: Seeder,
    address: SocketAddr,
    folder: PathBuf,
    runtime: tokio::runtime::Runtime,
}

impl Bench {
    fn new(sample: Sample) -> Self {
        Self::with_settings(
            sample,
            Settings {
                enabled: true,
                dht: false,
                ..Default::default()
            },
        )
    }

    fn with_settings(sample: Sample, settings: Settings) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        })
        .unwrap();
        let proxy = store.proxy_settings().unwrap();
        store
            .save_proxy_settings(
                &proxy,
                download_engine::proxy::Settings {
                    mode: download_engine::proxy::Mode::Direct,
                    ..Default::default()
                },
                download_engine::proxy::LoginChange::Remove,
            )
            .unwrap();
        store.save_torrent_settings(settings).unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let seeder = Seeder {
            sample: Arc::new(sample),
            sent: Arc::default(),
            connections: Arc::default(),
        };
        let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let address = listener.local_addr().unwrap();
        let accepting = seeder.clone();
        runtime.spawn(async move {
            while let Ok((peer, _)) = listener.accept().await {
                accepting.connections.fetch_add(1, Ordering::Relaxed);
                let _ = peer.set_nodelay(true);
                tokio::spawn(accepting.clone().serve(peer));
            }
        });
        store.torrents.set_test_peers(vec![address]);
        let folder = temp.path().join("saved");
        std::fs::create_dir_all(&folder).unwrap();
        let scheduler = Scheduler::start(store.clone()).unwrap();
        Self {
            _temp: temp,
            store,
            scheduler,
            seeder,
            address,
            folder,
            runtime,
        }
    }

    /// Gives up on a file list that never comes, instead of waiting forever as the app would.
    fn fetch(&self, link: &str) -> Option<Details> {
        let cancel = tokio_util::sync::CancellationToken::new();
        let watchdog = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(PATIENCE);
            watchdog.cancel();
        });
        let details = self.store.fetch_torrent_details(link, &cancel).unwrap();
        assert!(!cancel.is_cancelled(), "no file list arrived");
        details
    }

    fn details(&self) -> Details {
        let path = self.folder.parent().unwrap().join("sample.torrent");
        std::fs::write(&path, &self.seeder.sample.torrent).unwrap();
        self.store.read_torrent_file(&path).unwrap()
    }

    fn add(&self, details: &Details, chosen: Vec<usize>) -> Job {
        self.store
            .add_torrent(
                details,
                chosen,
                Some(&self.folder),
                MAIN,
                uuid::Uuid::new_v4().to_string(),
            )
            .unwrap()
    }

    fn wait(&self, what: &str, done: impl Fn(&Job) -> bool, id: &str) -> Job {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let job = self.store.get(id).unwrap().unwrap();
            if done(&job) {
                return job;
            }
            assert!(
                Instant::now() < deadline,
                "{what}: {:?} {:?}",
                job.status,
                job.error
            );
            assert!(job.status != Status::Failed, "{what}: {:?}", job.error);
            std::thread::sleep(POLL);
        }
    }

    fn wait_complete(&self, id: &str) -> Job {
        self.wait(
            "the download to finish",
            |job| job.status == Status::Complete,
            id,
        )
    }

    fn wait_for_helper_to_stop(&self) {
        let deadline = Instant::now() + PATIENCE;
        while self.store.torrents.is_running() {
            assert!(Instant::now() < deadline, "the helper kept running");
            std::thread::sleep(POLL);
        }
    }

    fn leftovers(&self) -> Vec<String> {
        std::fs::read_dir(&self.folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".idm-ex-"))
            .collect()
    }

    fn state_files(&self) -> Vec<String> {
        match std::fs::read_dir(self.store.paths.data.join("torrents")) {
            Ok(entries) => entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| !name.starts_with("dht"))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Returns once the scheduler has the limit, so that later downloads start under it.
    fn set_speed_limit(&self, rate: Option<u64>) {
        self.scheduler
            .send(Command::Bandwidth(bandwidth::Update {
                request: uuid::Uuid::new_v4(),
                global: rate,
                download: None,
            }))
            .unwrap();
        while self.store.bandwidth_limits().unwrap().global != rate {
            std::thread::sleep(POLL);
        }
    }

    fn set_enabled(&self, enabled: bool) {
        let settings = self.store.torrent_settings().unwrap();
        self.store
            .save_torrent_settings(Settings {
                enabled,
                ..settings
            })
            .unwrap();
        self.scheduler.send(Command::TorrentSettings).unwrap();
    }
}

fn single() -> Sample {
    sample("one.bin", &[("one.bin", 300_000)], 1)
}

fn several() -> Sample {
    sample(
        "pack",
        &[
            ("a.bin", 100_000),
            ("sub/b.bin", 150_000),
            ("c.bin", 50_000),
        ],
        2,
    )
}

#[test]
fn torrents_start_switched_off_and_nothing_can_run_until_they_are_on() {
    let bench = Bench::with_settings(single(), Settings::default());
    assert!(!bench.store.torrent_settings().unwrap().enabled);

    let error = bench
        .store
        .read_torrent_file(Path::new("missing.torrent"))
        .unwrap_err();
    assert!(error.to_string().contains(OFF_MESSAGE), "{error}");
    let cancel = tokio_util::sync::CancellationToken::new();
    let error = bench
        .store
        .fetch_torrent_details(&magnet(&bench.seeder.sample), &cancel)
        .unwrap_err();
    assert!(error.to_string().contains(OFF_MESSAGE), "{error}");
    let error = bench
        .store
        .add_magnet_for_later(
            &magnet(&bench.seeder.sample),
            Some(&bench.folder),
            MAIN,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap_err();
    assert!(error.to_string().contains(OFF_MESSAGE), "{error}");

    assert!(
        !bench.store.torrents.is_running(),
        "the helper must not start"
    );
    assert_eq!(bench.seeder.connections.load(Ordering::Relaxed), 0);
}

#[test]
fn a_torrent_resumed_while_off_fails_without_starting_the_helper() {
    let bench = Bench::new(single());
    let details = bench.details();
    bench.wait_for_helper_to_stop();
    // A new queue doesn't run, so this download can't start before the switch goes off.
    let queue = bench.store.create_queue("Held").unwrap();
    let job = bench
        .store
        .add_torrent(
            &details,
            vec![0],
            Some(&bench.folder),
            &queue.id,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    bench.set_enabled(false);
    bench.wait("the pause", |job| job.status == Status::Paused, &job.id);
    // Nothing a script or a schedule does can get around the switch.
    bench.scheduler.send(Command::StartQueue(queue.id)).unwrap();
    bench
        .scheduler
        .send(Command::Resume(job.id.clone()))
        .unwrap();
    let deadline = Instant::now() + PATIENCE;
    let failed = loop {
        let saved = bench.store.get(&job.id).unwrap().unwrap();
        if saved.status == Status::Failed {
            break saved;
        }
        assert!(Instant::now() < deadline, "{:?}", saved.status);
        std::thread::sleep(POLL);
    };
    assert_eq!(failed.error.as_deref(), Some(OFF_MESSAGE));
    assert!(!bench.store.torrents.is_running());
    assert_eq!(bench.seeder.connections.load(Ordering::Relaxed), 0);
}

#[test]
fn a_lone_file_lands_in_the_download_folder_and_nothing_stays_behind() {
    let bench = Bench::new(single());
    let details = bench.details();
    assert_eq!(details.name, "one.bin");
    assert_eq!(details.files.len(), 1);

    let job = bench.add(&details, vec![0]);
    let finished = bench.wait_complete(&job.id);

    let saved = bench.folder.join("one.bin");
    assert_eq!(finished.saved_path.as_deref(), Some(saved.as_path()));
    assert_eq!(std::fs::read(&saved).unwrap(), bench.seeder.sample.data);
    assert_eq!(finished.downloaded, 300_000);
    assert_eq!(bench.leftovers(), Vec::<String>::new());
    bench.wait_for_helper_to_stop();
    assert_eq!(bench.state_files(), Vec::<String>::new());
}

#[test]
fn a_finished_torrent_never_replaces_a_file_that_is_already_there() {
    let bench = Bench::new(single());
    let existing = bench.folder.join("one.bin");
    std::fs::write(&existing, b"something of the user's").unwrap();
    let details = bench.details();
    let job = bench.add(&details, vec![0]);
    let finished = bench.wait_complete(&job.id);
    assert_eq!(
        std::fs::read(&existing).unwrap(),
        b"something of the user's"
    );
    let saved = bench.folder.join("one (1).bin");
    assert_eq!(finished.saved_path.as_deref(), Some(saved.as_path()));
    assert_eq!(std::fs::read(&saved).unwrap(), bench.seeder.sample.data);
}

#[test]
fn several_files_keep_their_folder_and_skipped_files_are_not_kept() {
    let bench = Bench::new(several());
    let details = bench.details();
    assert_eq!(details.files.len(), 3);
    let job = bench.add(&details, vec![1]);
    let finished = bench.wait_complete(&job.id);

    let folder = bench.folder.join("pack");
    assert_eq!(finished.saved_path.as_deref(), Some(folder.as_path()));
    let data = &bench.seeder.sample.data;
    assert_eq!(
        std::fs::read(folder.join("sub").join("b.bin")).unwrap(),
        data[100_000..250_000]
    );
    assert!(!folder.join("a.bin").exists());
    assert!(!folder.join("c.bin").exists());
    assert_eq!(bench.leftovers(), Vec::<String>::new());
}

#[test]
fn a_folder_with_the_same_name_is_left_alone() {
    let bench = Bench::new(several());
    let existing = bench.folder.join("pack");
    std::fs::create_dir(&existing).unwrap();
    std::fs::write(existing.join("mine.txt"), b"mine").unwrap();
    let details = bench.details();
    let job = bench.add(&details, vec![0, 1, 2]);
    let finished = bench.wait_complete(&job.id);
    let saved = bench.folder.join("pack (1)");
    assert_eq!(finished.saved_path.as_deref(), Some(saved.as_path()));
    assert!(saved.join("c.bin").exists());
    assert_eq!(std::fs::read_dir(&existing).unwrap().count(), 1);
}

#[test]
fn the_same_torrent_cannot_be_added_twice() {
    let bench = Bench::new(single());
    let details = bench.details();
    let job = bench.add(&details, vec![0]);
    let second = bench.store.add_torrent(
        &details,
        vec![0],
        Some(&bench.folder),
        MAIN,
        uuid::Uuid::new_v4().to_string(),
    );
    assert!(
        second
            .unwrap_err()
            .to_string()
            .contains("Already in your list")
    );
    bench.wait_complete(&job.id);
}

#[test]
fn a_magnet_link_added_for_later_waits_for_a_choice_then_downloads_only_that() {
    let bench = Bench::new(several());
    let job = bench
        .store
        .add_magnet_for_later(
            &magnet(&bench.seeder.sample),
            Some(&bench.folder),
            MAIN,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    assert_eq!(job.name, "sample");

    let waiting = bench.wait(
        "the file list",
        |job| job.torrent.as_ref().unwrap().stage == Stage::Choosing,
        &job.id,
    );
    let waiting = bench.wait("the pause", |job| job.status == Status::Paused, &waiting.id);
    assert_eq!(waiting.name, "pack");
    assert!(download_engine::torrent::waiting_for_choice(&waiting));
    bench.wait_for_helper_to_stop();
    assert_eq!(
        bench.seeder.sent.load(Ordering::Relaxed),
        0,
        "no file data yet"
    );

    // Resuming without choosing must not download everything.
    bench
        .scheduler
        .send(Command::Resume(job.id.clone()))
        .unwrap();
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(bench.seeder.sent.load(Ordering::Relaxed), 0);
    assert!(!bench.store.torrents.is_running());
    bench.wait(
        "the second pause",
        |job| job.status == Status::Paused,
        &job.id,
    );

    bench
        .scheduler
        .send(Command::ChooseTorrentFiles {
            id: job.id.clone(),
            chosen: vec![2],
        })
        .unwrap();
    let finished = bench.wait_complete(&job.id);
    let folder = finished.saved_path.unwrap();
    assert_eq!(
        std::fs::read(folder.join("c.bin")).unwrap(),
        bench.seeder.sample.data[250_000..]
    );
    assert!(!folder.join("a.bin").exists());
}

#[test]
fn the_file_list_of_a_magnet_link_comes_from_peers_and_can_be_cancelled() {
    let bench = Bench::new(several());
    let details = bench.fetch(&magnet(&bench.seeder.sample)).unwrap();
    assert_eq!(details.name, "pack");
    assert_eq!(details.files.len(), 3);
    assert_eq!(details.files[1].path, "sub/b.bin");
    bench.wait_for_helper_to_stop();

    // The details are remembered, so the second time needs no peer at all.
    bench.store.torrents.set_test_peers(Vec::new());
    assert!(bench.fetch(&magnet(&bench.seeder.sample)).is_some());
    bench.wait_for_helper_to_stop();

    let unknown = sample("other", &[("other", 40_000)], 9);
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    assert!(
        bench
            .store
            .fetch_torrent_details(&magnet(&unknown), &cancel)
            .unwrap()
            .is_none()
    );
    bench.wait_for_helper_to_stop();
}

#[test]
fn turning_torrents_off_stops_a_running_torrent_and_its_helper() {
    let bench = Bench::new(sample("big.bin", &[("big.bin", 4_000_000)], 3));
    let details = bench.details();
    bench.set_speed_limit(Some(256 * 1024));
    let job = bench.add(&details, vec![0]);
    bench.wait(
        "some progress",
        |job| job.status == Status::Downloading && job.downloaded > 0,
        &job.id,
    );

    bench.set_enabled(false);
    bench.wait("the pause", |job| job.status == Status::Paused, &job.id);
    bench.wait_for_helper_to_stop();
    let sent = bench.seeder.sent.load(Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(bench.seeder.sent.load(Ordering::Relaxed), sent);
    assert!(!bench.folder.join("big.bin").exists(), "not finished yet");
    assert_eq!(bench.leftovers().len(), 1, "progress stays for later");

    // Turning them on again picks up where it stopped.
    bench.set_enabled(true);
    bench.set_speed_limit(None);
    bench
        .scheduler
        .send(Command::Resume(job.id.clone()))
        .unwrap();
    let finished = bench.wait_complete(&job.id);
    assert_eq!(
        std::fs::read(finished.saved_path.unwrap()).unwrap(),
        bench.seeder.sample.data
    );
    assert!(
        bench.seeder.sent.load(Ordering::Relaxed) < 2 * 4_000_000,
        "resuming must not start over"
    );
}

#[test]
fn a_speed_limit_holds_a_torrent_like_any_other_download() {
    const SIZE: usize = 1_500_000;
    const RATE: u64 = 512 * 1024;
    let bench = Bench::new(sample("limited.bin", &[("limited.bin", SIZE)], 4));
    let details = bench.details();
    bench.set_speed_limit(Some(RATE));
    let started = Instant::now();
    let job = bench.add(&details, vec![0]);
    bench.wait_complete(&job.id);
    let seconds = started.elapsed().as_secs_f64();
    // The first window arrives before it is paid for, so this runs slightly over the limit.
    let unlimited = (SIZE as f64 - 128.0 * 1024.0) / RATE as f64;
    assert!(seconds > unlimited * 0.9, "{seconds}");
}

#[test]
fn a_proxy_stops_torrents_from_starting() {
    let bench = Bench::new(single());
    let details = bench.details();
    bench.wait_for_helper_to_stop();
    let proxy = bench.store.proxy_settings().unwrap();
    bench
        .store
        .save_proxy_settings(
            &proxy,
            download_engine::proxy::Settings {
                mode: download_engine::proxy::Mode::Custom,
                endpoint: "http://127.0.0.1:9".into(),
                ..Default::default()
            },
            download_engine::proxy::LoginChange::Remove,
        )
        .unwrap();
    assert!(bench.store.proxy_blocks_torrents().unwrap());
    let job = bench.add(&details, vec![0]);
    let deadline = Instant::now() + PATIENCE;
    loop {
        let saved = bench.store.get(&job.id).unwrap().unwrap();
        if saved.status == Status::Failed {
            let error = saved.error.unwrap();
            assert!(error.contains("proxy"), "{error}");
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(POLL);
    }
    assert!(!bench.store.torrents.is_running());
    assert_eq!(bench.seeder.connections.load(Ordering::Relaxed), 0);
}

#[test]
fn removing_a_paused_torrent_clears_its_temporary_folder() {
    let bench = Bench::new(sample("big.bin", &[("big.bin", 4_000_000)], 5));
    let details = bench.details();
    bench.set_speed_limit(Some(128 * 1024));
    let job = bench.add(&details, vec![0]);
    bench.wait(
        "some progress",
        |job| job.status == Status::Downloading && job.downloaded > 0,
        &job.id,
    );
    bench
        .scheduler
        .send(Command::Pause(job.id.clone()))
        .unwrap();
    bench.wait("the pause", |job| job.status == Status::Paused, &job.id);
    bench.wait_for_helper_to_stop();
    assert_eq!(bench.leftovers().len(), 1);

    bench
        .scheduler
        .send(Command::Remove(job.id.clone()))
        .unwrap();
    let deadline = Instant::now() + PATIENCE;
    while bench.store.get(&job.id).unwrap().is_some() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(POLL);
    }
    assert_eq!(bench.leftovers(), Vec::<String>::new());
    assert_eq!(bench.state_files(), Vec::<String>::new());
}

#[test]
fn deleting_a_finished_torrent_leaves_what_the_user_added_to_its_folder() {
    let bench = Bench::new(several());
    let details = bench.details();
    let job = bench.add(&details, vec![0, 1, 2]);
    let finished = bench.wait_complete(&job.id);
    let folder = finished.saved_path.clone().unwrap();
    std::fs::write(folder.join("notes.txt"), b"mine").unwrap();
    std::fs::write(folder.join("c.bin"), b"replaced by the user").unwrap();

    bench
        .scheduler
        .send(Command::DeleteFile {
            id: job.id.clone(),
            path: folder.clone(),
        })
        .unwrap();
    let deadline = Instant::now() + PATIENCE;
    while bench.store.get(&job.id).unwrap().is_some() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(POLL);
    }
    assert!(folder.join("notes.txt").exists());
    assert!(
        folder.join("c.bin").exists(),
        "it isn't the downloaded file now"
    );
    assert!(!folder.join("a.bin").exists());
    assert!(!folder.join("sub").exists());
}

#[test]
fn a_finished_torrent_in_a_synchronized_queue_does_not_break_the_list() {
    let bench = Bench::new(single());
    let details = bench.details();
    let queue = bench.store.create_sync_queue("Mirrors").unwrap();
    let job = bench
        .store
        .add_torrent(
            &details,
            vec![0],
            Some(&bench.folder),
            &queue.id,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    bench
        .scheduler
        .send(Command::StartQueue(queue.id.clone()))
        .unwrap();
    bench.wait_complete(&job.id);
    bench
        .scheduler
        .send(Command::PauseQueue(queue.id.clone()))
        .unwrap();
    bench.scheduler.send(Command::StartQueue(queue.id)).unwrap();
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(bench.store.list().unwrap().len(), 1);
    let saved = bench.store.get(&job.id).unwrap().unwrap();
    assert_eq!(saved.status, Status::Complete);
    assert!(saved.synchronization.is_none());
}

#[test]
fn a_magnet_link_cannot_choose_its_own_files() {
    let bench = Bench::new(several());
    let link = format!("{}&so=0", magnet(&bench.seeder.sample));
    let details = bench.fetch(&link).unwrap();
    assert_eq!(details.files.len(), 3);
    bench.wait_for_helper_to_stop();
    assert_eq!(bench.seeder.sent.load(Ordering::Relaxed), 0, "no file data");

    let job = bench
        .store
        .add_magnet_for_later(
            &link,
            Some(&bench.folder),
            MAIN,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    bench.wait(
        "the file list",
        |job| job.torrent.as_ref().unwrap().stage == Stage::Choosing,
        &job.id,
    );
    assert_eq!(bench.seeder.sent.load(Ordering::Relaxed), 0, "no file data");
}

#[test]
fn a_proxy_turned_on_stops_a_running_torrent() {
    let bench = Bench::new(sample("big.bin", &[("big.bin", 4_000_000)], 9));
    let details = bench.details();
    // Slow enough to outlast a busy scheduler, which can take seconds to get to the command.
    bench.set_speed_limit(Some(64 * 1024));
    let job = bench.add(&details, vec![0]);
    bench.wait(
        "some progress",
        |job| job.status == Status::Downloading && job.downloaded > 0,
        &job.id,
    );
    let proxy = bench.store.proxy_settings().unwrap();
    bench
        .store
        .save_proxy_settings(
            &proxy,
            download_engine::proxy::Settings {
                mode: download_engine::proxy::Mode::Custom,
                endpoint: "http://127.0.0.1:9".into(),
                ..Default::default()
            },
            download_engine::proxy::LoginChange::Remove,
        )
        .unwrap();
    bench.scheduler.send(Command::TorrentSettings).unwrap();
    bench.wait("the pause", |job| job.status == Status::Paused, &job.id);
    bench.wait_for_helper_to_stop();
}

#[test]
fn a_lone_file_keeps_its_own_name_inside_a_differently_named_torrent() {
    let bench = Bench::new(sample("Ubuntu 22", &[("ubuntu.iso", 100_000)], 10));
    let details = bench.details();
    assert_eq!(details.name, "Ubuntu 22");
    let job = bench.add(&details, vec![0]);
    let finished = bench.wait_complete(&job.id);
    assert_eq!(
        finished.saved_path.as_deref(),
        Some(bench.folder.join("ubuntu.iso").as_path())
    );
}

#[test]
fn a_helper_that_dies_is_replaced_and_the_download_carries_on() {
    let bench = Bench::new(sample("big.bin", &[("big.bin", 4_000_000)], 6));
    let details = bench.details();
    bench.set_speed_limit(Some(512 * 1024));
    let job = bench.add(&details, vec![0]);
    bench.wait(
        "some progress",
        |job| job.status == Status::Downloading && job.downloaded > 0,
        &job.id,
    );
    bench.store.torrents.kill_for_tests();
    bench.wait(
        "the automatic retry",
        |job| job.retry.attempts > 0 || job.status == Status::Complete,
        &job.id,
    );
    bench.set_speed_limit(None);
    let finished = bench.wait_complete(&job.id);
    assert_eq!(
        std::fs::read(finished.saved_path.unwrap()).unwrap(),
        bench.seeder.sample.data
    );
}

#[test]
fn a_data_quota_holds_a_torrent_and_counts_its_traffic() {
    let bench = Bench::new(sample("big.bin", &[("big.bin", 3_000_000)], 7));
    let details = bench.details();
    let hour = 1024 * 1024;
    bench
        .scheduler
        .send(Command::Quota(quota::Update {
            request: uuid::Uuid::new_v4(),
            limits: quota::Limits {
                hourly: Some(hour),
                daily: None,
            },
        }))
        .unwrap();
    while !bench.store.quota_status().unwrap().limits.enabled() {
        std::thread::sleep(Duration::from_millis(5));
    }
    // Slow enough for the worker's next look at the traffic to come before the end.
    bench.set_speed_limit(Some(512 * 1024));
    let job = bench.add(&details, vec![0]);
    let held = bench.wait("the quota", |job| job.quota_waiting, &job.id);
    assert_eq!(
        held.status,
        Status::Queued,
        "it continues when the quota allows"
    );
    assert!(bench.store.quota_status().unwrap().blocked());
    assert!(!bench.folder.join("big.bin").exists());
    bench.wait_for_helper_to_stop();
    assert_eq!(bench.leftovers().len(), 1, "progress is kept");
}

#[test]
fn a_magnet_link_is_not_a_download_url_for_anything_else() {
    let bench = Bench::new(single());
    let link = magnet(&bench.seeder.sample);
    assert!(Job::new(&link, None, bench.folder.clone(), "request".into()).is_err());
    assert!(
        bench
            .store
            .new_download(&link, None, None, "request".into())
            .is_err()
    );
    assert_eq!(bench.seeder.connections.load(Ordering::Relaxed), 0);
    drop(bench.runtime);
    let _ = bench.address;
}
