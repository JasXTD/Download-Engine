//! All running torrents, and what they share.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use librqbit_core::magnet::Magnet;
use librqbit_dualstack_sockets::{BindOpts, TcpListener};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::debug_log;
use crate::discovery::{Discovery, DiscoveryOptions};
use crate::meta::{TorrentMeta, usable_tracker};
use crate::peer::read_handshake;
use crate::rate::ByteLimiter;
use crate::resume;
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
    /// The DHT's first contacts, or None for the public ones.
    pub dht_bootstrap: Option<Vec<String>>,
    pub state_dir: Option<PathBuf>,
    /// Whether peer lists from strangers may name this computer. Only for local testing.
    pub allow_loopback_peers: bool,
    /// Sends no torrent data to anyone.
    pub no_upload: bool,
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
            connect_out: true,
            allow_loopback: self.options.allow_loopback_peers,
            allowance: Arc::default(),
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
        let mut add = add;
        if add.only.is_none() {
            add.only = magnet.get_select_only();
        }
        let options = self.torrent_options(add);
        let saved = self
            .options
            .state_dir
            .as_deref()
            .and_then(|state_dir| resume::load_info(state_dir, info_hash, trackers.clone()));
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
        session
            .add_magnet("one".into(), &magnet_for(&meta), add_options(output.path()))
            .await
            .unwrap();
        let torrent = session.torrent("one").unwrap();
        wait_until(&torrent, "the search for details", |stats| {
            stats.state == "metadata"
        })
        .await;
        assert!(torrent.meta().is_none());
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
