//! All running torrents, and what they share.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use librqbit_core::magnet::Magnet;
use librqbit_core::torrent_limits::{MAX_TRACKER_URL_BYTES, MAX_TRACKERS};
use librqbit_dualstack_sockets::{BindOpts, TcpListener};
use parking_lot::Mutex;
use serde::Serialize;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::admission::Admission;
use crate::debug_log;
use crate::discovery::{Discovery, DiscoveryOptions};
use crate::meta::{TorrentMeta, usable_tracker};
use crate::mse::{self, Encryption};
use crate::peer::read_handshake;
use crate::rate::ByteLimiter;
use crate::reason::{self, Reason};
use crate::resume;
use crate::router::{self, RouterState};
use crate::torrent::{Context, Torrent, TorrentOptions, TorrentStats, Totals};
use crate::transport::Peers;

/// Connections that haven't said which torrent they want yet.
const MAX_PENDING_HANDSHAKES: usize = 32;

#[derive(Clone, Default)]
pub struct SessionOptions {
    pub binding: Option<crate::network::Binding>,
    pub encryption: Encryption,
    pub use_utp: bool,
    pub web_seeds: bool,
    /// Incoming connections stay off unless a port is set.
    pub listen_port: Option<u16>,
    pub upload_limit: Option<u32>,
    pub download_limit: Option<u32>,
    pub seed: bool,
    pub use_trackers: bool,
    pub use_dht: bool,
    /// The DHT's first contacts, or None for the public ones.
    pub dht_bootstrap: Option<Vec<String>>,
    pub state_dir: Option<PathBuf>,
    /// Whether peer lists from strangers may name this computer. Only for local testing.
    pub allow_loopback_peers: bool,
    /// Sends no torrent data to anyone.
    pub no_upload: bool,
    /// Asks the router to forward `listen_port` while the session runs. Does nothing
    /// without a port. See `router`.
    pub upnp: bool,
}

/// Router setup while it runs.
struct RouterTask {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    state: Arc<Mutex<RouterState>>,
}

/// Removing the rule takes two short requests. Past this, the rule expires by itself.
const ROUTER_STOP_WAIT: Duration = Duration::from_secs(10);

fn start_router(port: u16) -> RouterTask {
    let state = Arc::new(Mutex::new(RouterState::Looking));
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let (state, cancel) = (state.clone(), cancel.clone());
        async move {
            let Some(gateway) = router::default_gateway() else {
                *state.lock() = RouterState::Unavailable;
                return;
            };
            match router::Network::new() {
                Ok(network) => router::keep_open(network, gateway, port, state, cancel).await,
                Err(error) => {
                    debug_log!("router setup could not start: {error:#}");
                    *state.lock() = RouterState::Failed;
                }
            }
        }
    });
    RouterTask {
        cancel,
        task,
        state,
    }
}

async fn stop_router(router: RouterTask) {
    router.cancel.cancel();
    if tokio::time::timeout(ROUTER_STOP_WAIT, router.task)
        .await
        .is_err()
    {
        debug_log!("the router did not answer in time; its rule expires by itself");
    }
}

#[derive(Clone, Default)]
pub struct AddOptions {
    pub output: PathBuf,
    pub only: Option<Vec<usize>>,
    pub peers: Vec<SocketAddr>,
    pub choose_later: bool,
    pub seed_only: bool,
    pub file_name: Option<String>,
    /// Trackers the user added earlier. Ones that aren't usable are left out.
    pub trackers: Vec<String>,
    /// The person said yes to looking this magnet up on the DHT. Only used when nothing
    /// else could find it, and only while the DHT is on.
    pub dht_search: bool,
}

/// What a magnet link may use to find its details.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MagnetDiscovery {
    /// Its trackers, saved details or given peers, as always.
    AsToday,
    /// Only a DHT search, which announces nothing until the details say public.
    DhtSearchOnly,
    Refuse(Reason),
}

/// One decision for every magnet. An unknown hash reaches the DHT only with a yes for this
/// magnet, and never while the DHT is off.
pub fn magnet_discovery(
    usable_trackers: usize,
    cached: bool,
    given_peers: bool,
    dht_on: bool,
    consent: bool,
) -> MagnetDiscovery {
    if usable_trackers > 0 || cached || given_peers {
        return MagnetDiscovery::AsToday;
    }
    match (dht_on, consent) {
        (false, _) => MagnetDiscovery::Refuse(reason::DHT_OFF),
        (true, false) => MagnetDiscovery::Refuse(reason::NEEDS_DHT_CONSENT),
        (true, true) => MagnetDiscovery::DhtSearchOnly,
    }
}

#[derive(Serialize)]
pub struct Counters {
    pub uploaded_bytes: u64,
    pub downloaded_bytes: u64,
}

#[derive(Serialize)]
pub struct SessionTotals {
    pub counters: Counters,
}

#[derive(Serialize)]
pub struct SessionStats {
    pub torrents: BTreeMap<String, TorrentStats>,
    pub managed: usize,
    pub session: SessionTotals,
    /// Router setup, while it is on.
    pub router: Option<RouterState>,
}

pub struct Session {
    options: SessionOptions,
    context: Context,
    torrents: Mutex<BTreeMap<String, Arc<Torrent>>>,
    cancel: CancellationToken,
    listener: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    utp_listener: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    pending: Arc<Admission>,
    handshakes: tokio_util::task::TaskTracker,
    router: Mutex<Option<RouterTask>>,
}

impl Session {
    pub async fn new(options: SessionOptions) -> Result<Arc<Self>> {
        let cancel = CancellationToken::new();
        let startup = cancel.clone().drop_guard();
        let binding = options
            .binding
            .as_ref()
            .map(crate::network::Binding::open)
            .transpose()?
            .map(Arc::new);
        anyhow::ensure!(
            binding.is_none() || !options.upnp,
            "Router port mapping is unavailable while a torrent interface is bound"
        );
        let peer_id = librqbit_core::peer_id::generate_azereus_style(*b"DE", (0, 1, 0, 0));
        let discovery = if options.use_trackers || options.use_dht {
            Some(Discovery::new(
                DiscoveryOptions {
                    binding: binding.clone(),
                    peer_id,
                    listen_port: options.listen_port,
                    use_trackers: options.use_trackers,
                    use_dht: options.use_dht,
                    dht_bootstrap: options.dht_bootstrap.clone(),
                    state_dir: options.state_dir.clone(),
                    allow_local_trackers: options.allow_loopback_peers,
                },
                cancel.clone(),
            )?)
        } else {
            None
        };
        let context = Context {
            peers: Peers::new(
                options.encryption,
                options.use_utp,
                options.listen_port,
                options.allow_loopback_peers,
                cancel.clone(),
                binding.clone(),
            )
            .await?,
            peer_id,
            up: Arc::new(ByteLimiter::new(options.upload_limit)),
            down: Arc::new(ByteLimiter::new(options.download_limit)),
            totals: Arc::new(Totals::default()),
            discovery,
            no_upload: Arc::new(AtomicBool::new(options.no_upload)),
        };
        let session = Arc::new(Self {
            options,
            context,
            torrents: Mutex::default(),
            cancel,
            listener: tokio::sync::Mutex::new(None),
            utp_listener: tokio::sync::Mutex::new(None),
            pending: Admission::new(MAX_PENDING_HANDSHAKES),
            handshakes: tokio_util::task::TaskTracker::new(),
            router: Mutex::new(None),
        });
        if let Some(port) = session.options.listen_port {
            let listener = match bind(port, binding.as_deref()) {
                Ok(listener) => listener,
                Err(error) => {
                    session.context.peers.shutdown().await;
                    return Err(error);
                }
            };
            *session.listener.lock().await = Some(tokio::spawn(session.clone().listen(listener)));
            if session.options.use_utp {
                *session.utp_listener.lock().await =
                    Some(tokio::spawn(session.clone().listen_utp()));
            }
            // Only once something listens, and only when the person turned it on.
            if session.options.upnp {
                *session.router.lock() = Some(start_router(port));
            }
        }
        startup.disarm();
        Ok(session)
    }

    /// Turns router setup on or off while the session runs. Turning it off waits until the
    /// rule is removed, or until the router had its chance to answer.
    pub async fn set_upnp(&self, on: bool) {
        if on && self.options.binding.is_some() {
            return;
        }
        let stopping = {
            let mut router = self.router.lock();
            match (on, router.is_some(), self.options.listen_port) {
                (true, false, Some(port)) if !self.cancel.is_cancelled() => {
                    *router = Some(start_router(port));
                    None
                }
                (false, true, _) => router.take(),
                _ => None,
            }
        };
        if let Some(router) = stopping {
            stop_router(router).await;
        }
    }

    /// What router setup has come to, while it is on.
    pub fn router_status(&self) -> Option<RouterState> {
        self.router
            .lock()
            .as_ref()
            .map(|router| *router.state.lock())
    }

    /// The JSON helper exits after joining its transfers on interface loss. A retry
    /// starts a new snapshot; the old session never switches to a different adapter.
    pub async fn wait_for_interface_loss(&self) {
        let Some(device) = self.context.peers.binding.as_ref() else {
            std::future::pending::<()>().await;
            return;
        };
        loop {
            if device.check().is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    fn torrent_options(&self, add: AddOptions) -> TorrentOptions {
        let trackers = add
            .trackers
            .iter()
            .filter(|text| text.len() <= MAX_TRACKER_URL_BYTES)
            .filter_map(|text| usable_tracker(text.trim()))
            .take(MAX_TRACKERS)
            .collect();
        TorrentOptions {
            web_seeds: self.options.web_seeds,
            output: add.output,
            only: add.only,
            peers: add.peers,
            seed: self.options.seed || add.seed_only,
            seed_only: add.seed_only,
            file_name: add.file_name,
            listen_port: self.options.listen_port,
            state_dir: if add.seed_only {
                None
            } else {
                self.options.state_dir.clone()
            },
            choose_later: add.choose_later,
            connect_out: true,
            allow_loopback: self.options.allow_loopback_peers,
            allowance: Arc::default(),
            trackers,
            // Only add_magnet turns this on, after its own check.
            dht_search: false,
        }
    }

    fn insert(&self, name: String, torrent: Arc<Torrent>) -> Result<Arc<Torrent>> {
        let mut torrents = self.torrents.lock();
        if torrents.contains_key(&name) {
            bail!("torrent {name} was already added");
        }
        if torrents
            .values()
            .any(|other| other.info_hash() == torrent.info_hash())
        {
            bail!("this torrent was already added");
        }
        torrents.insert(name, torrent.clone());
        Ok(torrent)
    }

    /// Reads a .torrent file and remembers its details, so that a magnet link for it needs
    /// nothing from the file or from strangers afterwards.
    pub fn import(&self, bytes: &[u8]) -> Result<TorrentMeta> {
        let meta = TorrentMeta::from_torrent_file(bytes)?;
        if let Some(state_dir) = &self.options.state_dir {
            resume::save_info(state_dir, &meta).context("Could not save the torrent's details")?;
        }
        Ok(meta)
    }

    /// Adds a torrent from the bytes of a .torrent file and starts it.
    pub async fn add_torrent_file(
        &self,
        name: String,
        bytes: &[u8],
        add: AddOptions,
    ) -> Result<String> {
        let meta = Arc::new(TorrentMeta::from_torrent_file(bytes)?);
        let options = self.torrent_options(add);
        if let Some(state_dir) = &options.state_dir {
            let meta = meta.clone();
            let state_dir = state_dir.clone();
            tokio::task::spawn_blocking(move || resume::save_info(&state_dir, &meta)).await??;
        }
        let torrent = Torrent::from_meta(meta, options, self.context.clone())?;
        let torrent = self.insert(name, torrent)?;
        torrent.start().await;
        Ok(torrent.info_hash().as_string())
    }

    /// Adds a magnet link and starts fetching its details.
    pub async fn add_magnet(&self, name: String, link: &str, add: AddOptions) -> Result<String> {
        let magnet = Magnet::parse(link).context("This is not a valid magnet link")?;
        let info_hash = magnet
            .as_id20()
            .context("This magnet link has no version 1 hash")?;
        let trackers: Vec<Url> = magnet
            .trackers
            .iter()
            .filter_map(|tracker| usable_tracker(tracker))
            .collect();
        // A link's own `so=` list is ignored, because a stranger's link must not choose
        // what gets downloaded.
        let consent = add.dht_search;
        let mut options = self.torrent_options(add);
        let saved = self
            .options
            .state_dir
            .as_deref()
            .and_then(|state_dir| resume::load_info(state_dir, info_hash, trackers.clone()));
        anyhow::ensure!(
            !options.seed_only || saved.is_some(),
            "Seeding needs saved metadata. Open the original .torrent file again."
        );
        // Only trackers that would really be asked count. One that leads to this computer or
        // its network is refused later, so it can't stand in for a yes to the DHT.
        let usable = match &self.context.discovery {
            Some(discovery) if discovery.uses_trackers() => trackers
                .iter()
                .filter(|tracker| discovery.allows_tracker(tracker))
                .count(),
            _ => 0,
        };
        match magnet_discovery(
            usable,
            saved.is_some(),
            !options.peers.is_empty(),
            self.options.use_dht,
            consent,
        ) {
            MagnetDiscovery::AsToday => {}
            MagnetDiscovery::DhtSearchOnly => options.dht_search = true,
            MagnetDiscovery::Refuse(reason) => return Err(reason.into()),
        }
        let torrent = match saved {
            Some(meta) => Torrent::from_meta(Arc::new(meta), options, self.context.clone())?,
            None => Torrent::from_magnet(info_hash, trackers, options, self.context.clone()),
        };
        let torrent = self.insert(name, torrent)?;
        torrent.start().await;
        Ok(torrent.info_hash().as_string())
    }

    pub fn torrent(&self, name: &str) -> Result<Arc<Torrent>> {
        self.torrents
            .lock()
            .get(name)
            .cloned()
            .with_context(|| format!("unknown torrent {name}"))
    }

    fn all(&self) -> Vec<(String, Arc<Torrent>)> {
        self.torrents
            .lock()
            .iter()
            .map(|(name, torrent)| (name.clone(), torrent.clone()))
            .collect()
    }

    /// Stops a torrent and lets go of its files, which stay on disk.
    pub async fn remove(&self, name: &str) -> Result<()> {
        self.torrent(name)?.pause().await;
        self.torrents.lock().remove(name);
        Ok(())
    }

    /// Changes how much is uploaded, for every torrent and straight away.
    pub fn configure(&self, upload_limit: Option<u32>, no_upload: bool) {
        self.context.up.set_limit(upload_limit);
        self.context.no_upload.store(no_upload, Ordering::Relaxed);
        if no_upload {
            for (_, torrent) in self.all() {
                torrent.stop_uploading();
            }
        }
    }

    /// Lets a torrent receive up to `until` bytes in all, or any amount with None.
    /// Returns how many it has received so far.
    pub fn allow(&self, name: &str, until: Option<u64>) -> Result<u64> {
        Ok(self.torrent(name)?.allowance().allow(until))
    }

    pub async fn pause(&self, name: &str) -> Result<()> {
        self.torrent(name)?.pause().await;
        Ok(())
    }

    pub async fn resume(&self, name: &str) -> Result<()> {
        self.torrent(name)?.start().await;
        Ok(())
    }

    /// Pauses every running torrent and names them.
    pub async fn stop_all(&self) -> Vec<String> {
        let mut stopped = Vec::new();
        for (name, torrent) in self.all() {
            if !torrent.is_paused() {
                torrent.pause().await;
                stopped.push(name);
            }
        }
        stopped
    }

    /// Resumes every paused torrent and names them.
    pub async fn resume_all(&self) -> Vec<String> {
        let mut resumed = Vec::new();
        for (name, torrent) in self.all() {
            if torrent.is_paused() {
                torrent.start().await;
                resumed.push(name);
            }
        }
        resumed
    }

    pub fn stats(&self) -> SessionStats {
        let torrents: BTreeMap<String, TorrentStats> = self
            .all()
            .into_iter()
            .map(|(name, torrent)| (name, torrent.stats()))
            .collect();
        SessionStats {
            managed: torrents.len(),
            torrents,
            session: SessionTotals {
                counters: Counters {
                    uploaded_bytes: self.context.totals.uploaded.load(Ordering::Relaxed),
                    downloaded_bytes: self.context.totals.downloaded.load(Ordering::Relaxed),
                },
            },
            router: self.router_status(),
        }
    }

    /// Stops everything and saves progress.
    pub async fn shutdown(&self) {
        for (_, torrent) in self.all() {
            torrent.pause().await;
        }
        self.cancel.cancel();
        // Keep the handle until it finishes, even if this shutdown is cancelled.
        // Concurrent/repeated callers must all wait until the socket is closed.
        let mut listener = self.listener.lock().await;
        if let Some(task) = listener.as_mut() {
            if let Err(error) = task.await {
                debug_log!("listener task ended unexpectedly: {error}");
            }
            *listener = None;
        }
        let mut utp_listener = self.utp_listener.lock().await;
        if let Some(task) = utp_listener.as_mut() {
            if let Err(error) = task.await {
                debug_log!("uTP listener ended unexpectedly: {error}");
            }
            *utp_listener = None;
        }
        self.handshakes.close();
        self.handshakes.wait().await;
        self.context.peers.shutdown().await;
        // The port is closed first, so a new helper can use it while the rule goes.
        let router = self.router.lock().take();
        if let Some(router) = router {
            stop_router(router).await;
        }
    }

    async fn listen(self: Arc<Self>, listener: TcpListener) {
        loop {
            let accepted = tokio::select! {
                _ = self.cancel.cancelled() => return,
                accepted = listener.accept() => accepted,
            };
            let (stream, addr) = match accepted {
                Ok(connection) => connection,
                Err(error) => {
                    debug_log!("accept failed: {error}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            // When strangers are this busy, dropping one costs us nothing.
            let Some(permit) = self.pending.try_acquire(Some(addr)) else {
                continue;
            };
            let session = self.clone();
            self.handshakes.spawn(async move {
                let _permit = permit;
                if stream.set_nodelay(true).is_err() {
                    return;
                }
                let result = tokio::select! {
                    _ = session.cancel.cancelled() => return,
                    result = session.accept(Box::new(stream), addr, crate::details::PeerTransport::Tcp) => result,
                };
                if let Err(error) = result {
                    debug_log!("incoming peer {addr}: {error:#}");
                }
            });
        }
    }

    async fn listen_utp(self: Arc<Self>) {
        loop {
            let accepted = tokio::select! {
                _ = self.cancel.cancelled() => return,
                result = self.context.peers.accept_utp() => result,
            };
            let (stream, addr) = match accepted {
                Ok(value) => value,
                Err(error) => {
                    debug_log!("uTP listener stopped: {error:#}");
                    return;
                }
            };
            let Some(permit) = self.pending.try_acquire(Some(addr)) else {
                continue;
            };
            let session = self.clone();
            self.handshakes.spawn(async move {
                let _permit = permit;
                tokio::select! {
                    _ = session.cancel.cancelled() => {},
                    result = session.accept(stream, addr, crate::details::PeerTransport::Utp) => {
                        if let Err(error) = result { debug_log!("incoming uTP peer failed: {error:#}"); }
                    },
                }
            });
        }
    }

    async fn accept(
        &self,
        stream: crate::peer::BoxedStream,
        addr: SocketAddr,
        transport: crate::details::PeerTransport,
    ) -> Result<()> {
        let hashes = self
            .all()
            .into_iter()
            .map(|(_, torrent)| torrent.info_hash())
            .collect();
        let (mut stream, encrypted_hash) =
            mse::incoming(stream, self.options.encryption, hashes).await?;
        let (info_hash, mut hello) = read_handshake(&mut stream).await?;
        hello.connection = crate::details::ConnectionInfo {
            transport,
            encrypted: Some(encrypted_hash.is_some()),
        };
        anyhow::ensure!(
            encrypted_hash.is_none_or(|hash| hash == info_hash),
            "Peer changed torrent after encryption negotiation"
        );
        let torrent = self
            .all()
            .into_iter()
            .map(|(_, torrent)| torrent)
            .find(|torrent| torrent.info_hash() == info_hash)
            .context("peer asked for a torrent we don't have")?;
        torrent.offer(stream, addr, hello)
    }
}

fn bind(port: u16, device: Option<&librqbit_dualstack_sockets::BindDevice>) -> Result<TcpListener> {
    let both = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port);
    TcpListener::bind_tcp(
        both,
        BindOpts {
            device,
            ..Default::default()
        },
    )
    .or_else(|_| {
        let only_v4 = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
        let options = BindOpts {
            device,
            request_dualstack: false,
            ..Default::default()
        };
        TcpListener::bind_tcp(only_v4, options)
            .with_context(|| format!("Could not listen on port {port}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{TempFolder, meta_of, sample_bytes, single_file_torrent, wait_until};

    const PIECE: u32 = 16384;

    fn session_options(state_dir: PathBuf) -> SessionOptions {
        SessionOptions {
            state_dir: Some(state_dir),
            ..Default::default()
        }
    }

    fn add_options(output: PathBuf) -> AddOptions {
        AddOptions {
            output,
            ..Default::default()
        }
    }

    fn magnet_for(meta: &TorrentMeta) -> String {
        format!("magnet:?xt=urn:btih:{}", meta.info_hash.as_string())
    }

    #[tokio::test]
    async fn test_a_removed_torrent_can_be_added_again() {
        let (state, output) = (TempFolder::new(), TempFolder::new());
        let data = sample_bytes(40_000, 40);
        let file = single_file_torrent("a.bin", &data, PIECE);
        let session = Session::new(session_options(state.path())).await.unwrap();
        session
            .add_torrent_file("one".into(), &file, add_options(output.path()))
            .await
            .unwrap();
        assert!(
            session
                .add_torrent_file("two".into(), &file, add_options(output.path()))
                .await
                .is_err(),
            "it is already here"
        );
        session.remove("one").await.unwrap();
        assert!(session.torrent("one").is_err());
        session
            .add_torrent_file("two".into(), &file, add_options(output.path()))
            .await
            .unwrap();
        session.shutdown().await;
    }

    #[tokio::test]
    async fn test_removing_an_unknown_torrent_is_an_error() {
        let state = TempFolder::new();
        let session = Session::new(session_options(state.path())).await.unwrap();
        assert!(session.remove("nothing").await.is_err());
    }

    #[tokio::test]
    async fn test_a_magnet_link_starts_from_imported_details_without_any_peer() {
        let (state, output) = (TempFolder::new(), TempFolder::new());
        let data = sample_bytes(40_000, 41);
        let file = single_file_torrent("a.bin", &data, PIECE);
        let meta = meta_of(&file);
        let session = Session::new(session_options(state.path())).await.unwrap();
        session.import(&file).unwrap();
        session
            .add_magnet(
                "one".into(),
                &magnet_for(&meta),
                AddOptions {
                    output: output.path(),
                    choose_later: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let torrent = session.torrent("one").unwrap();
        wait_until(&torrent, "the file list", |stats| stats.state == "ready").await;
        assert_eq!(torrent.meta().unwrap().files[0].len, 40_000);
        session.shutdown().await;
    }

    #[tokio::test]
    async fn test_a_magnet_link_cannot_choose_its_own_files() {
        let (state, output) = (TempFolder::new(), TempFolder::new());
        let file = single_file_torrent("a.bin", &sample_bytes(40_000, 44), PIECE);
        let meta = meta_of(&file);
        let session = Session::new(session_options(state.path())).await.unwrap();
        session.import(&file).unwrap();
        session
            .add_magnet(
                "one".into(),
                &format!("{}&so=0", magnet_for(&meta)),
                AddOptions {
                    output: output.path(),
                    choose_later: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let torrent = session.torrent("one").unwrap();
        wait_until(&torrent, "the file list", |stats| stats.state == "ready").await;
        session.shutdown().await;
    }

    #[tokio::test]
    async fn test_damaged_saved_details_are_not_believed() {
        let (state, output) = (TempFolder::new(), TempFolder::new());
        let file = single_file_torrent("a.bin", &sample_bytes(40_000, 42), PIECE);
        let meta = meta_of(&file);
        let session = Session::new(session_options(state.path())).await.unwrap();
        session.import(&file).unwrap();
        let info = state
            .path()
            .join(format!("{}.info", meta.info_hash.as_string()));
        let mut bytes = std::fs::read(&info).unwrap();
        bytes[10] ^= 1;
        std::fs::write(&info, bytes).unwrap();
        let error = session
            .add_magnet("one".into(), &magnet_for(&meta), add_options(output.path()))
            .await
            .unwrap_err();
        assert!(error.to_string().contains(".torrent file"));
        // Damaged details count as none, so the magnet is as unknown as one never seen.
        assert_eq!(reason::code_of(&error), Some("dht_off"));
        assert!(session.torrent("one").is_err());
        session.shutdown().await;
    }

    #[test]
    fn every_magnet_case_has_one_decision() {
        use MagnetDiscovery::*;
        // Trackers, saved details or given peers: as always, whatever the DHT and the yes.
        for (trackers, cached, peers) in [(1, false, false), (0, true, false), (0, false, true)] {
            for dht in [false, true] {
                for consent in [false, true] {
                    assert_eq!(
                        magnet_discovery(trackers, cached, peers, dht, consent),
                        AsToday
                    );
                }
            }
        }
        assert_eq!(
            magnet_discovery(0, false, false, false, false),
            Refuse(reason::DHT_OFF)
        );
        assert_eq!(
            magnet_discovery(0, false, false, false, true),
            Refuse(reason::DHT_OFF),
            "a yes never turns the DHT on"
        );
        assert_eq!(
            magnet_discovery(0, false, false, true, false),
            Refuse(reason::NEEDS_DHT_CONSENT)
        );
        assert_eq!(magnet_discovery(0, false, false, true, true), DhtSearchOnly);
    }

    #[tokio::test]
    async fn uncached_trackerless_magnet_requires_a_torrent_file() {
        let session = Session::new(SessionOptions::default()).await.unwrap();
        let link = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789";
        let error = session
            .add_magnet("one".into(), link, AddOptions::default())
            .await
            .unwrap_err();
        assert_eq!(reason::code_of(&error), Some("dht_off"));
        let asked = AddOptions {
            dht_search: true,
            ..Default::default()
        };
        let error = session
            .add_magnet("one".into(), link, asked)
            .await
            .unwrap_err();
        assert_eq!(reason::code_of(&error), Some("dht_off"));
        assert!(session.all().is_empty());
        session.shutdown().await;
    }

    #[tokio::test]
    async fn a_trackerless_magnet_waits_for_a_yes_while_the_dht_is_on() {
        // Trackers and the DHT start lazily, so refusing here opens no socket.
        let session = Session::new(SessionOptions {
            use_trackers: true,
            use_dht: true,
            dht_bootstrap: Some(Vec::new()),
            ..Default::default()
        })
        .await
        .unwrap();
        let link = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789&tr=http%3A%2F%2F127.0.0.1%2Fannounce";
        let error = session
            .add_magnet("one".into(), link, AddOptions::default())
            .await
            .unwrap_err();
        assert_eq!(
            reason::code_of(&error),
            Some("needs_dht_consent"),
            "a tracker on this computer is not a usable tracker"
        );
        assert!(error.to_string().contains(".torrent file"));
        assert!(session.all().is_empty());
        session.shutdown().await;
    }

    #[tokio::test]
    async fn test_a_removed_torrent_has_let_go_of_its_files() {
        let (state, output) = (TempFolder::new(), TempFolder::new());
        let data = sample_bytes(40_000, 43);
        let file = single_file_torrent("a.bin", &data, PIECE);
        std::fs::write(output.path().join("a.bin"), &data).unwrap();
        let session = Session::new(session_options(state.path())).await.unwrap();
        session
            .add_torrent_file("one".into(), &file, add_options(output.path()))
            .await
            .unwrap();
        let torrent = session.torrent("one").unwrap();
        wait_until(&torrent, "the check", |stats| stats.finished).await;
        session.remove("one").await.unwrap();
        std::fs::rename(output.path().join("a.bin"), output.path().join("b.bin")).unwrap();
    }

    #[tokio::test]
    async fn test_uploads_can_be_turned_off_while_running() {
        let state = TempFolder::new();
        let session = Session::new(session_options(state.path())).await.unwrap();
        assert!(!session.context.no_upload.load(Ordering::Relaxed));
        session.configure(Some(1024), true);
        assert!(session.context.no_upload.load(Ordering::Relaxed));
        session.configure(None, false);
        assert!(!session.context.no_upload.load(Ordering::Relaxed));
    }
}
