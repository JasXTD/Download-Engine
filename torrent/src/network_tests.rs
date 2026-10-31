//! Engines talking over real sockets on the loopback address, including how they find each
//! other through trackers and the DHT. These only build on Linux, where they run in a
//! container with no network, because listening on Windows can bring up a firewall prompt.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bencode::{BencodeValue, ByteBuf, dyn_from_bytes};
use librqbit_core::hash_id::Id20;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::time::timeout;

use crate::meta::TorrentMeta;
use crate::session::{AddOptions, Session, SessionOptions};
use crate::testkit::{
    TempFolder, meta_of, multi_file_torrent_with, sample_bytes, single_file_torrent, write_files,
};

const PIECE: u32 = 32 * 1024;
const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;

async fn free_port() -> u16 {
    let listener = TcpListener::bind((LOOPBACK, 0)).await.unwrap();
    listener.local_addr().unwrap().port()
}

async fn wait_finished(session: &Session, name: &str) {
    let finished = timeout(Duration::from_secs(60), async {
        loop {
            let stats = session.stats();
            let entry = &stats.torrents[name];
            assert!(entry.error.is_none(), "{:?}", entry.error);
            if entry.finished {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        finished.is_ok(),
        "timed out: {:?}",
        session.stats().torrents[name]
    );
}

struct Seed {
    session: Arc<Session>,
    port: u16,
    _folder: TempFolder,
}

/// A session seeding one torrent, waiting for peers on a local port.
async fn seed_for(torrent: &[u8], data: &[u8], options: SessionOptions) -> Seed {
    let folder = TempFolder::new();
    write_files(&folder, &[("a", data)]);
    let port = free_port().await;
    let session = Session::new(SessionOptions {
        listen_port: Some(port),
        seed: true,
        ..options
    })
    .await
    .unwrap();
    let add = AddOptions {
        output: folder.path(),
        ..Default::default()
    };
    session
        .add_torrent_file("seed".into(), torrent, add)
        .await
        .unwrap();
    wait_finished(&session, "seed").await;
    Seed {
        session,
        port,
        _folder: folder,
    }
}

fn single_tracker_torrent(data: &[u8], tracker: &str) -> Vec<u8> {
    multi_file_torrent_with(
        "pack",
        &[(&["a"], data, false)],
        PIECE,
        Some(tracker),
        false,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn test_two_sessions_transfer_over_tcp() {
    let data = sample_bytes(2_000_000, 40);
    let torrent = single_file_torrent("a", &data, PIECE);
    let seed = seed_for(&torrent, &data, SessionOptions::default()).await;

    let session = Session::new(SessionOptions::default()).await.unwrap();
    let folder = TempFolder::new();
    let add = AddOptions {
        output: folder.path(),
        peers: vec![SocketAddr::from((LOOPBACK, seed.port))],
        ..Default::default()
    };
    session
        .add_torrent_file("leech".into(), &torrent, add)
        .await
        .unwrap();
    wait_finished(&session, "leech").await;
    session.shutdown().await;
    assert!(std::fs::read(folder.path().join("a")).unwrap() == data);
    assert_eq!(
        seed.session.stats().session.counters.uploaded_bytes,
        data.len() as u64
    );
    seed.session.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_magnet_link_downloads_over_tcp() {
    let data = sample_bytes(500_000, 41);
    let torrent = single_file_torrent("a", &data, PIECE);
    let info_hash = TorrentMeta::from_torrent_file(&torrent).unwrap().info_hash;
    let seed = seed_for(&torrent, &data, SessionOptions::default()).await;

    let session = Session::new(SessionOptions::default()).await.unwrap();
    let folder = TempFolder::new();
    let add = AddOptions {
        output: folder.path(),
        peers: vec![SocketAddr::from((LOOPBACK, seed.port))],
        ..Default::default()
    };
    let link = format!("magnet:?xt=urn:btih:{}", info_hash.as_string());
    session
        .add_magnet("leech".into(), &link, add)
        .await
        .unwrap();
    wait_finished(&session, "leech").await;
    session.shutdown().await;
    assert!(std::fs::read(folder.path().join("a")).unwrap() == data);
    seed.session.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_add_and_choose_later_waits_for_a_selection() {
    let first = sample_bytes(100_000, 42);
    let second = sample_bytes(100_000, 43);
    let torrent = multi_file_torrent_with(
        "pack",
        &[(&["a"], &first, false), (&["b"], &second, false)],
        PIECE,
        None,
        false,
    );
    let info_hash = TorrentMeta::from_torrent_file(&torrent).unwrap().info_hash;
    let seed_folder = TempFolder::new();
    write_files(&seed_folder, &[("a", &first), ("b", &second)]);
    let port = free_port().await;
    let seed = Session::new(SessionOptions {
        listen_port: Some(port),
        seed: true,
        ..Default::default()
    })
    .await
    .unwrap();
    let add = AddOptions {
        output: seed_folder.path(),
        ..Default::default()
    };
    seed.add_torrent_file("seed".into(), &torrent, add)
        .await
        .unwrap();
    wait_finished(&seed, "seed").await;

    let session = Session::new(SessionOptions::default()).await.unwrap();
    let folder = TempFolder::new();
    let add = AddOptions {
        output: folder.path(),
        peers: vec![SocketAddr::from((LOOPBACK, port))],
        choose_later: true,
        ..Default::default()
    };
    let link = format!("magnet:?xt=urn:btih:{}", info_hash.as_string());
    session
        .add_magnet("leech".into(), &link, add)
        .await
        .unwrap();
    let ready = timeout(Duration::from_secs(30), async {
        while session.stats().torrents["leech"].state != "ready" {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(ready.is_ok(), "{:?}", session.stats().torrents["leech"]);

    let torrent = session.torrent("leech").unwrap();
    let meta = torrent.meta().unwrap();
    assert_eq!(meta.files[1].display, "b");
    assert_eq!(seed.stats().session.counters.uploaded_bytes, 0);
    torrent.select(Some(vec![1])).unwrap();
    wait_finished(&session, "leech").await;
    session.shutdown().await;
    assert!(std::fs::read(folder.path().join("b")).unwrap() == second);
    seed.shutdown().await;
}

/// Answers every announce with the same peer, and reports the requests it saw.
async fn fake_http_tracker(peer: SocketAddrV4) -> (u16, tokio::sync::mpsc::Receiver<String>) {
    let listener = TcpListener::bind((LOOPBACK, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (seen, requests) = tokio::sync::mpsc::channel(16);
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0u8; 4096];
            let read = stream.read(&mut request).await.unwrap_or(0);
            let text = String::from_utf8_lossy(&request[..read]).into_owned();
            let _ = seen.try_send(text);
            let mut body = b"d8:intervali60e5:peers6:".to_vec();
            body.extend(peer.ip().octets());
            body.extend(peer.port().to_be_bytes());
            body.push(b'e');
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(&body).await;
        }
    });
    (port, requests)
}

async fn fake_udp_tracker(peer: SocketAddrV4) -> u16 {
    let socket = UdpSocket::bind((LOOPBACK, 0)).await.unwrap();
    let port = socket.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut buffer = [0u8; 2048];
        loop {
            let (read, from) = socket.recv_from(&mut buffer).await.unwrap();
            if read < 16 {
                continue;
            }
            let action = u32::from_be_bytes(buffer[8..12].try_into().unwrap());
            let transaction = &buffer[12..16];
            let mut reply = action.to_be_bytes().to_vec();
            reply.extend(transaction);
            if action == 0 {
                reply.extend(0x1234_5678_9ABC_DEF0_u64.to_be_bytes());
            } else {
                reply.extend(60u32.to_be_bytes());
                reply.extend(0u32.to_be_bytes());
                reply.extend(1u32.to_be_bytes());
                reply.extend(peer.ip().octets());
                reply.extend(peer.port().to_be_bytes());
            }
            socket.send_to(&reply, from).await.unwrap();
        }
    });
    port
}

async fn download_through(torrent: &[u8], options: SessionOptions, data: &[u8]) {
    let session = Session::new(options).await.unwrap();
    let folder = TempFolder::new();
    let add = AddOptions {
        output: folder.path(),
        ..Default::default()
    };
    session
        .add_torrent_file("leech".into(), torrent, add)
        .await
        .unwrap();
    wait_finished(&session, "leech").await;
    session.shutdown().await;
    assert!(std::fs::read(folder.path().join("a")).unwrap() == data);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_an_http_tracker_finds_the_seed() {
    let data = sample_bytes(300_000, 44);
    // The seed's torrent must match, so the seed port has to be known first.
    let seed_port = free_port().await;
    let seed_address = SocketAddrV4::new(LOOPBACK, seed_port);
    let (tracker_port, mut requests) = fake_http_tracker(seed_address).await;
    let url = format!("http://127.0.0.1:{tracker_port}/announce");
    let torrent = single_tracker_torrent(&data, &url);

    let folder = TempFolder::new();
    write_files(&folder, &[("a", &data)]);
    let seed = Session::new(SessionOptions {
        listen_port: Some(seed_port),
        seed: true,
        ..Default::default()
    })
    .await
    .unwrap();
    let add = AddOptions {
        output: folder.path(),
        ..Default::default()
    };
    seed.add_torrent_file("seed".into(), &torrent, add)
        .await
        .unwrap();
    wait_finished(&seed, "seed").await;

    let options = SessionOptions {
        use_trackers: true,
        allow_loopback_peers: true,
        ..Default::default()
    };
    download_through(&torrent, options, &data).await;
    let request = requests.recv().await.unwrap();
    assert!(request.starts_with("GET /announce?info_hash="), "{request}");
    assert!(
        !request.contains('@'),
        "a tracker must not get anything like an email address: {request}"
    );
    seed.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_udp_tracker_finds_the_seed() {
    let data = sample_bytes(300_000, 45);
    let seed_port = free_port().await;
    let tracker_port = fake_udp_tracker(SocketAddrV4::new(LOOPBACK, seed_port)).await;
    let url = format!("udp://127.0.0.1:{tracker_port}/announce");
    let torrent = single_tracker_torrent(&data, &url);

    let folder = TempFolder::new();
    write_files(&folder, &[("a", &data)]);
    let seed = Session::new(SessionOptions {
        listen_port: Some(seed_port),
        seed: true,
        ..Default::default()
    })
    .await
    .unwrap();
    let add = AddOptions {
        output: folder.path(),
        ..Default::default()
    };
    seed.add_torrent_file("seed".into(), &torrent, add)
        .await
        .unwrap();
    wait_finished(&seed, "seed").await;

    let options = SessionOptions {
        use_trackers: true,
        allow_loopback_peers: true,
        ..Default::default()
    };
    download_through(&torrent, options, &data).await;
    seed.shutdown().await;
}

/// A DHT node that answers like a public one. It lists itself as a node close to `id`, and
/// hands out every peer that announced to it.
struct FakeDhtNode {
    port: u16,
    queries: Arc<AtomicUsize>,
    announced: Arc<Mutex<Vec<u16>>>,
}

async fn fake_dht_node(id: Id20) -> FakeDhtNode {
    let socket = UdpSocket::bind((LOOPBACK, 0)).await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let queries = Arc::new(AtomicUsize::new(0));
    let announced = Arc::new(Mutex::new(Vec::new()));
    let node = FakeDhtNode {
        port,
        queries: queries.clone(),
        announced: announced.clone(),
    };
    tokio::spawn(async move {
        let mut buffer = [0u8; 2048];
        loop {
            let (read, from) = socket.recv_from(&mut buffer).await.unwrap();
            let Ok(BencodeValue::Dict(message)) = dyn_from_bytes::<ByteBuf>(&buffer[..read]) else {
                continue;
            };
            let (Some(BencodeValue::Bytes(transaction)), Some(BencodeValue::Bytes(query))) =
                (message.get(&ByteBuf(b"t")), message.get(&ByteBuf(b"q")))
            else {
                continue;
            };
            queries.fetch_add(1, Ordering::Relaxed);
            if query.0 == b"announce_peer"
                && let Some(BencodeValue::Dict(arguments)) = message.get(&ByteBuf(b"a"))
                && let Some(BencodeValue::Integer(peer_port)) = arguments.get(&ByteBuf(b"port"))
            {
                announced.lock().push(*peer_port as u16);
            }
            let mut body = b"d2:id20:".to_vec();
            body.extend(id.0);
            if query.0 == b"find_node" {
                body.extend(b"5:nodes26:");
                body.extend(id.0);
                body.extend(LOOPBACK.octets());
                body.extend(port.to_be_bytes());
            } else if query.0 == b"get_peers" {
                body.extend(b"5:token4:abcd6:valuesl");
                for peer_port in announced.lock().iter() {
                    body.extend(b"6:");
                    body.extend(LOOPBACK.octets());
                    body.extend(peer_port.to_be_bytes());
                }
                body.push(b'e');
            }
            body.push(b'e');
            let mut reply = b"d1:r".to_vec();
            reply.extend(body);
            reply.extend(format!("1:t{}:", transaction.0.len()).as_bytes());
            reply.extend(transaction.0);
            reply.extend(b"1:y1:re");
            let _ = socket.send_to(&reply, from).await;
        }
    });
    node
}

#[tokio::test(flavor = "multi_thread")]
async fn test_the_dht_finds_the_seed() {
    let data = sample_bytes(300_000, 46);
    let torrent = single_file_torrent("a", &data, PIECE);
    let meta = meta_of(&torrent);
    // Peers announce only to nodes close to the torrent, so this one has the hash as its id.
    let node = fake_dht_node(meta.info_hash).await;
    let on_the_dht = SessionOptions {
        use_dht: true,
        dht_bootstrap: Some(vec![format!("127.0.0.1:{}", node.port)]),
        allow_loopback_peers: true,
        ..Default::default()
    };

    let seed = seed_for(&torrent, &data, on_the_dht.clone()).await;
    let announced = timeout(Duration::from_secs(30), async {
        while node.announced.lock().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(announced.is_ok(), "the seed never announced itself");
    assert_eq!(node.announced.lock()[0], seed.port);

    download_through(&torrent, on_the_dht, &data).await;
    seed.session.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_private_torrent_stays_off_the_dht() {
    let data = sample_bytes(1000, 47);
    let torrent = multi_file_torrent_with("pack", &[(&["a"], &data, false)], PIECE, None, true);
    let meta = meta_of(&torrent);
    let node = fake_dht_node(meta.info_hash).await;
    let options = SessionOptions {
        use_dht: true,
        dht_bootstrap: Some(vec![format!("127.0.0.1:{}", node.port)]),
        allow_loopback_peers: true,
        ..Default::default()
    };
    let session = Session::new(options).await.unwrap();
    let folder = TempFolder::new();
    let add = AddOptions {
        output: folder.path(),
        ..Default::default()
    };
    session
        .add_torrent_file("private".into(), &torrent, add)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        node.queries.load(Ordering::Relaxed),
        0,
        "nothing should have contacted the DHT"
    );
    session.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_a_public_torrent_does_use_the_dht() {
    // The check above would pass if the DHT never worked at all, so this proves it can fail.
    let data = sample_bytes(1000, 48);
    let torrent = multi_file_torrent_with("pack", &[(&["a"], &data, false)], PIECE, None, false);
    let meta = meta_of(&torrent);
    let node = fake_dht_node(meta.info_hash).await;
    let options = SessionOptions {
        use_dht: true,
        dht_bootstrap: Some(vec![format!("127.0.0.1:{}", node.port)]),
        allow_loopback_peers: true,
        ..Default::default()
    };
    let session = Session::new(options).await.unwrap();
    let folder = TempFolder::new();
    let add = AddOptions {
        output: folder.path(),
        ..Default::default()
    };
    session
        .add_torrent_file("public".into(), &torrent, add)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(node.queries.load(Ordering::Relaxed) > 0);
    session.shutdown().await;
}
