//! One torrent from start to finish: fetching its details if it came from a magnet link,
//! checking what is already on disk, then running the swarm until it is paused or done.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use futures_util::StreamExt;
use librqbit_core::hash_id::Id20;
use librqbit_dualstack_sockets::{ConnectOpts, tcp_connect};
use librqbit_peer_protocol::Request;
use librqbit_tracker_comms::{TorrentStatsProvider, TrackerCommsStats, TrackerCommsStatsState};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{Notify, mpsc};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Instant, MissedTickBehavior, timeout};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::bitfield::Bitfield;
use crate::debug_log;
use crate::discovery::{Discovery, Search};
use crate::meta::TorrentMeta;
use crate::metadata;
use crate::peer::{
    BoxedStream, Hello, Out, PeerLink, read_handshake, run_connection, send_handshake,
};
use crate::picker::{Chunk, Picker};
use crate::rate::{Allowance, ByteLimiter};
use crate::resume;
use crate::storage::Storage;

const MAX_PEERS: usize = 50;
/// Peers we reach out to ourselves. The rest of `MAX_PEERS` stays free for peers that
/// connect to us.
const MAX_OUTGOING: usize = 40;
/// Most addresses from trackers are dead, and each takes seconds to give up on.
const MAX_CONNECTING: usize = 16;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_METADATA_ATTEMPTS: usize = 8;
const METADATA_TIMEOUT: Duration = Duration::from_secs(60);
/// How many peers we send to at once. More would split the upload too thinly to help.
const UPLOAD_SLOTS: usize = 4;
const RECHOKE_SECONDS: u32 = 10;
/// A peer that sent part of this many bad pieces is dropped.
const MAX_STRIKES: u32 = 3;
const MAX_KNOWN_PEERS: usize = 5000;
const FIRST_RETRY: Duration = Duration::from_secs(2);
/// Tries in a row that came to nothing before an address is mostly left alone. Trackers keep
/// handing out dead addresses, and security software reports each attempt to the user.
const QUICK_TRIES: u32 = 5;
/// A peer can come back, so a long torrent still looks in on it now and then.
const RETRY_AFTER_GIVING_UP: Duration = Duration::from_secs(3600);

fn hash_workers() -> usize {
    std::thread::available_parallelism().map_or(1, |count| count.get().clamp(1, 4))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for peers to send the details of a magnet link.
    Metadata,
    /// The details are here, and the user is choosing files.
    Ready,
    /// Looking at what is already on disk.
    Checking,
    Live,
    /// Done, and not uploading.
    Finished,
    Paused,
    Error(String),
}

impl Phase {
    fn name(&self) -> &'static str {
        match self {
            Phase::Metadata => "metadata",
            Phase::Ready => "ready",
            Phase::Checking => "initializing",
            Phase::Live => "live",
            Phase::Finished => "finished",
            Phase::Paused => "paused",
            Phase::Error(_) => "error",
        }
    }
}

#[derive(Default)]
pub struct Totals {
    pub uploaded: AtomicU64,
    pub downloaded: AtomicU64,
}

/// What all torrents in a session share.
#[derive(Clone)]
pub struct Context {
    pub peer_id: Id20,
    pub up: Arc<ByteLimiter>,
    pub down: Arc<ByteLimiter>,
    pub totals: Arc<Totals>,
    pub discovery: Option<Arc<Discovery>>,
    /// Sends no torrent data to anyone, at the cost of some peers giving us less in return.
    pub no_upload: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
pub struct TorrentOptions {
    pub output: PathBuf,
    /// The files to download, or None for all of them.
    pub only: Option<Vec<usize>>,
    pub peers: Vec<SocketAddr>,
    /// Whether to keep uploading once the download is done.
    pub seed: bool,
    pub listen_port: Option<u16>,
    /// Where to keep resume data. Without it, each start checks the files again.
    pub state_dir: Option<PathBuf>,
    /// For "Add and choose later": wait for `select` once a magnet link's files are known.
    pub choose_later: bool,
    /// Whether to connect to peers. Off, the torrent only uses connections given to it.
    pub connect_out: bool,
    /// Whether peer lists from strangers may name this computer. Only for local testing.
    pub allow_loopback: bool,
    pub allowance: Arc<Allowance>,
}

#[derive(Serialize, Debug, Clone)]
pub struct TorrentStats {
    pub state: &'static str,
    pub error: Option<String>,
    pub finished: bool,
    pub progress_bytes: u64,
    pub total_bytes: u64,
    pub file_progress: Vec<u64>,
    pub downloaded_bytes: u64,
    pub uploaded_bytes: u64,
    /// Bytes that arrived but weren't needed, because another peer got there first.
    pub wasted_bytes: u64,
    pub download_bps: u64,
    pub upload_bps: u64,
    pub peers: usize,
    pub name: Option<String>,
}

#[derive(Default)]
struct Rates {
    down: f64,
    up: f64,
    last_down: u64,
    last_up: u64,
}

pub(crate) enum Event {
    PieceComplete(u32),
    Fatal(String),
    Ban(SocketAddr),
    Stream {
        stream: BoxedStream,
        addr: Option<SocketAddr>,
        hello: Option<Hello>,
    },
}

/// A torrent whose details are known.
pub(crate) struct TorrentCore {
    pub meta: Arc<TorrentMeta>,
    pub storage: Arc<Storage>,
    pub picker: Mutex<Picker>,
    pub context: Context,
    pub options: TorrentOptions,
    pub wasted: AtomicU64,
    crashed_connections: AtomicU32,
    downloaded: AtomicU64,
    uploaded: AtomicU64,
    only: Mutex<Option<Vec<usize>>>,
    checked: AtomicBool,
    live: Mutex<Option<Arc<Live>>>,
    rates: Mutex<Rates>,
}

impl TorrentCore {
    fn new(
        meta: Arc<TorrentMeta>,
        options: TorrentOptions,
        context: Context,
        only: Option<Vec<usize>>,
    ) -> Result<Arc<Self>> {
        let wanted = meta.wanted_pieces(only.as_deref())?;
        let have = Bitfield::new(meta.lengths.total_pieces());
        let picker = Picker::new(
            meta.lengths,
            have,
            wanted,
            u64::from(librqbit_core::random::random_u32()) << 16 | 1,
        );
        let storage = Arc::new(Storage::new(meta.clone(), options.output.clone()));
        Ok(Arc::new(Self {
            meta,
            storage,
            picker: Mutex::new(picker),
            context,
            options,
            wasted: AtomicU64::new(0),
            crashed_connections: AtomicU32::new(0),
            downloaded: AtomicU64::new(0),
            uploaded: AtomicU64::new(0),
            only: Mutex::new(only),
            checked: AtomicBool::new(false),
            live: Mutex::new(None),
            rates: Mutex::default(),
        }))
    }

    pub fn count_uploaded(&self, bytes: u64) {
        self.uploaded.fetch_add(bytes, Ordering::Relaxed);
        self.context
            .totals
            .uploaded
            .fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn count_downloaded(&self, bytes: u64) {
        self.downloaded.fetch_add(bytes, Ordering::Relaxed);
        self.context
            .totals
            .downloaded
            .fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn uploads_off(&self) -> bool {
        self.context.no_upload.load(Ordering::Relaxed)
    }

    pub fn request_for(&self, chunk: Chunk) -> Option<Request> {
        let lengths = &self.meta.lengths;
        let piece = lengths.validate_piece_index(chunk.piece)?;
        Some(Request::new(
            chunk.piece,
            lengths.chunk_offset_in_piece(piece, chunk.index)?,
            lengths.chunk_size(piece, chunk.index)?,
        ))
    }

    fn live(&self) -> Option<Arc<Live>> {
        self.live.lock().clone()
    }

    /// Saves which pieces we have, for a quick start next time. This needs the file times
    /// to be final, so everything goes to disk and the files are closed first.
    fn save_progress(&self) {
        let Some(state_dir) = &self.options.state_dir else {
            return;
        };
        let have = self.picker.lock().have().clone();
        let result = self.storage.sync().and_then(|()| {
            self.storage.close();
            resume::save(state_dir, &self.meta, &self.storage, &have)
        });
        if let Err(error) = result {
            debug_log!("could not save resume data: {error}");
        }
    }
}

/// Lets trackers see this torrent's progress.
struct CoreStats(Arc<TorrentCore>);

impl TorrentStatsProvider for CoreStats {
    fn get(&self) -> TrackerCommsStats {
        let core = &self.0;
        let (total, have, finished) = {
            let picker = core.picker.lock();
            let total = core.meta.bytes_in(picker.wanted().iter_set());
            let have = core.meta.bytes_in(
                picker
                    .wanted()
                    .iter_set()
                    .filter(|piece| picker.have().get(*piece)),
            );
            (total, have, picker.is_finished())
        };
        TrackerCommsStats {
            uploaded_bytes: core.uploaded.load(Ordering::Relaxed),
            downloaded_bytes: have,
            total_bytes: total,
            torrent_state: if finished || have > 0 || total > 0 {
                TrackerCommsStatsState::Live
            } else {
                TrackerCommsStatsState::Initializing
            },
        }
    }
}

/// One run of a torrent's swarm, from starting or resuming until pausing.
pub(crate) struct Live {
    pub core: Arc<TorrentCore>,
    pub cancel: CancellationToken,
    pub events: mpsc::UnboundedSender<Event>,
    found: mpsc::UnboundedSender<SocketAddr>,
    links: Mutex<HashMap<u64, Arc<PeerLink>>>,
    next_key: AtomicU64,
    connecting: AtomicUsize,
}

impl Live {
    /// Adds a connected peer, unless it is already connected or is ourselves.
    pub fn register(&self, peer_id: Id20, addr: Option<SocketAddr>) -> Option<Arc<PeerLink>> {
        if peer_id == self.core.context.peer_id {
            return None;
        }
        let mut links = self.links.lock();
        if links.values().any(|link| link.peer_id == peer_id) {
            return None;
        }
        let key = self.next_key.fetch_add(1, Ordering::Relaxed);
        let link = Arc::new(PeerLink::new(key, peer_id, addr));
        links.insert(key, link.clone());
        Some(link)
    }

    pub fn forget(&self, key: u64) {
        self.links.lock().remove(&key);
    }

    fn links(&self) -> Vec<Arc<PeerLink>> {
        self.links.lock().values().cloned().collect()
    }

    pub fn nudge_all(&self) {
        for link in self.links() {
            link.nudge();
        }
    }

    pub fn discovered(&self, peers: impl Iterator<Item = SocketAddr>) {
        for peer in peers {
            let _ = self.found.send(peer);
        }
    }

    /// Tells the other peers asked for this chunk that it has arrived.
    pub fn chunk_finished(&self, chunk: Chunk, from: u64) {
        for link in self.links() {
            if link.key != from {
                link.chunk_finished_elsewhere(chunk);
            }
        }
    }

    /// Starts uploading to a peer at once when a slot is free. Otherwise it waits its turn.
    pub fn try_unchoke(&self, link: &PeerLink) {
        if self.core.uploads_off() {
            return;
        }
        let open = self
            .links()
            .iter()
            .filter(|link| !link.is_choking())
            .count();
        if open < UPLOAD_SLOTS {
            link.unchoke();
        }
    }

    /// Decides who gets an upload slot. Peers that give us the most keep their slots,
    /// because that rewards sharing. One slot rotates, so that new peers get a chance.
    fn rechoke(&self, seen: &mut HashMap<u64, u64>, seeding: bool, turn: u32) {
        if self.core.uploads_off() {
            self.choke_all();
            return;
        }
        let links = self.links();
        let mut interested: Vec<(u64, Arc<PeerLink>)> = links
            .iter()
            .filter(|link| link.is_interested())
            .map(|link| {
                let total = if seeding {
                    link.uploaded.load(Ordering::Relaxed)
                } else {
                    link.downloaded.load(Ordering::Relaxed)
                };
                let previous = seen.insert(link.key, total).unwrap_or(0);
                (total.saturating_sub(previous), link.clone())
            })
            .collect();
        interested.sort_by_key(|(gain, link)| (std::cmp::Reverse(*gain), link.key));
        let keep = UPLOAD_SLOTS.saturating_sub(1).min(interested.len());
        let rest = interested.len() - keep;
        let rotating = (rest > 0).then(|| keep + turn as usize % rest);
        for (position, (_, link)) in interested.iter().enumerate() {
            if position < keep || Some(position) == rotating {
                link.unchoke();
            } else {
                link.choke();
            }
        }
        for link in links.iter().filter(|link| !link.is_interested()) {
            link.choke();
        }
    }

    fn choke_all(&self) {
        for link in self.links() {
            link.choke();
        }
    }

    fn piece_passed(&self, piece: u32) {
        let added = self.core.picker.lock().piece_passed(piece);
        if added {
            for link in self.links() {
                link.push(Out::Have(piece));
                link.nudge();
            }
        }
    }

    fn piece_failed(&self, piece: u32) {
        let blamed = self.core.picker.lock().piece_failed(piece);
        debug_log!("piece {piece} failed its hash check");
        let links = self.links.lock();
        for key in blamed {
            if let Some(link) = links.get(&key)
                && link.add_strike(MAX_STRIKES)
            {
                link.kick();
                if let Some(addr) = link.addr {
                    let _ = self.events.send(Event::Ban(addr));
                }
            }
        }
        drop(links);
        self.nudge_all();
    }
}

#[derive(Default)]
struct PeerEntry {
    next_try: Option<Instant>,
    failures: u32,
    connected: bool,
}

/// Every peer heard of, and when each should be tried next.
#[derive(Default)]
struct PeerBook {
    peers: HashMap<SocketAddr, PeerEntry>,
    banned: HashSet<IpAddr>,
}

fn can_reach(addr: SocketAddr) -> bool {
    let ip = addr.ip().to_canonical();
    let reserved = match ip {
        // 0.x.x.x is not an address, and 240 and above are reserved or broadcast.
        IpAddr::V4(ip) => ip.octets()[0] == 0 || ip.octets()[0] >= 240,
        IpAddr::V6(ip) => ip.is_unspecified(),
    };
    addr.port() != 0 && !reserved && !ip.is_multicast()
}

impl PeerBook {
    /// Adds a peer the user named.
    fn add(&mut self, addr: SocketAddr) {
        if can_reach(addr) && self.peers.len() < MAX_KNOWN_PEERS {
            self.peers.entry(addr).or_default();
        }
    }

    /// Adds a peer from a tracker, the DHT or another peer. Those are strangers, and they
    /// must not be able to make this computer connect to services on itself.
    fn add_found(&mut self, addr: SocketAddr, allow_loopback: bool) {
        if allow_loopback || !addr.ip().to_canonical().is_loopback() {
            self.add(addr);
        }
    }

    fn next_ready(&mut self, now: Instant) -> Option<SocketAddr> {
        let banned = &self.banned;
        let (addr, entry) = self.peers.iter_mut().find(|(addr, entry)| {
            !entry.connected
                && !banned.contains(&addr.ip())
                && entry.next_try.is_none_or(|next| next <= now)
        })?;
        entry.connected = true;
        Some(*addr)
    }

    /// Waits longer each time a peer is no use, and only briefly after one that was.
    fn ended(&mut self, addr: SocketAddr, useful: bool, now: Instant) {
        if let Some(entry) = self.peers.get_mut(&addr) {
            entry.connected = false;
            entry.failures = if useful { 0 } else { entry.failures + 1 };
            let wait = if entry.failures < QUICK_TRIES {
                FIRST_RETRY.saturating_mul(1 << entry.failures)
            } else {
                RETRY_AFTER_GIVING_UP
            };
            entry.next_try = Some(now + wait);
        }
    }

    fn ban(&mut self, addr: SocketAddr) {
        self.banned.insert(addr.ip());
    }

    fn is_banned(&self, addr: SocketAddr) -> bool {
        self.banned.contains(&addr.ip())
    }
}

struct RunHandle {
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

pub struct Torrent {
    info_hash: Id20,
    context: Context,
    options: TorrentOptions,
    trackers: Vec<Url>,
    meta: OnceLock<Arc<TorrentMeta>>,
    core: OnceLock<Arc<TorrentCore>>,
    phase: Mutex<Phase>,
    selection: Mutex<Option<Vec<usize>>>,
    selected: Notify,
    run: tokio::sync::Mutex<Option<RunHandle>>,
}

impl Torrent {
    pub fn from_meta(
        meta: Arc<TorrentMeta>,
        options: TorrentOptions,
        context: Context,
    ) -> Result<Arc<Self>> {
        meta.wanted_pieces(options.only.as_deref())?;
        let torrent = Self::blank(meta.info_hash, meta.trackers.clone(), options, context);
        let _ = torrent.meta.set(meta);
        Ok(Arc::new(torrent))
    }

    pub fn from_magnet(
        info_hash: Id20,
        trackers: Vec<Url>,
        options: TorrentOptions,
        context: Context,
    ) -> Arc<Self> {
        Arc::new(Self::blank(info_hash, trackers, options, context))
    }

    fn blank(
        info_hash: Id20,
        trackers: Vec<Url>,
        options: TorrentOptions,
        context: Context,
    ) -> Self {
        Self {
            info_hash,
            context,
            selection: Mutex::new(options.only.clone()),
            options,
            trackers,
            meta: OnceLock::new(),
            core: OnceLock::new(),
            phase: Mutex::new(Phase::Paused),
            selected: Notify::new(),
            run: tokio::sync::Mutex::new(None),
        }
    }

    pub fn info_hash(&self) -> Id20 {
        self.info_hash
    }

    /// The torrent's details, once they are known.
    pub fn meta(&self) -> Option<Arc<TorrentMeta>> {
        self.meta.get().cloned()
    }

    fn set_phase(&self, phase: Phase) {
        *self.phase.lock() = phase;
    }

    pub fn is_paused(&self) -> bool {
        *self.phase.lock() == Phase::Paused
    }

    /// Starts or resumes the torrent. Does nothing if it is already running.
    pub async fn start(self: &Arc<Self>) {
        let mut run = self.run.lock().await;
        if run
            .as_ref()
            .is_some_and(|handle| !handle.task.is_finished())
        {
            return;
        }
        // Nothing should see this torrent as paused while its task gets going.
        self.set_phase(if self.meta.get().is_some() {
            Phase::Checking
        } else {
            Phase::Metadata
        });
        let cancel = CancellationToken::new();
        let task = tokio::spawn(self.clone().lifetime(cancel.clone()));
        *run = Some(RunHandle { cancel, task });
    }

    /// Stops all traffic and waits until it has stopped. Everything is kept for resuming.
    pub async fn pause(&self) {
        let handle = self.run.lock().await.take();
        if let Some(handle) = handle {
            handle.cancel.cancel();
            let _ = handle.task.await;
        }
        let mut phase = self.phase.lock();
        if !matches!(*phase, Phase::Error(_) | Phase::Finished) {
            *phase = Phase::Paused;
        }
    }

    /// Stops sending data to this torrent's peers straight away.
    pub(crate) fn stop_uploading(&self) {
        if let Some(live) = self.core.get().and_then(|core| core.live()) {
            live.choke_all();
        }
    }

    pub fn allowance(&self) -> &Allowance {
        &self.options.allowance
    }

    /// Chooses which files to download, at any time.
    pub fn select(&self, only: Option<Vec<usize>>) -> Result<()> {
        if let Some(meta) = self.meta.get() {
            meta.wanted_pieces(only.as_deref())?;
        }
        if let Some(core) = self.core.get() {
            let wanted = core.meta.wanted_pieces(only.as_deref())?;
            core.picker.lock().set_wanted(wanted);
            core.storage
                .create_empty_files(only.as_deref())
                .context("Could not create the download files")?;
            *core.only.lock() = only.clone();
            if let Some(live) = core.live() {
                live.nudge_all();
            }
        }
        *self.selection.lock() = only;
        self.selected.notify_one();
        Ok(())
    }

    async fn lifetime(self: Arc<Self>, cancel: CancellationToken) {
        match self.run_stages(&cancel).await {
            Ok(()) => {}
            Err(error) => self.set_phase(Phase::Error(format!("{error:#}"))),
        }
    }

    async fn run_stages(self: &Arc<Self>, cancel: &CancellationToken) -> Result<()> {
        let meta = match self.meta.get() {
            Some(meta) => meta.clone(),
            None => {
                self.set_phase(Phase::Metadata);
                let Some(meta) = self.fetch_metadata(cancel).await else {
                    return Ok(());
                };
                if let Some(state_dir) = &self.options.state_dir
                    && let Err(error) = resume::save_info(state_dir, &meta)
                {
                    debug_log!("could not save the torrent's details: {error}");
                }
                let _ = self.meta.set(meta.clone());
                meta
            }
        };
        let core = match self.core.get() {
            Some(core) => core.clone(),
            None => {
                if self.options.choose_later && self.selection.lock().is_none() {
                    self.set_phase(Phase::Ready);
                    tokio::select! {
                        _ = cancel.cancelled() => return Ok(()),
                        _ = self.selected.notified() => {}
                    }
                }
                let only = self.selection.lock().clone();
                let core =
                    TorrentCore::new(meta, self.options.clone(), self.context.clone(), only)?;
                self.core.get_or_init(|| core).clone()
            }
        };
        if !core.checked.load(Ordering::Acquire) {
            self.set_phase(Phase::Checking);
            if !check_existing(&core, cancel).await? {
                return Ok(());
            }
            core.checked.store(true, Ordering::Release);
        }
        self.set_phase(Phase::Live);
        if let Some(phase) = run_swarm(core, cancel.clone()).await {
            self.set_phase(phase);
        }
        Ok(())
    }

    async fn fetch_metadata(&self, cancel: &CancellationToken) -> Option<Arc<TorrentMeta>> {
        // Ending this stage must also end the searches started for it.
        let cancel = cancel.child_token();
        let _stop_search = cancel.clone().drop_guard();
        let (found_tx, mut found) = mpsc::unbounded_channel();
        if let Some(discovery) = &self.context.discovery {
            discovery.start(Search {
                info_hash: self.info_hash,
                trackers: self.trackers.clone(),
                private: false,
                stats: Box::new(()),
                found: found_tx.clone(),
                cancel: cancel.clone(),
            });
        }
        let mut book = PeerBook::default();
        for peer in &self.options.peers {
            book.add(*peer);
        }
        let allow_loopback = self.options.allow_loopback;
        let mut attempts: JoinSet<(SocketAddr, Result<bytes::Bytes>)> = JoinSet::new();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return None,
                Some(peer) = found.recv() => book.add_found(peer, allow_loopback),
                Some(done) = attempts.join_next(), if !attempts.is_empty() => {
                    let Ok((addr, result)) = done else {
                        continue;
                    };
                    let meta = result.and_then(|info| {
                        TorrentMeta::from_info_bytes(self.info_hash, info, self.trackers.clone())
                    });
                    match meta {
                        Ok(meta) => return Some(Arc::new(meta)),
                        Err(error) => {
                            debug_log!("no metadata from {addr}: {error:#}");
                            book.ended(addr, false, Instant::now());
                        }
                    }
                }
                _ = tick.tick() => {}
            }
            while self.options.connect_out && attempts.len() < MAX_METADATA_ATTEMPTS {
                let Some(addr) = book.next_ready(Instant::now()) else {
                    break;
                };
                let (info_hash, our_id) = (self.info_hash, self.context.peer_id);
                attempts.spawn(async move {
                    let result = timeout(METADATA_TIMEOUT, async {
                        let stream = connect(addr).await?;
                        metadata::fetch(stream, info_hash, our_id).await
                    })
                    .await
                    .unwrap_or_else(|_| Err(anyhow!("timed out")));
                    (addr, result)
                });
            }
        }
    }

    /// Hands a connection from the listener to this torrent, if it is running.
    pub(crate) fn offer(&self, stream: BoxedStream, addr: SocketAddr, hello: Hello) {
        if let Some(live) = self.core.get().and_then(|core| core.live()) {
            let _ = live.events.send(Event::Stream {
                stream,
                addr: Some(addr),
                hello: Some(hello),
            });
        }
    }

    /// Connects this torrent to a peer over an existing stream. Both sides call this.
    pub fn connect_stream(&self, stream: BoxedStream) -> Result<()> {
        let live = self
            .core
            .get()
            .and_then(|core| core.live())
            .context("this torrent is not running")?;
        live.events
            .send(Event::Stream {
                stream,
                addr: None,
                hello: None,
            })
            .map_err(|_| anyhow!("this torrent is stopping"))
    }

    /// How many connections ended in a panic. Anything above zero is a bug.
    pub fn crashed_connections(&self) -> u32 {
        self.core
            .get()
            .map_or(0, |core| core.crashed_connections.load(Ordering::Relaxed))
    }

    pub fn stats(&self) -> TorrentStats {
        let phase = self.phase.lock().clone();
        let error = match &phase {
            Phase::Error(message) => Some(message.clone()),
            _ => None,
        };
        let name = self.meta.get().map(|meta| meta.name.clone());
        let Some(core) = self.core.get() else {
            return TorrentStats {
                state: phase.name(),
                error,
                finished: false,
                progress_bytes: 0,
                total_bytes: 0,
                file_progress: Vec::new(),
                downloaded_bytes: 0,
                uploaded_bytes: 0,
                wasted_bytes: 0,
                download_bps: 0,
                upload_bps: 0,
                peers: 0,
                name,
            };
        };
        let (progress_bytes, total_bytes, file_progress, finished) = {
            let picker = core.picker.lock();
            let have = picker.have();
            let progress = core
                .meta
                .bytes_in(picker.wanted().iter_set().filter(|piece| have.get(*piece)));
            let total = core.meta.bytes_in(picker.wanted().iter_set());
            (
                progress,
                total,
                core.meta.file_progress(have),
                picker.is_finished(),
            )
        };
        let rates = core.rates.lock();
        TorrentStats {
            state: phase.name(),
            error,
            // Pieces found on disk don't count as finished until checking is over.
            finished: finished && core.checked.load(Ordering::Acquire),
            progress_bytes,
            total_bytes,
            file_progress,
            downloaded_bytes: core.downloaded.load(Ordering::Relaxed),
            uploaded_bytes: core.uploaded.load(Ordering::Relaxed),
            wasted_bytes: core.wasted.load(Ordering::Relaxed),
            download_bps: rates.down as u64,
            upload_bps: rates.up as u64,
            peers: core.live().map_or(0, |live| live.links.lock().len()),
            name,
        }
    }
}

async fn connect(addr: SocketAddr) -> Result<BoxedStream> {
    // Tests on Windows run on the user's own computer, and must never reach a network.
    #[cfg(all(test, windows))]
    panic!("a test tried to connect to {addr}");
    #[cfg_attr(all(test, windows), allow(unreachable_code))]
    let stream = timeout(CONNECT_TIMEOUT, tcp_connect(addr, ConnectOpts::default()))
        .await
        .context("timed out connecting")??;
    stream.set_nodelay(true)?;
    Ok(Box::new(stream))
}

/// Finishes the handshake on a stream, then runs the connection. Returns whether the peer
/// was any use.
async fn serve_peer(
    live: Arc<Live>,
    mut stream: BoxedStream,
    addr: Option<SocketAddr>,
    hello: Option<Hello>,
) -> bool {
    let info_hash = live.core.meta.info_hash;
    let our_id = live.core.context.peer_id;
    let shaken: Result<Hello> = async {
        send_handshake(&mut stream, info_hash, our_id).await?;
        match hello {
            Some(hello) => Ok(hello),
            None => {
                let (their_hash, hello) = read_handshake(&mut stream).await?;
                anyhow::ensure!(their_hash == info_hash, "peer is on another torrent");
                Ok(hello)
            }
        }
    }
    .await;
    match shaken {
        Ok(hello) => run_connection(live, stream, hello, addr).await,
        Err(error) => {
            debug_log!("handshake with {addr:?} failed: {error:#}");
            false
        }
    }
}

async fn connect_and_serve(live: Arc<Live>, addr: SocketAddr) -> bool {
    let cancel = live.cancel.clone();
    let connected = tokio::select! {
        _ = cancel.cancelled() => return false,
        connected = connect(addr) => connected,
    };
    live.connecting.fetch_sub(1, Ordering::Relaxed);
    match connected {
        Ok(stream) => serve_peer(live, stream, Some(addr), None).await,
        Err(error) => {
            debug_log!("could not connect to {addr}: {error:#}");
            false
        }
    }
}

/// Works out what is already on disk. Returns false if it was stopped before the end.
async fn check_existing(core: &Arc<TorrentCore>, cancel: &CancellationToken) -> Result<bool> {
    let saved = match &core.options.state_dir {
        Some(state_dir) => {
            let state_dir = state_dir.clone();
            let meta = core.meta.clone();
            let storage = core.storage.clone();
            tokio::task::spawn_blocking(move || resume::load(&state_dir, &meta, &storage)).await?
        }
        None => None,
    };
    if let Some(have) = saved {
        debug_log!("resume data accepted, {} pieces", have.count());
        let mut picker = core.picker.lock();
        for piece in have.iter_set() {
            picker.piece_passed(piece);
        }
    } else if core.storage.any_file_exists() && !hash_files(core, cancel).await? {
        return Ok(false);
    }
    let only = core.only.lock().clone();
    core.storage
        .create_empty_files(only.as_deref())
        .context("Could not create the download files")?;
    Ok(true)
}

async fn hash_files(core: &Arc<TorrentCore>, cancel: &CancellationToken) -> Result<bool> {
    let pieces = core.meta.lengths.total_pieces();
    let next = Arc::new(AtomicU32::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let mut workers: JoinSet<io::Result<()>> = JoinSet::new();
    for _ in 0..hash_workers() {
        let (core, next, stop) = (core.clone(), next.clone(), stop.clone());
        workers.spawn_blocking(move || {
            while !stop.load(Ordering::Relaxed) {
                let piece = next.fetch_add(1, Ordering::Relaxed);
                if piece >= pieces {
                    break;
                }
                if core.storage.piece_matches(piece)? {
                    core.picker.lock().piece_passed(piece);
                }
            }
            Ok(())
        });
    }
    let mut stopped = false;
    loop {
        tokio::select! {
            _ = cancel.cancelled(), if !stopped => {
                stopped = true;
                stop.store(true, Ordering::Relaxed);
            }
            done = workers.join_next() => match done {
                None => return Ok(!stopped),
                Some(result) => result?.context("Could not read the download files")?,
            },
        }
    }
}

/// Runs the swarm until it is stopped. Returns the phase to move to, if a new one is due.
async fn run_swarm(core: Arc<TorrentCore>, cancel: CancellationToken) -> Option<Phase> {
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let (found_tx, mut found) = mpsc::unbounded_channel();
    let live = Arc::new(Live {
        core: core.clone(),
        cancel: cancel.clone(),
        events: events_tx,
        found: found_tx.clone(),
        links: Mutex::default(),
        next_key: AtomicU64::new(0),
        connecting: AtomicUsize::new(0),
    });
    *core.live.lock() = Some(live.clone());

    let search_cancel = cancel.child_token();
    if let Some(discovery) = &core.context.discovery {
        discovery.start(Search {
            info_hash: core.meta.info_hash,
            trackers: core.meta.trackers.clone(),
            private: core.meta.private,
            stats: Box::new(CoreStats(core.clone())),
            found: found_tx.clone(),
            cancel: search_cancel.clone(),
        });
    }

    let mut book = PeerBook::default();
    for peer in &core.options.peers {
        book.add(*peer);
    }
    let mut peers: JoinSet<(Option<SocketAddr>, bool)> = JoinSet::new();
    let mut hashing: JoinSet<(u32, io::Result<bool>)> = JoinSet::new();
    let mut waiting: VecDeque<u32> = VecDeque::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut seconds = 0u32;
    let mut uploaded_seen = HashMap::new();
    let mut finish_noted = core.picker.lock().is_finished();
    if finish_noted {
        // Everything was on disk already, so this start is also the finish.
        save_in_background(&core).await;
    }
    let mut outcome = None;

    loop {
        if finish_noted && !core.options.seed {
            outcome = Some(Phase::Finished);
            break;
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            Some(event) = events.recv() => match event {
                Event::PieceComplete(piece) => waiting.push_back(piece),
                Event::Fatal(message) => {
                    outcome = Some(Phase::Error(message));
                    break;
                }
                Event::Ban(addr) => book.ban(addr),
                Event::Stream { stream, addr, hello } => {
                    let refused = addr.is_some_and(|addr| book.is_banned(addr));
                    if peers.len() < MAX_PEERS && !refused {
                        peers.spawn(tracked(addr, serve_peer(live.clone(), stream, addr, hello)));
                    } else {
                        debug_log!("refused a connection, {} peers", peers.len());
                    }
                }
            },
            Some(done) = hashing.join_next(), if !hashing.is_empty() => {
                if let Some(message) = apply_hash(&live, done) {
                    outcome = Some(Phase::Error(message));
                    break;
                }
                let finished = core.picker.lock().is_finished();
                if finished && !finish_noted {
                    finish_noted = true;
                    save_in_background(&core).await;
                }
            }
            Some(done) = peers.join_next(), if !peers.is_empty() => {
                peer_ended(&core, &mut book, done);
            }
            Some(peer) = found.recv() => book.add_found(peer, core.options.allow_loopback),
            _ = tick.tick() => {
                seconds = seconds.wrapping_add(1);
                update_rates(&core);
                if seconds.is_multiple_of(RECHOKE_SECONDS) {
                    let seeding = core.picker.lock().is_finished();
                    live.rechoke(&mut uploaded_seen, seeding, seconds / RECHOKE_SECONDS);
                }
            }
        }
        while hashing.len() < hash_workers() {
            let Some(piece) = waiting.pop_front() else {
                break;
            };
            let storage = core.storage.clone();
            hashing.spawn_blocking(move || (piece, storage.piece_matches(piece)));
        }
        // A finished torrent waits for peers to come to it.
        if core.options.connect_out && !core.picker.lock().is_finished() {
            let now = Instant::now();
            while may_connect(peers.len(), live.connecting.load(Ordering::Relaxed)) {
                let Some(addr) = book.next_ready(now) else {
                    break;
                };
                live.connecting.fetch_add(1, Ordering::Relaxed);
                peers.spawn(tracked(Some(addr), connect_and_serve(live.clone(), addr)));
            }
        }
    }

    // Stop everything, then settle what was under way, so nothing moves after this returns.
    cancel.cancel();
    search_cancel.cancel();
    *core.live.lock() = None;
    while let Some(done) = peers.join_next().await {
        peer_ended(&core, &mut book, done);
    }
    while let Ok(event) = events.try_recv() {
        if let Event::PieceComplete(piece) = event {
            waiting.push_back(piece);
        }
    }
    while let Some(done) = hashing.join_next().await {
        apply_hash(&live, done);
    }
    for piece in waiting {
        let storage = core.storage.clone();
        let done = tokio::task::spawn_blocking(move || (piece, storage.piece_matches(piece))).await;
        apply_hash(&live, done);
    }
    update_rates(&core);
    save_in_background(&core).await;
    core.storage.close();
    outcome
}

fn may_connect(connections: usize, connecting: usize) -> bool {
    connections < MAX_OUTGOING && connecting < MAX_CONNECTING
}

fn peer_ended(
    core: &TorrentCore,
    book: &mut PeerBook,
    done: Result<(Option<SocketAddr>, bool), tokio::task::JoinError>,
) {
    match done {
        Ok((Some(addr), useful)) => book.ended(addr, useful, Instant::now()),
        Ok((None, _)) => {}
        Err(error) => {
            core.crashed_connections.fetch_add(1, Ordering::Relaxed);
            debug_log!("a peer connection crashed: {error}");
        }
    }
}

/// Pushing files to disk can take seconds, and must not hold up the other torrents.
async fn save_in_background(core: &Arc<TorrentCore>) {
    let core = core.clone();
    let _ = tokio::task::spawn_blocking(move || core.save_progress()).await;
}

async fn tracked(
    addr: Option<SocketAddr>,
    work: impl Future<Output = bool>,
) -> (Option<SocketAddr>, bool) {
    (addr, work.await)
}

fn apply_hash(
    live: &Live,
    done: Result<(u32, io::Result<bool>), tokio::task::JoinError>,
) -> Option<String> {
    match done {
        Ok((piece, Ok(true))) => live.piece_passed(piece),
        Ok((piece, Ok(false))) => live.piece_failed(piece),
        Ok((piece, Err(error))) => {
            live.piece_failed(piece);
            return Some(format!("Could not read the download files: {error}"));
        }
        Err(error) => return Some(format!("A hash check failed: {error}")),
    }
    None
}

fn update_rates(core: &TorrentCore) {
    let down = core.downloaded.load(Ordering::Relaxed);
    let up = core.uploaded.load(Ordering::Relaxed);
    let mut rates = core.rates.lock();
    let down_now = down.saturating_sub(rates.last_down) as f64;
    let up_now = up.saturating_sub(rates.last_up) as f64;
    rates.down = rates.down * 0.5 + down_now * 0.5;
    rates.up = rates.up * 0.5 + up_now * 0.5;
    rates.last_down = down;
    rates.last_up = up;
}

/// Passes on peers from a search until it ends or nobody is listening.
pub(crate) async fn drain_addresses<S>(mut stream: S, found: mpsc::UnboundedSender<SocketAddr>)
where
    S: futures_util::Stream<Item = SocketAddr> + Unpin,
{
    while let Some(addr) = stream.next().await {
        if found.send(addr).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{TempFolder, context, meta_of, options_for, single_file_torrent};

    fn address(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    #[test]
    fn test_useless_addresses_are_not_kept() {
        let mut book = PeerBook::default();
        let bad = [
            "1.2.3.4:0",
            "0.0.0.0:80",
            "0.196.171.30:48137",
            "224.0.0.1:80",
            "246.69.85.169:27338",
            "255.255.255.255:80",
            "[::]:80",
            "[ff02::1]:80",
            "[::ffff:0.1.2.3]:80",
        ];
        for address_text in bad {
            book.add(address(address_text));
        }
        assert!(book.peers.is_empty(), "{:?}", book.peers.keys());
        book.add(address("1.2.3.4:6881"));
        assert_eq!(book.peers.len(), 1);
    }

    #[test]
    fn test_strangers_cannot_point_us_at_this_computer() {
        let mut book = PeerBook::default();
        for local in [
            "127.0.0.1:80",
            "127.9.9.9:80",
            "[::1]:80",
            "[::ffff:127.0.0.1]:80",
        ] {
            book.add_found(address(local), false);
        }
        assert!(book.peers.is_empty(), "{:?}", book.peers.keys());
        book.add_found(address("1.2.3.4:6881"), false);
        book.add_found(address("127.0.0.1:80"), true);
        assert_eq!(book.peers.len(), 2);
    }

    #[test]
    fn test_the_user_may_name_a_peer_on_this_computer() {
        let mut book = PeerBook::default();
        book.add(address("127.0.0.1:6881"));
        assert_eq!(book.peers.len(), 1);
    }

    #[test]
    fn test_connection_attempts_leave_room_for_others() {
        assert!(may_connect(0, 0));
        assert!(!may_connect(0, MAX_CONNECTING), "too many under way");
        assert!(may_connect(MAX_OUTGOING - 1, MAX_CONNECTING - 1));
        assert!(
            !may_connect(MAX_OUTGOING, 0),
            "the rest is for incoming peers"
        );
        const { assert!(MAX_OUTGOING < MAX_PEERS) };
    }

    #[test]
    fn test_the_book_has_a_size_limit() {
        let mut book = PeerBook::default();
        for port in 1..=u16::MAX {
            for host in 1..=2u8 {
                book.add(SocketAddr::from(([10, 0, 0, host], port)));
            }
        }
        assert_eq!(book.peers.len(), MAX_KNOWN_PEERS);
    }

    #[test]
    fn test_a_peer_in_use_is_not_offered_twice() {
        let mut book = PeerBook::default();
        book.add(address("1.2.3.4:6881"));
        let now = Instant::now();
        assert!(book.next_ready(now).is_some());
        assert!(book.next_ready(now).is_none());
    }

    #[test]
    fn test_each_failure_waits_longer_up_to_a_limit() {
        let mut book = PeerBook::default();
        let peer = address("1.2.3.4:6881");
        book.add(peer);
        let mut now = Instant::now();
        let mut last_wait = Duration::ZERO;
        for _ in 1..QUICK_TRIES {
            assert_eq!(book.next_ready(now), Some(peer));
            book.ended(peer, false, now);
            let next = book.peers[&peer].next_try.unwrap();
            let wait = next - now;
            assert!(wait > last_wait, "{wait:?}");
            assert!(book.next_ready(now).is_none());
            last_wait = wait;
            now = next;
        }
        assert!(last_wait < Duration::from_secs(60), "{last_wait:?}");
    }

    #[test]
    fn test_a_dead_address_is_mostly_left_alone_after_a_few_tries() {
        let mut book = PeerBook::default();
        let peer = address("1.2.3.4:6881");
        book.add(peer);
        let mut now = Instant::now();
        let start = now;
        let mut tries = 0;
        while now < start + Duration::from_secs(4 * 3600) {
            assert_eq!(book.next_ready(now), Some(peer));
            book.ended(peer, false, now);
            tries += 1;
            now = book.peers[&peer].next_try.unwrap();
        }
        assert_eq!(
            tries,
            QUICK_TRIES + 3,
            "one try an hour after the first few"
        );
    }

    #[test]
    fn test_a_useful_peer_is_tried_again_soon() {
        let mut book = PeerBook::default();
        let peer = address("1.2.3.4:6881");
        book.add(peer);
        let now = Instant::now();
        book.next_ready(now);
        book.ended(peer, false, now);
        book.next_ready(book.peers[&peer].next_try.unwrap());
        book.ended(peer, true, now);
        assert_eq!(book.peers[&peer].next_try, Some(now + FIRST_RETRY));
    }

    #[test]
    fn test_a_banned_address_is_never_tried_on_any_port() {
        let mut book = PeerBook::default();
        book.add(address("1.2.3.4:6881"));
        book.add(address("1.2.3.4:6882"));
        book.ban(address("1.2.3.4:1"));
        assert!(book.is_banned(address("1.2.3.4:9999")));
        assert!(book.next_ready(Instant::now()).is_none());
    }

    fn live_for_tests(folder: &TempFolder) -> Arc<Live> {
        let meta = meta_of(&single_file_torrent("a", &[1u8; 100], 16384));
        let core = TorrentCore::new(meta, options_for(folder.path()), context(1), None).unwrap();
        let (events, _) = mpsc::unbounded_channel();
        let (found, _) = mpsc::unbounded_channel();
        Arc::new(Live {
            core,
            cancel: CancellationToken::new(),
            events,
            found,
            links: Mutex::default(),
            next_key: AtomicU64::new(0),
            connecting: AtomicUsize::new(0),
        })
    }

    #[test]
    fn test_we_do_not_connect_to_ourselves_or_twice_to_one_peer() {
        let folder = TempFolder::new();
        let live = live_for_tests(&folder);
        assert!(live.register(Id20::new([1; 20]), None).is_none());
        assert!(live.register(Id20::new([2; 20]), None).is_some());
        assert!(live.register(Id20::new([2; 20]), None).is_none());
    }

    #[test]
    fn test_the_best_peers_keep_upload_slots_and_one_slot_rotates() {
        let folder = TempFolder::new();
        let live = live_for_tests(&folder);
        let links: Vec<Arc<PeerLink>> = (0..7u8)
            .map(|index| {
                let link = live.register(Id20::new([index + 10; 20]), None).unwrap();
                link.mark_interested();
                // Peer 6 gives the most, and peer 0 the least.
                link.downloaded
                    .store(u64::from(index) * 1000 + 1, Ordering::Relaxed);
                link
            })
            .collect();
        let unchoked = |links: &[Arc<PeerLink>]| -> Vec<usize> {
            (0..links.len())
                .filter(|index| !links[*index].is_choking())
                .collect()
        };

        let mut seen = HashMap::new();
        live.rechoke(&mut seen, false, 0);
        let first = unchoked(&links);
        assert_eq!(first.len(), UPLOAD_SLOTS);
        assert!(first.ends_with(&[4, 5, 6]), "{first:?}");

        // The same gains again, on a later turn, must move the rotating slot.
        for (index, link) in links.iter().enumerate() {
            link.downloaded
                .fetch_add(index as u64 * 1000 + 1, Ordering::Relaxed);
        }
        live.rechoke(&mut seen, false, 1);
        let second = unchoked(&links);
        assert_eq!(second.len(), UPLOAD_SLOTS);
        assert!(second.ends_with(&[4, 5, 6]), "{second:?}");
        assert_ne!(first[0], second[0]);
    }

    #[test]
    fn test_a_peer_gets_a_free_slot_at_once_but_not_a_fifth() {
        let folder = TempFolder::new();
        let live = live_for_tests(&folder);
        let links: Vec<Arc<PeerLink>> = (0..5u8)
            .map(|index| live.register(Id20::new([index + 10; 20]), None).unwrap())
            .collect();
        for link in &links {
            live.try_unchoke(link);
        }
        let open = links.iter().filter(|link| !link.is_choking()).count();
        assert_eq!(open, UPLOAD_SLOTS);
    }
}
