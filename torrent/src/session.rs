//! All running torrents, and what they share.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use librqbit_core::magnet::Magnet;
use librqbit_dualstack_sockets::{BindOpts, TcpListener};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::debug_log;
use crate::discovery::{Discovery, DiscoveryOptions};
use crate::meta::{TorrentMeta, usable_tracker};
use crate::peer::read_handshake;
use crate::rate::ByteLimiter;
use crate::torrent::{Context, Torrent, TorrentOptions, TorrentStats, Totals};

/// Connections that haven't said which torrent they want yet.
const MAX_PENDING_HANDSHAKES: usize = 32;

#[derive(Clone, Default)]
pub struct SessionOptions {
    /// Incoming connections stay off unless a port is set.
    pub listen_port: Option<u16>,
    pub upload_limit: Option<u32>,
    pub download_limit: Option<u32>,
    pub seed: bool,
    pub use_trackers: bool,
    pub use_dht: bool,
    pub state_dir: Option<PathBuf>,
}

#[derive(Clone, Default)]
pub struct AddOptions {
    pub output: PathBuf,
    pub only: Option<Vec<usize>>,
    pub peers: Vec<SocketAddr>,
    pub choose_later: bool,
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
}

pub struct Session {
    options: SessionOptions,
    context: Context,
    torrents: Mutex<BTreeMap<String, Arc<Torrent>>>,
    cancel: CancellationToken,
}

impl Session {
    pub async fn new(options: SessionOptions) -> Result<Arc<Self>> {
        let cancel = CancellationToken::new();
        let peer_id = librqbit_core::peer_id::generate_azereus_style(*b"DE", (0, 1, 0, 0));
        let discovery = if options.use_trackers || options.use_dht {
            Some(Discovery::new(
                DiscoveryOptions {
                    peer_id,
                    listen_port: options.listen_port,
                    use_trackers: options.use_trackers,
                    use_dht: options.use_dht,
                    state_dir: options.state_dir.clone(),
                },
                cancel.clone(),
            )?)
        } else {
            None
        };
        let context = Context {
            peer_id,
            up: Arc::new(ByteLimiter::new(options.upload_limit)),
            down: Arc::new(ByteLimiter::new(options.download_limit)),
            totals: Arc::new(Totals::default()),
            discovery,
        };
        let session = Arc::new(Self {
            options,
            context,
            torrents: Mutex::default(),
            cancel,
        });
        if let Some(port) = session.options.listen_port {
            let listener = bind(port)?;
            tokio::spawn(session.clone().listen(listener));
        }
        Ok(session)
    }

    fn torrent_options(&self, add: AddOptions) -> TorrentOptions {
        TorrentOptions {
            output: add.output,
            only: add.only,
            peers: add.peers,
            seed: self.options.seed,
            listen_port: self.options.listen_port,
            state_dir: self.options.state_dir.clone(),
            choose_later: add.choose_later,
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

    /// Adds a torrent from the bytes of a .torrent file and starts it.
    pub async fn add_torrent_file(
        &self,
        name: String,
        bytes: &[u8],
        add: AddOptions,
    ) -> Result<String> {
        let meta = Arc::new(TorrentMeta::from_torrent_file(bytes)?);
        let options = self.torrent_options(add);
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
        let trackers = magnet
            .trackers
            .iter()
            .filter_map(|tracker| usable_tracker(tracker))
            .collect();
        let mut add = add;
        if add.only.is_none() {
            add.only = magnet.get_select_only();
        }
        let options = self.torrent_options(add);
        let torrent = Torrent::from_magnet(info_hash, trackers, options, self.context.clone());
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
        }
    }

    /// Stops everything and saves progress.
    pub async fn shutdown(&self) {
        for (_, torrent) in self.all() {
            torrent.pause().await;
        }
        self.cancel.cancel();
    }

    async fn listen(self: Arc<Self>, listener: TcpListener) {
        let pending = Arc::new(Semaphore::new(MAX_PENDING_HANDSHAKES));
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
            let Ok(permit) = pending.clone().try_acquire_owned() else {
                continue;
            };
            let session = self.clone();
            tokio::spawn(async move {
                let _permit = permit;
                if let Err(error) = session.accept(stream, addr).await {
                    debug_log!("incoming peer {addr}: {error:#}");
                }
            });
        }
    }

    async fn accept(&self, mut stream: tokio::net::TcpStream, addr: SocketAddr) -> Result<()> {
        stream.set_nodelay(true)?;
        let (info_hash, hello) = read_handshake(&mut stream).await?;
        let torrent = self
            .all()
            .into_iter()
            .map(|(_, torrent)| torrent)
            .find(|torrent| torrent.info_hash() == info_hash)
            .context("peer asked for a torrent we don't have")?;
        torrent.offer(Box::new(stream), addr, hello);
        Ok(())
    }
}

fn bind(port: u16) -> Result<TcpListener> {
    let both = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port);
    TcpListener::bind_tcp(both, BindOpts::default()).or_else(|_| {
        let only_v4 = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
        let options = BindOpts {
            request_dualstack: false,
            ..Default::default()
        };
        TcpListener::bind_tcp(only_v4, options)
            .with_context(|| format!("Could not listen on port {port}"))
    })
}
