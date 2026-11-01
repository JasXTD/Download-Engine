//! Finds peers through trackers and the DHT, using the copied crates.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures_util::StreamExt;
use librqbit_core::hash_id::Id20;
use librqbit_core::network::is_public_endpoint;
use librqbit_dht::{Dht, DhtBuilder, DhtConfig, DhtPersistenceConfig, PersistentDht};
use librqbit_tracker_comms::{
    TorrentStatsProvider, TrackerComms, UdpTrackerClient, is_local_address,
};
use parking_lot::Mutex;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use tokio::sync::{OnceCell, mpsc};
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

use crate::debug_log;
use crate::torrent::{MAX_KNOWN_PEERS, can_reach};

/// Trackers refuse an announce without a port, even when nothing is listening on it.
const PORT_WHEN_NOT_LISTENING: u16 = 6881;
const TRACKER_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const TRACKER_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_PENDING_PEERS: usize = 128;

/// One shared budget for trackers, DHT and peer exchange, including already known peers.
#[derive(Clone)]
pub struct PeerCandidates {
    tx: mpsc::Sender<SocketAddr>,
    seen: Arc<Mutex<HashSet<SocketAddr>>>,
    limit: usize,
    allow_local: bool,
}

impl PeerCandidates {
    pub(crate) fn channel(
        known: impl Iterator<Item = SocketAddr>,
        allow_local: bool,
    ) -> (Self, mpsc::Receiver<SocketAddr>) {
        let (tx, rx) = mpsc::channel(MAX_PENDING_PEERS);
        let mut remaining = MAX_KNOWN_PEERS;
        let seen: HashSet<_> = known
            .take(MAX_KNOWN_PEERS)
            .map(|addr| {
                remaining -= 1;
                canonical(addr)
            })
            .collect();
        // Explicit aliases can occupy separate book entries but share one canonical key.
        let limit = seen.len() + remaining;
        (
            Self {
                tx,
                seen: Arc::new(Mutex::new(seen)),
                limit,
                allow_local,
            },
            rx,
        )
    }

    fn accepts(&self, addr: SocketAddr) -> bool {
        if !can_reach(addr) || (!self.allow_local && !is_public_endpoint(addr)) {
            return false;
        }
        let seen = self.seen.lock();
        seen.len() < self.limit && !seen.contains(&addr)
    }

    fn publish(&self, addr: SocketAddr, slot: mpsc::Permit<'_, SocketAddr>) {
        let mut seen = self.seen.lock();
        // Another producer may have queued this address while we waited for space.
        if seen.len() < self.limit && seen.insert(addr) {
            slot.send(addr);
        }
    }

    /// Backpressure the searches, but keep tracker announcements alive after the book fills.
    async fn send(&self, addr: SocketAddr) -> bool {
        if self.tx.is_closed() {
            return false;
        }
        let addr = canonical(addr);
        if !self.accepts(addr) {
            return true;
        }
        let Ok(slot) = self.tx.reserve().await else {
            return false;
        };
        self.publish(addr, slot);
        true
    }

    /// Peer exchange must not block a connection's reader on discovery backpressure.
    pub(crate) fn try_send(&self, addr: SocketAddr) {
        let addr = canonical(addr);
        if self.accepts(addr)
            && let Ok(slot) = self.tx.try_reserve()
        {
            self.publish(addr, slot);
        }
    }
}

fn canonical(addr: SocketAddr) -> SocketAddr {
    SocketAddr::new(addr.ip().to_canonical(), addr.port())
}

async fn drain_addresses<S>(mut stream: S, found: PeerCandidates)
where
    S: futures_util::Stream<Item = SocketAddr> + Unpin,
{
    while let Some(addr) = stream.next().await {
        if !found.send(addr).await {
            break;
        }
    }
}

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
    /// None until magnet metadata has been fetched and hash-checked.
    pub private: Option<bool>,
    pub stats: Box<dyn TorrentStatsProvider>,
    pub found: PeerCandidates,
    pub cancel: CancellationToken,
}

/// Whether an address leads to this computer or its own network. A tracker address comes
/// from a stranger, and must not make us send requests to a router or a local service.
fn stays_local(url: &Url) -> bool {
    // Only web addresses have their hosts read as numbers. A udp:// one keeps plain text,
    // which the system would still read as a number, so read every host as a web address.
    let Some(web) = url
        .host_str()
        .and_then(|host| Url::parse(&format!("http://{host}/")).ok())
    else {
        return true;
    };
    match web.host() {
        Some(Host::Ipv4(ip)) => is_local_address(ip.into()),
        Some(Host::Ipv6(ip)) => is_local_address(ip.into()),
        Some(Host::Domain(name)) => {
            let name = name.trim_end_matches('.');
            name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local")
        }
        None => true,
    }
}

/// Looks up tracker names and keeps only public addresses. The address in a tracker's URL
/// isn't enough to go on, because any name can point anywhere.
struct PublicNames;

impl Resolve for PublicNames {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let found = tokio::net::lookup_host((name.as_str(), 0)).await?;
            let public: Vec<SocketAddr> = found
                .filter(|address| !is_local_address(address.ip()))
                .collect();
            if public.is_empty() {
                return Err(format!("{} has no public address", name.as_str()).into());
            }
            Ok(Box::new(public.into_iter()) as Addrs)
        })
    }
}

impl Discovery {
    pub fn new(options: DiscoveryOptions, cancel: CancellationToken) -> Result<Arc<Self>> {
        // A tracker comes from a stranger, so it gets no say in where this client goes next.
        let mut http = reqwest::Client::builder()
            .user_agent(crate::peer::CLIENT_NAME)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(TRACKER_CONNECT_TIMEOUT)
            .timeout(TRACKER_TIMEOUT);
        if !options.allow_local_trackers {
            http = http.dns_resolver(Arc::new(PublicNames));
        }
        let http = http.build()?;
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
                UdpTrackerClient::new(
                    self.cancel.clone(),
                    None,
                    !self.options.allow_local_trackers,
                )
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
                            allow_local_addresses: self.options.allow_local_trackers,
                        };
                        PersistentDht::create(config, None, false, bootstrap, cancel, None).await
                    }
                    None => DhtBuilder::with_config(DhtConfig {
                        bootstrap_addrs: bootstrap,
                        cancellation_token: cancel,
                        allow_local_addresses: self.options.allow_local_trackers,
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
        if self.options.use_dht && permits_dht(private) {
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

fn permits_dht(private: Option<bool>) -> bool {
    private == Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn discovery_backpressures_before_draining_a_large_source() {
        let (found, mut received) = PeerCandidates::channel(std::iter::empty(), false);
        let polled = AtomicUsize::new(0);
        let source = futures_util::stream::iter((1..1000).map(|port| {
            polled.fetch_add(1, Ordering::Relaxed);
            SocketAddr::from(([1, 2, 3, 4], port))
        }));
        let mut pumping = Box::pin(drain_addresses(source, found));
        assert!(pumping.as_mut().now_or_never().is_none());
        tokio::task::yield_now().await;
        assert!(pumping.as_mut().now_or_never().is_none());
        assert_eq!(received.len(), 128);
        assert_eq!(polled.load(Ordering::Relaxed), 129);
        received.recv().await.unwrap();
        assert!(pumping.as_mut().now_or_never().is_none());
        assert_eq!(received.len(), 128);
        assert_eq!(polled.load(Ordering::Relaxed), 130);
        drop(received);
        pumping.await;
        assert_eq!(polled.load(Ordering::Relaxed), 130);
    }

    #[tokio::test]
    async fn discovery_budget_is_shared_and_ignores_duplicates_and_rejected_addresses() {
        let known = (1..=4998).map(|port| SocketAddr::from(([9, 9, 9, 9], port)));
        let (found, mut received) = PeerCandidates::channel(known, false);
        assert!(found.send("[::ffff:9.9.9.9]:1".parse().unwrap()).await);
        assert!(found.send("10.0.0.1:80".parse().unwrap()).await);
        assert!(found.send("8.8.8.8:0".parse().unwrap()).await);
        let other = found.clone();
        let one = "1.2.3.4:6881".parse().unwrap();
        let two = "1.2.3.5:6881".parse().unwrap();
        assert_eq!(tokio::join!(found.send(one), other.send(one)), (true, true));
        other.try_send(two);
        assert!(found.send("1.2.3.6:6881".parse().unwrap()).await);
        assert_eq!(received.try_recv().unwrap(), one);
        assert_eq!(received.try_recv().unwrap(), two);
        assert!(received.try_recv().is_err());

        // Announcements must keep running when their peer results are no longer needed.
        let updates = AtomicUsize::new(0);
        let source = futures_util::stream::iter((0..3).map(|_| {
            updates.fetch_add(1, Ordering::Relaxed);
            "1.2.3.7:6881".parse().unwrap()
        }));
        drain_addresses(source, found).await;
        assert_eq!(updates.load(Ordering::Relaxed), 3);
        assert!(received.try_recv().is_err());
    }

    #[tokio::test]
    async fn peer_exchange_drops_overflow_without_spending_the_search_budget() {
        let (found, mut received) = PeerCandidates::channel(std::iter::empty(), false);
        for port in 1..=129 {
            found.try_send(([1, 2, 3, 4], port).into());
        }
        assert_eq!(received.len(), 128);
        assert_eq!(received.recv().await.unwrap().port(), 1);
        found.try_send(([1, 2, 3, 4], 129).into());
        let mut ports = Vec::new();
        while let Ok(addr) = received.try_recv() {
            ports.push(addr.port());
        }
        assert_eq!(ports, (2..=129).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn searches_waiting_for_space_still_deduplicate_against_each_other() {
        let (found, mut received) = PeerCandidates::channel(std::iter::empty(), false);
        for port in 1..=128 {
            found.try_send(([1, 2, 3, 4], port).into());
        }
        let peer = SocketAddr::from(([1, 2, 3, 4], 129));
        let other = found.clone();
        let mut one = Box::pin(found.send(peer));
        let mut two = Box::pin(other.send(peer));
        assert!(one.as_mut().now_or_never().is_none());
        assert!(two.as_mut().now_or_never().is_none());
        received.recv().await.unwrap();
        received.recv().await.unwrap();
        assert_eq!(tokio::join!(one, two), (true, true));
        let mut peers = Vec::new();
        while let Ok(addr) = received.try_recv() {
            peers.push(addr);
        }
        assert_eq!(peers.len(), 127);
        assert_eq!(peers.iter().filter(|addr| **addr == peer).count(), 1);
    }

    #[tokio::test]
    async fn explicit_address_aliases_do_not_create_more_book_capacity() {
        let known = (1..=2500).flat_map(|port| {
            [
                SocketAddr::from(([1, 2, 3, 4], port)),
                format!("[::ffff:1.2.3.4]:{port}").parse().unwrap(),
            ]
        });
        let (found, mut received) = PeerCandidates::channel(known, false);
        assert!(found.send("1.2.3.5:6881".parse().unwrap()).await);
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn dht_requires_metadata_confirming_a_public_torrent() {
        assert!(!permits_dht(None));
        assert!(!permits_dht(Some(true)));
        assert!(permits_dht(Some(false)));
    }

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

    #[tokio::test]
    async fn test_a_name_that_leads_here_is_not_used() {
        let name = "localhost".parse().unwrap();
        assert!(PublicNames.resolve(name).await.is_err());
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
