//! Builds small torrents, throwaway folders and scripted peers for tests.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use librqbit_core::hash_id::Id20;
use librqbit_peer_protocol::{Message, Request};
use sha1w::{ISha1, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::time::timeout;

use crate::meta::TorrentMeta;
use crate::peer::{frame_len, read_handshake, send_handshake};
use crate::rate::ByteLimiter;
use crate::torrent::{Context, Torrent, TorrentOptions, TorrentStats, Totals};

pub const PATIENCE: Duration = Duration::from_secs(30);

/// A folder that is removed when the test ends.
pub struct TempFolder(PathBuf);

impl TempFolder {
    pub fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "de-torrent-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> PathBuf {
        self.0.clone()
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Bytes that differ everywhere, so a misplaced block can't pass by accident.
pub fn sample_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

fn text(value: &str) -> Vec<u8> {
    format!("{}:{value}", value.len()).into_bytes()
}

fn raw(value: &[u8]) -> Vec<u8> {
    [format!("{}:", value.len()).as_bytes(), value].concat()
}

pub fn piece_hashes(data: &[u8], piece_length: u32) -> Vec<u8> {
    data.chunks(piece_length as usize)
        .flat_map(|piece| {
            let mut hash = Sha1::new();
            hash.update(piece);
            hash.finish()
        })
        .collect()
}

fn wrap(info: Vec<u8>, announce: Option<&str>) -> Vec<u8> {
    let mut out = b"d".to_vec();
    if let Some(url) = announce {
        out.extend(b"8:announce");
        out.extend(text(url));
    }
    out.extend(b"4:info");
    out.extend(info);
    out.push(b'e');
    out
}

pub fn single_file_torrent(name: &str, data: &[u8], piece_length: u32) -> Vec<u8> {
    let mut info = format!("d6:lengthi{}e4:name", data.len()).into_bytes();
    info.extend(text(name));
    info.extend(format!("12:piece lengthi{piece_length}e6:pieces").as_bytes());
    info.extend(raw(&piece_hashes(data, piece_length)));
    info.push(b'e');
    wrap(info, None)
}

pub fn multi_file_torrent(
    name: &str,
    files: &[(&[&str], &[u8], bool)],
    piece_length: u32,
) -> Vec<u8> {
    multi_file_torrent_with(name, files, piece_length, None, false)
}

pub fn multi_file_torrent_with(
    name: &str,
    files: &[(&[&str], &[u8], bool)],
    piece_length: u32,
    announce: Option<&str>,
    private: bool,
) -> Vec<u8> {
    let joined: Vec<u8> = files.iter().flat_map(|file| file.1.to_vec()).collect();
    let mut info = b"d5:filesl".to_vec();
    for (path, data, padding) in files {
        info.push(b'd');
        if *padding {
            info.extend(b"4:attr1:p");
        }
        info.extend(format!("6:lengthi{}e4:pathl", data.len()).as_bytes());
        for part in *path {
            info.extend(text(part));
        }
        info.extend(b"ee");
    }
    info.extend(b"e4:name");
    info.extend(text(name));
    info.extend(format!("12:piece lengthi{piece_length}e6:pieces").as_bytes());
    info.extend(raw(&piece_hashes(&joined, piece_length)));
    if private {
        info.extend(b"7:privatei1e");
    }
    info.push(b'e');
    wrap(info, announce)
}

pub fn meta_of(torrent: &[u8]) -> Arc<TorrentMeta> {
    Arc::new(TorrentMeta::from_torrent_file(torrent).unwrap())
}

pub fn context(id: u8) -> Context {
    limited_context(id, None)
}

pub fn limited_context(id: u8, upload_limit: Option<u32>) -> Context {
    Context {
        peer_id: Id20::new([id; 20]),
        up: Arc::new(ByteLimiter::new(upload_limit)),
        down: Arc::new(ByteLimiter::new(None)),
        totals: Arc::new(Totals::default()),
        discovery: None,
    }
}

pub fn options_for(output: PathBuf) -> TorrentOptions {
    TorrentOptions {
        output,
        seed: true,
        ..Default::default()
    }
}

pub fn start_torrent(meta: &Arc<TorrentMeta>, output: PathBuf, id: u8) -> Arc<Torrent> {
    Torrent::from_meta(meta.clone(), options_for(output), context(id)).unwrap()
}

pub async fn wait_until(torrent: &Torrent, what: &str, done: impl Fn(&TorrentStats) -> bool) {
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

pub fn is_live(stats: &TorrentStats) -> bool {
    stats.state == "live"
}

pub fn is_done(stats: &TorrentStats) -> bool {
    stats.state == "live" && stats.finished
}

pub fn join(left: &Torrent, right: &Torrent) {
    let (one, two) = tokio::io::duplex(256 * 1024);
    left.connect_stream(Box::new(one)).unwrap();
    right.connect_stream(Box::new(two)).unwrap();
}

pub fn write_files(folder: &TempFolder, files: &[(&str, &[u8])]) {
    for (name, data) in files {
        let path = folder.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, data).unwrap();
    }
}

/// Starts a torrent over complete files, and waits until it knows they are complete.
pub async fn seed_of(
    meta: &Arc<TorrentMeta>,
    folder: &TempFolder,
    files: &[(&str, &[u8])],
) -> Arc<Torrent> {
    write_files(folder, files);
    let seed = start_torrent(meta, folder.path(), 1);
    seed.start().await;
    wait_until(&seed, "the seed to check its files", is_done).await;
    seed
}

/// A peer that does exactly what a test tells it to, including what no good peer would do.
pub struct FakePeer {
    stream: DuplexStream,
    buffer: BytesMut,
}

impl FakePeer {
    pub async fn join(torrent: &Torrent, id: u8) -> Self {
        let (one, mut two) = tokio::io::duplex(1 << 20);
        torrent.connect_stream(Box::new(one)).unwrap();
        send_handshake(&mut two, torrent.info_hash(), Id20::new([id; 20]))
            .await
            .unwrap();
        read_handshake(&mut two).await.unwrap();
        Self {
            stream: two,
            buffer: BytesMut::new(),
        }
    }

    pub async fn send(&mut self, message: Message<'_>) {
        let mut out = vec![0u8; 256 * 1024];
        let len = message.serialize(&mut out, &Default::default).unwrap();
        self.send_raw(&out[..len]).await;
    }

    pub async fn send_raw(&mut self, bytes: &[u8]) {
        // The engine may already have hung up, and tests check for that separately.
        let _ = self.stream.write_all(bytes).await;
    }

    /// The next whole message from the engine, or None once it has hung up.
    pub async fn next_frame(&mut self, wait: Duration) -> Option<Bytes> {
        loop {
            if let Some(len) = frame_len(&self.buffer, 1 << 24).unwrap() {
                return Some(self.buffer.split_to(len).freeze());
            }
            match timeout(wait, self.stream.read_buf(&mut self.buffer)).await {
                Ok(Ok(count)) if count > 0 => {}
                _ => return None,
            }
        }
    }

    /// The next block request from the engine, skipping everything else.
    pub async fn next_request(&mut self) -> Option<Request> {
        while let Some(frame) = self.next_frame(PATIENCE).await {
            if let Ok((Message::Request(request), _)) = Message::deserialize(&frame, &[]) {
                return Some(request);
            }
        }
        None
    }

    /// Waits for the engine to say one thing, skipping everything else.
    pub async fn wait_for(&mut self, wanted: impl Fn(&Message<'_>) -> bool) -> bool {
        while let Some(frame) = self.next_frame(PATIENCE).await {
            if let Ok((message, _)) = Message::deserialize(&frame, &[])
                && wanted(&message)
            {
                return true;
            }
        }
        false
    }

    /// Whether the engine hangs up, reading all it says until then.
    pub async fn is_dropped(&mut self) -> bool {
        let dropped = timeout(Duration::from_secs(10), async {
            loop {
                self.buffer.reserve(64 * 1024);
                match self.stream.read_buf(&mut self.buffer).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => self.buffer.clear(),
                }
            }
        })
        .await;
        dropped.is_ok()
    }
}
