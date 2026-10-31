//! Finds peers through trackers and the DHT, using the copied crates.

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use librqbit_core::hash_id::Id20;
use librqbit_dht::{Dht, DhtBuilder, DhtConfig, DhtPersistenceConfig, PersistentDht};
use librqbit_tracker_comms::{TorrentStatsProvider, TrackerComms, UdpTrackerClient};
use tokio::sync::{OnceCell, mpsc};
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

use crate::debug_log;
use crate::torrent::drain_addresses;

/// Trackers refuse an announce without a port, even when nothing is listening on it.
const PORT_WHEN_NOT_LISTENING: u16 = 6881;
const TRACKER_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const TRACKER_TIMEOUT: Duration = Duration::from_secs(60);

pub struct DiscoveryOptions {
    pub peer_id: Id20,
    pub listen_port: Option<u16>,
    pub use_trackers: bool,
    pub use_dht: bool,
    pub dht_bootstrap: Option<Vec<String>>,
    pub state_dir: Option<PathBuf>,
    /// Whether trackers may be on this computer or its network. Only for local testing.
    pub allow_local_trackers: bool,
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

/// Whether an address leads to this computer or its own network. A tracker address comes
/// from a stranger, and must not make us send requests to a router or a local service.
fn stays_local(url: &Url) -> bool {
    fn ipv4_stays_local(ip: Ipv4Addr) -> bool {
        ip.is_private()
            || ip.is_loopback()
            || ip.is_link_local()
            || ip.is_broadcast()
            || ip.octets()[0] == 0
    }
    // Only web addresses have their hosts read as numbers. A udp:// one keeps plain text,
    // which the system would still read as a number, so read every host as a web address.
    let Some(web) = url
        .host_str()
        .and_then(|host| Url::parse(&format!("http://{host}/")).ok())
    else {
        return true;
    };
    match web.host() {
        Some(Host::Ipv4(ip)) => ipv4_stays_local(ip),
        Some(Host::Ipv6(ip)) => match ip.to_ipv4_mapped() {
            Some(ip) => ipv4_stays_local(ip),
            None => {
                let first = ip.segments()[0];
                ip.is_loopback()
                    || ip.is_unspecified()
                    || first & 0xfe00 == 0xfc00
                    || first & 0xffc0 == 0xfe80
            }
        },
        Some(Host::Domain(name)) => {
            let name = name.trim_end_matches('.');
            name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local")
        }
        None => true,
    }
}

impl Discovery {
    pub fn new(options: DiscoveryOptions, cancel: CancellationToken) -> Result<Arc<Self>> {
        // A tracker comes from a stranger, so it gets no say in where this client goes next.
        let http = reqwest::Client::builder()
            .user_agent(crate::peer::CLIENT_NAME)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(TRACKER_CONNECT_TIMEOUT)
            .timeout(TRACKER_TIMEOUT)
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
                let bootstrap = self.options.dht_bootstrap.clone();
                let dht = match &self.options.state_dir {
                    Some(state_dir) => {
                        let config = DhtPersistenceConfig {
                            dump_interval: None,
                            config_filename: state_dir.join("dht.json"),
                        };
                        PersistentDht::create(config, None, false, bootstrap, cancel, None).await
                    }
                    None => DhtBuilder::with_config(DhtConfig {
                        bootstrap_addrs: bootstrap,
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
                let trackers: HashSet<Url> = trackers
                    .into_iter()
                    .filter(|url| this.options.allow_local_trackers || !stays_local(url))
                    .collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_tracker_cannot_lead_to_this_computer_or_its_network() {
        for address in [
            "http://127.0.0.1:8080/announce",
            "http://127.9.9.9/announce",
            "http://10.0.0.1/announce",
            "http://172.16.0.1/announce",
            "http://192.168.1.1/announce",
            "http://169.254.169.254/latest",
            "http://0.0.0.0/announce",
            "http://255.255.255.255/announce",
            "http://[::1]/announce",
            "http://[::]/announce",
            "http://[fc00::1]/announce",
            "http://[fe80::1]/announce",
            "http://[::ffff:192.168.1.1]/announce",
            "http://localhost/announce",
            "http://LOCALHOST./announce",
            "http://router.local/announce",
            "http://printer.localhost/announce",
            "udp://192.168.1.1:80/announce",
            "udp://127.0.0.1:80/announce",
            "udp://2130706433:80/announce",
            "udp://0x7f.1:80/announce",
            "udp://[::1]:80/announce",
            "udp://LOCALHOST:80/announce",
        ] {
            assert!(stays_local(&Url::parse(address).unwrap()), "{address}");
        }
    }

    #[test]
    fn test_public_trackers_are_left_alone() {
        for address in [
            "http://bttracker.debian.org:6969/announce",
            "https://tracker.example.org/announce",
            "udp://tracker.example.org:1337/announce",
            "http://93.184.216.34/announce",
            "http://[2606:2800:220:1::1]/announce",
            "http://local.example.org/announce",
        ] {
            assert!(!stays_local(&Url::parse(address).unwrap()), "{address}");
        }
    }
}
