//! Finds peers through trackers and the DHT, using the copied crates.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use librqbit_core::hash_id::Id20;
use librqbit_dht::{Dht, DhtBuilder, DhtConfig, DhtPersistenceConfig, PersistentDht};
use librqbit_tracker_comms::{TorrentStatsProvider, TrackerComms, UdpTrackerClient};
use tokio::sync::{OnceCell, mpsc};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::debug_log;
use crate::torrent::drain_addresses;

/// Trackers refuse an announce without a port, even when nothing is listening on it.
const PORT_WHEN_NOT_LISTENING: u16 = 6881;

pub struct DiscoveryOptions {
    pub peer_id: Id20,
    pub listen_port: Option<u16>,
    pub use_trackers: bool,
    pub use_dht: bool,
    pub state_dir: Option<PathBuf>,
}

pub struct Discovery {
    options: DiscoveryOptions,
    http: reqwest::Client,
    udp: OnceCell<Option<UdpTrackerClient>>,
    dht: OnceCell<Option<Dht>>,
    cancel: CancellationToken,
}

/// One torrent's request for peers.
pub struct Search {
    pub info_hash: Id20,
    pub trackers: Vec<Url>,
    pub private: bool,
    pub stats: Box<dyn TorrentStatsProvider>,
    pub found: mpsc::UnboundedSender<SocketAddr>,
    pub cancel: CancellationToken,
}

impl Discovery {
    pub fn new(options: DiscoveryOptions, cancel: CancellationToken) -> Result<Arc<Self>> {
        let http = reqwest::Client::builder()
            .user_agent(crate::peer::CLIENT_NAME)
            .build()?;
        Ok(Arc::new(Self {
            options,
            http,
            udp: OnceCell::new(),
            dht: OnceCell::new(),
            cancel,
        }))
    }

    async fn udp_client(&self) -> Option<UdpTrackerClient> {
        self.udp
            .get_or_init(|| async {
                UdpTrackerClient::new(self.cancel.clone(), None)
                    .await
                    .inspect_err(|error| debug_log!("UDP trackers are off: {error:#}"))
                    .ok()
            })
            .await
            .clone()
    }

    async fn dht(&self) -> Option<Dht> {
        self.dht
            .get_or_init(|| async {
                let cancel = Some(self.cancel.clone());
                let dht = match &self.options.state_dir {
                    Some(state_dir) => {
                        let config = DhtPersistenceConfig {
                            dump_interval: None,
                            config_filename: state_dir.join("dht.json"),
                        };
                        PersistentDht::create(config, None, false, None, cancel, None).await
                    }
                    None => DhtBuilder::with_config(DhtConfig {
                        cancellation_token: cancel,
                        ..Default::default()
                    })
                    .await
                    .map_err(anyhow::Error::from),
                };
                dht.inspect_err(|error| debug_log!("the DHT is off: {error:#}"))
                    .ok()
            })
            .await
            .clone()
    }

    /// Starts looking. Peers arrive on `found` until `cancel` fires.
    pub fn start(self: &Arc<Self>, search: Search) {
        let Search {
            info_hash,
            trackers,
            private,
            stats,
            found,
            cancel,
        } = search;
        if self.options.use_trackers && !trackers.is_empty() {
            let this = self.clone();
            let (found, cancel) = (found.clone(), cancel.clone());
            tokio::spawn(async move {
                let Some(udp) = this.udp_client().await else {
                    return;
                };
                let trackers: HashSet<Url> = trackers.into_iter().collect();
                let port = this.options.listen_port.unwrap_or(PORT_WHEN_NOT_LISTENING);
                let stream = TrackerComms::start(
                    info_hash,
                    this.options.peer_id,
                    trackers,
                    stats,
                    None,
                    port,
                    this.http.clone(),
                    udp,
                );
                if let Some(stream) = stream {
                    tokio::select! {
                        _ = cancel.cancelled() => {}
                        _ = drain_addresses(stream, found) => {}
                    }
                }
            });
        }
        // A private torrent's peers may only come from its own trackers.
        if self.options.use_dht && !private {
            let this = self.clone();
            tokio::spawn(async move {
                let Some(dht) = this.dht().await else {
                    return;
                };
                let stream = dht.get_peers(info_hash, this.options.listen_port);
                tokio::select! {
                    _ = cancel.cancelled() => {}
                    _ = drain_addresses(stream, found) => {}
                }
            });
        }
    }
}
