// Modified by Download Engine: replaced rand, backon and async-stream, and added public-only
// name lookups and bounded tracker resources. See ../../patch.md.
use std::collections::HashSet;
use std::net::SocketAddr;
use std::net::SocketAddrV4;
use std::net::SocketAddrV6;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::Context as TaskContext;
use std::task::Poll;
use std::time::Duration;

use anyhow::Context;
use anyhow::bail;
use futures::FutureExt;
use futures::Stream;
use futures::StreamExt;
use futures::future::BoxFuture;
use futures::future::Either;
use futures::stream::BoxStream;
use futures::stream::FuturesUnordered;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::Instrument;
use tracing::debug;
use tracing::debug_span;
use tracing::trace;
use tracing::trace_span;
use url::Url;

use crate::tracker_comms_http;
use crate::tracker_comms_udp;
use crate::tracker_comms_udp::UdpTrackerClient;
use librqbit_core::backoff::Backoff;
use librqbit_core::hash_id::Id20;
pub use librqbit_core::network::is_local_address;
use librqbit_core::torrent_limits::{MAX_TRACKER_URL_BYTES, MAX_TRACKERS};

const MAX_TRACKER_RESPONSE: usize = 1024 * 1024;
/// Ask for this many peers and enforce it even when a tracker ignores numwant.
pub(crate) const MAX_TRACKER_PEERS: usize = 200;
const MIN_TRACKER_INTERVAL: Duration = Duration::from_secs(5);
const MAX_TRACKER_REQUESTS: usize = 4;
const MAX_GLOBAL_TRACKER_REQUESTS: usize = 8;
static TRACKER_REQUESTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

/// Both chunked and fixed-length responses are subject to the same byte limit.
async fn read_tracker_body(response: reqwest::Response) -> anyhow::Result<Vec<u8>> {
    let length = response.content_length();
    let chunks = futures::stream::try_unfold(response, |mut response| async move {
        let chunk = response.chunk().await?;
        Ok::<_, reqwest::Error>(chunk.map(|chunk| (chunk, response)))
    });
    read_tracker_chunks(length, chunks).await
}

async fn read_tracker_chunks<B, E>(
    length: Option<u64>,
    chunks: impl Stream<Item = Result<B, E>>,
) -> anyhow::Result<Vec<u8>>
where
    B: AsRef<[u8]>,
    E: std::error::Error + Send + Sync + 'static,
{
    anyhow::ensure!(
        length.is_none_or(|len| len <= MAX_TRACKER_RESPONSE as u64),
        "tracker response exceeds the size limit"
    );
    // Reserve the capped buffer once: tiny HTTP chunks must not cause repeated
    // reallocations, and geometric growth must not exceed the byte budget.
    let mut bytes = Vec::with_capacity(MAX_TRACKER_RESPONSE);
    futures::pin_mut!(chunks);
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        let chunk = chunk.as_ref();
        anyhow::ensure!(
            chunk.len() <= MAX_TRACKER_RESPONSE - bytes.len(),
            "tracker response exceeds the size limit"
        );
        bytes.extend_from_slice(chunk);
    }
    Ok(bytes)
}

fn tracker_interval(interval: u64, min_interval: Option<u64>) -> Duration {
    Duration::from_secs(interval.max(min_interval.unwrap_or(0))).max(MIN_TRACKER_INTERVAL)
}

/// The peers a torrent's trackers find. Trackers only run while this stream is polled,
/// dropping it stops them, and it ends once they have all stopped.
struct TrackerPeers {
    trackers: Option<BoxFuture<'static, ()>>,
    peers: tokio::sync::mpsc::Receiver<SocketAddr>,
}

impl Stream for TrackerPeers {
    type Item = SocketAddr;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<SocketAddr>> {
        if let Some(trackers) = &mut self.trackers
            && trackers.as_mut().poll(cx).is_ready()
        {
            self.trackers = None;
        }
        self.peers.poll_recv(cx)
    }
}

pub struct TrackerComms {
    info_hash: Id20,
    peer_id: Id20,
    stats: Box<dyn TorrentStatsProvider>,
    force_tracker_interval: Option<Duration>,
    tx: Sender,
    // This MUST be set as trackers don't work with 0 port.
    announce_port: u16,
    reqwest_client: reqwest::Client,
    key: u32,
    requests: Arc<Semaphore>,
}

#[derive(Default)]
pub enum TrackerCommsStatsState {
    #[default]
    None,
    Initializing,
    Paused,
    Live,
}

#[derive(Default)]
pub struct TrackerCommsStats {
    pub uploaded_bytes: u64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub torrent_state: TrackerCommsStatsState,
}

impl TrackerCommsStats {
    pub fn get_left_to_download_bytes(&self) -> u64 {
        let total = self.total_bytes;
        let down = self.downloaded_bytes;
        if total >= down {
            return total - down;
        }
        0
    }

    pub fn is_completed(&self) -> bool {
        self.downloaded_bytes >= self.total_bytes
    }
}

pub trait TorrentStatsProvider: Send + Sync {
    fn get(&self) -> TrackerCommsStats;
}

impl TorrentStatsProvider for () {
    fn get(&self) -> TrackerCommsStats {
        Default::default()
    }
}

type Sender = tokio::sync::mpsc::Sender<SocketAddr>;

enum SupportedTracker {
    Udp(Url),
    Http(Url),
}

impl std::fmt::Debug for SupportedTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupportedTracker::Udp(u) => std::fmt::Display::fmt(u, f),
            SupportedTracker::Http(u) => std::fmt::Display::fmt(u, f),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum UdpTrackerResolveResult {
    One(SocketAddr),
    Two(SocketAddrV4, SocketAddrV6),
}

async fn udp_tracker_to_socket_addrs(
    host: url::Host<&str>,
    port: u16,
    public_only: bool,
) -> anyhow::Result<UdpTrackerResolveResult> {
    let res = match host {
        url::Host::Domain(name) => {
            // Use the first IPv4 and the first IPv6 addresses only.

            let mut v4: Option<SocketAddrV4> = None;
            let mut v6: Option<SocketAddrV6> = None;
            for addr in tokio::net::lookup_host((name, port))
                .await
                .with_context(|| format!("error looking up hostname {name}"))?
                .filter(|addr| !public_only || !is_local_address(addr.ip()))
            {
                match (v4, v6, addr) {
                    (None, _, SocketAddr::V4(addr)) => v4 = Some(addr),
                    (_, None, SocketAddr::V6(addr)) => v6 = Some(addr),
                    _ => continue,
                }
            }
            let res = match (v4, v6) {
                (Some(v4), Some(v6)) => UdpTrackerResolveResult::Two(v4, v6),
                (Some(v4), None) => UdpTrackerResolveResult::One(v4.into()),
                (None, Some(v6)) => UdpTrackerResolveResult::One(v6.into()),
                _ => anyhow::bail!("zero addresses returned looking up {name}"),
            };
            trace!(?res, "resolved");
            res
        }
        url::Host::Ipv4(addr) => UdpTrackerResolveResult::One((addr, port).into()),
        url::Host::Ipv6(addr) => UdpTrackerResolveResult::One((addr, port).into()),
    };
    Ok(res)
}

impl TrackerComms {
    // TODO: fix too many args
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        info_hash: Id20,
        peer_id: Id20,
        trackers: HashSet<Url>,
        stats: Box<dyn TorrentStatsProvider>,
        force_interval: Option<Duration>,
        announce_port: u16,
        reqwest_client: reqwest::Client,
        udp_client: UdpTrackerClient,
    ) -> Option<BoxStream<'static, SocketAddr>> {
        // Callers validate at ingress too. Refuse an oversized set before spawning anything.
        if trackers.len() > MAX_TRACKERS
            || trackers
                .iter()
                .any(|t| t.as_str().len() > MAX_TRACKER_URL_BYTES)
        {
            debug!("tracker list exceeds the safety limits");
            return None;
        }
        let trackers = trackers
            .into_iter()
            .filter_map(|t| match t.scheme() {
                "http" | "https" => Some(SupportedTracker::Http(t)),
                "udp" => Some(SupportedTracker::Udp(t)),
                _ => {
                    debug!("unsupported tracker URL: {}", t);
                    None
                }
            })
            .collect::<Vec<_>>();
        if trackers.is_empty() {
            debug!(?info_hash, "trackers list is empty");
            return None;
        }

        tracing::trace!(?trackers);

        let (tx, rx) = tokio::sync::mpsc::channel::<SocketAddr>(16);

        let run_trackers = async move {
            let comms = Arc::new(Self {
                info_hash,
                peer_id,
                stats,
                force_tracker_interval: force_interval,
                tx,
                announce_port,
                reqwest_client,
                key: librqbit_core::random::random_u32(),
                requests: Arc::new(Semaphore::new(MAX_TRACKER_REQUESTS)),
            });
            let mut futures = FuturesUnordered::new();
            for tracker in trackers {
                futures.push(comms.add_tracker(tracker, &udp_client))
            }
            while let Some(result) = futures.next().await {
                if let Err(e) = result {
                    debug!("error: {e}");
                }
            }
        };

        Some(
            TrackerPeers {
                trackers: Some(run_trackers.boxed()),
                peers: rx,
            }
            .boxed(),
        )
    }

    async fn request_permits(
        &self,
    ) -> anyhow::Result<(OwnedSemaphorePermit, OwnedSemaphorePermit)> {
        let torrent = self.requests.clone().acquire_owned().await?;
        let global = TRACKER_REQUESTS
            .get_or_init(|| Arc::new(Semaphore::new(MAX_GLOBAL_TRACKER_REQUESTS)))
            .clone()
            .acquire_owned()
            .await?;
        Ok((torrent, global))
    }

    fn add_tracker(
        &self,
        url: SupportedTracker,
        client: &UdpTrackerClient,
    ) -> Either<
        impl std::future::Future<Output = anyhow::Result<()>> + '_ + Send,
        impl std::future::Future<Output = anyhow::Result<()>> + '_ + Send,
    > {
        let info_hash = self.info_hash;
        match url {
            SupportedTracker::Udp(url) => {
                let span = debug_span!(parent: None, "udp_tracker", tracker = %url, info_hash = ?info_hash);
                self.task_single_tracker_monitor_udp(url, client.clone())
                    .instrument(span)
                    .right_future()
            }
            SupportedTracker::Http(url) => {
                let span = debug_span!(
                    parent: None,
                    "http_tracker",
                    tracker = %url,
                    info_hash = ?info_hash
                );
                self.task_single_tracker_monitor_http(url)
                    .instrument(span)
                    .left_future()
            }
        }
    }

    async fn task_single_tracker_monitor_http(&self, tracker_url: Url) -> anyhow::Result<()> {
        trace!(url=%tracker_url, "starting monitor");
        let mut event = Some(tracker_comms_http::TrackerRequestEvent::Started);

        loop {
            let mut backoff = Backoff::new(Duration::from_secs(10), Duration::from_secs(600));
            let interval = loop {
                match self.tracker_one_request_http(&tracker_url, event).await {
                    Ok(interval) => break interval,
                    Err(err) => {
                        let retry_in = backoff.next_delay();
                        debug!(?retry_in, "error calling tracker: {err:#}");
                        tokio::time::sleep(retry_in).await;
                    }
                }
            };

            event = None;
            let interval = self
                .force_tracker_interval
                .unwrap_or(interval)
                .max(MIN_TRACKER_INTERVAL);
            debug!("sleeping for {:?} after calling tracker", interval);
            tokio::time::sleep(interval).await;
        }
    }

    async fn tracker_one_request_http(
        &self,
        tracker_url: &Url,
        event: Option<tracker_comms_http::TrackerRequestEvent>,
    ) -> anyhow::Result<Duration> {
        let stats = self.stats.get();
        let request = tracker_comms_http::TrackerRequest {
            info_hash: &self.info_hash,
            peer_id: &self.peer_id,
            port: self.announce_port,
            uploaded: stats.uploaded_bytes,
            downloaded: stats.downloaded_bytes,
            left: stats.get_left_to_download_bytes(),
            compact: true,
            no_peer_id: false,
            event,
            ip: None,
            numwant: Some(MAX_TRACKER_PEERS),
            key: Some(self.key),
            trackerid: None,
        };

        let mut url = tracker_url.clone();

        let mut queries = request.as_querystring();
        if let Some(url_query) = url.query() {
            queries.push_str(&format!("&{}", url_query));
        }
        url.set_query(Some(&queries));

        let permits = self.request_permits().await?;
        let response: reqwest::Response = self.reqwest_client.get(url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("tracker responded with {:?}", response.status());
        }
        let bytes = read_tracker_body(response).await?;
        if let Ok((error, _)) =
            bencode::from_bytes_with_rest::<tracker_comms_http::TrackerError>(&bytes)
        {
            anyhow::bail!(
                "tracker returned failure. Failure reason: {}",
                error.failure_reason
            )
        };
        let response = bencode::from_bytes_with_rest::<tracker_comms_http::TrackerResponse>(&bytes)
            .map_err(|e| {
                tracing::trace!("error deserializing TrackerResponse: {e:#}");
                e.into_kind()
            })?
            .0;

        // Do not hold a network permit while waiting for the consumer's peer queue.
        drop(permits);
        for peer in response.iter_peers() {
            self.tx.send(peer).await?;
        }
        Ok(tracker_interval(response.interval, response.min_interval))
    }

    async fn task_single_tracker_monitor_udp(
        &self,
        url: Url,
        client: UdpTrackerClient,
    ) -> anyhow::Result<()> {
        if url.scheme() != "udp" {
            bail!("expected UDP scheme in {}", url);
        }
        let (host, port) = (
            url.host().context("missing host")?,
            url.port().context("missing port")?,
        );

        let mut sleep_interval: Option<Duration> = None;
        let mut prev_addrs: Option<UdpTrackerResolveResult> = None;
        loop {
            if let Some(i) = sleep_interval {
                trace!(interval=?sleep_interval, "sleeping");
                tokio::time::sleep(i).await;
            }

            // This should retry forever until the addrs are resolved.
            let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
            let addrs = loop {
                let permits = self.request_permits().await?;
                let resolved =
                    udp_tracker_to_socket_addrs(host.clone(), port, client.public_only())
                        .instrument(trace_span!("resolve", ?host))
                        .await
                        .or_else(|err| prev_addrs.ok_or(err));
                drop(permits);
                match resolved {
                    Ok(addrs) => break addrs,
                    Err(err) => {
                        let retry_in = backoff.next_delay();
                        debug!(?retry_in, "error resolving tracker: {err:#}");
                        tokio::time::sleep(retry_in).await;
                    }
                }
            };

            prev_addrs = Some(addrs);

            match addrs {
                UdpTrackerResolveResult::One(addr) => {
                    match self
                        .tracker_one_request_udp(addr, &client)
                        .instrument(trace_span!("udp request", ?addr))
                        .await
                    {
                        Ok(sleep) => sleep_interval = Some(sleep),
                        Err(_) => {
                            sleep_interval = Some(sleep_interval.unwrap_or(Duration::from_secs(60)))
                        }
                    }
                }
                UdpTrackerResolveResult::Two(v4, v6) => {
                    let (r4, r6) = tokio::join!(
                        self.tracker_one_request_udp(v4.into(), &client)
                            .instrument(trace_span!("udp request", addr=?v4)),
                        self.tracker_one_request_udp(v6.into(), &client)
                            .instrument(trace_span!("udp request", addr=?v6))
                    );
                    sleep_interval = Some(
                        r4.or(r6)
                            .ok()
                            .or(sleep_interval)
                            .unwrap_or(Duration::from_secs(60)),
                    )
                }
            }
        }
    }

    async fn tracker_one_request_udp(
        &self,
        addr: SocketAddr,
        client: &UdpTrackerClient,
    ) -> anyhow::Result<Duration> {
        use tracker_comms_udp::*;

        let stats = self.stats.get();
        let request = AnnounceFields {
            info_hash: self.info_hash,
            peer_id: self.peer_id,
            downloaded: stats.downloaded_bytes,
            left: stats.get_left_to_download_bytes(),
            uploaded: stats.uploaded_bytes,
            event: match stats.torrent_state {
                TrackerCommsStatsState::None => EVENT_NONE,
                TrackerCommsStatsState::Initializing => EVENT_STARTED,
                TrackerCommsStatsState::Paused => EVENT_STOPPED,
                TrackerCommsStatsState::Live => {
                    if stats.is_completed() {
                        EVENT_COMPLETED
                    } else {
                        EVENT_STARTED
                    }
                }
            },
            key: self.key,
            port: self.announce_port,
        };

        let permits = self.request_permits().await?;
        let response = client.announce(addr, request).await;
        drop(permits);
        match response {
            Ok(response) => {
                trace!(len = response.addrs.len(), "received announce response");
                for addr in response.addrs.into_iter().take(MAX_TRACKER_PEERS) {
                    self.tx.send(addr).await.context("rx closed")?;
                }
                let sleep = tracker_interval(response.interval as u64, None);
                Ok(sleep)
            }
            Err(e) => {
                debug!(?addr, "error reading announce response: {e:#}");
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tracker_body_limit_covers_missing_or_dishonest_content_length() {
        for length in [None, Some(1), Some(MAX_TRACKER_RESPONSE as u64)] {
            let chunks = futures::stream::iter([
                Ok::<_, std::io::Error>(vec![0u8; MAX_TRACKER_RESPONSE]),
                Ok(vec![1u8]),
            ])
            .chain(futures::stream::poll_fn(|_| {
                panic!("must stop at the first excess byte")
            }));
            assert!(
                read_tracker_chunks(length, chunks)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("size limit")
            );
        }
        let unpolled =
            futures::stream::poll_fn(|_| -> Poll<Option<Result<Vec<u8>, std::io::Error>>> {
                panic!("oversized Content-Length must fail before reading")
            });
        assert!(
            read_tracker_chunks(Some(MAX_TRACKER_RESPONSE as u64 + 1), unpolled)
                .await
                .is_err()
        );
        let accepted =
            futures::stream::iter([Ok::<_, std::io::Error>(vec![7u8; MAX_TRACKER_RESPONSE])]);
        assert_eq!(
            read_tracker_chunks(None, accepted).await.unwrap().len(),
            MAX_TRACKER_RESPONSE
        );
        let broken =
            futures::stream::iter([Err::<Vec<u8>, _>(std::io::Error::other("broken body"))]);
        assert!(
            read_tracker_chunks(None, broken)
                .await
                .unwrap_err()
                .to_string()
                .contains("broken body")
        );
        let fragmented = futures::stream::iter((0..10_000).map(|_| Ok::<_, std::io::Error>([9u8])));
        assert_eq!(
            read_tracker_chunks(None, fragmented).await.unwrap(),
            vec![9u8; 10_000]
        );
    }

    #[test]
    fn tracker_cannot_force_a_busy_loop_and_longer_intervals_are_respected() {
        assert_eq!(tracker_interval(0, None), Duration::from_secs(5));
        assert_eq!(tracker_interval(0, Some(0)), Duration::from_secs(5));
        assert_eq!(tracker_interval(60, Some(1)), Duration::from_secs(60));
        assert_eq!(tracker_interval(60, Some(120)), Duration::from_secs(120));
        assert_eq!(
            tracker_interval(u64::MAX, None),
            Duration::from_secs(u64::MAX)
        );
    }

    fn comms() -> TrackerComms {
        TrackerComms {
            info_hash: Id20::new([1; 20]),
            peer_id: Id20::new([2; 20]),
            stats: Box::new(()),
            force_tracker_interval: None,
            tx: tokio::sync::mpsc::channel(1).0,
            announce_port: 6881,
            reqwest_client: reqwest::Client::new(),
            key: 0,
            requests: Arc::new(Semaphore::new(MAX_TRACKER_REQUESTS)),
        }
    }

    #[tokio::test]
    async fn request_fanout_is_bounded_per_torrent_and_globally_and_cancel_safe() {
        let (one, two, three) = (comms(), comms(), comms());
        let mut held = Vec::new();
        for comms in [&one, &two] {
            for _ in 0..MAX_TRACKER_REQUESTS {
                held.push(comms.request_permits().await.unwrap());
            }
            assert!(comms.request_permits().now_or_never().is_none());
        }
        assert_eq!(held.len(), MAX_GLOBAL_TRACKER_REQUESTS);
        assert!(three.request_permits().now_or_never().is_none());
        assert_eq!(three.requests.available_permits(), MAX_TRACKER_REQUESTS);
        drop(held);
        assert!(three.request_permits().now_or_never().unwrap().is_ok());
    }

    #[tokio::test]
    async fn test_every_peer_arrives_and_the_stream_ends_with_its_trackers() {
        // More peers than the channel holds, so sending depends on the stream being polled.
        let expected: Vec<SocketAddr> = (0..100u16)
            .map(|port| SocketAddr::from(([127, 0, 0, 1], port)))
            .collect();
        let found = expected.clone();
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let trackers = async move {
            for peer in found {
                tx.send(peer).await.unwrap();
            }
        };
        let stream = TrackerPeers {
            trackers: Some(trackers.boxed()),
            peers: rx,
        };
        assert_eq!(stream.collect::<Vec<_>>().await, expected);
    }

    #[tokio::test]
    async fn test_dropping_the_stream_stops_the_trackers() {
        let (_tx, rx) = tokio::sync::mpsc::channel(16);
        let (running, stopped) = tokio::sync::oneshot::channel::<()>();
        let trackers = async move {
            let _running = running;
            std::future::pending::<()>().await;
        };
        let mut stream = TrackerPeers {
            trackers: Some(trackers.boxed()),
            peers: rx,
        };
        assert!(stream.next().now_or_never().is_none());
        drop(stream);
        assert!(stopped.await.is_err());
    }
}

#[cfg(test)]
mod local_address_tests {
    use super::*;

    #[test]
    fn local_addresses_are_recognized() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "198.18.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fc00::1",
            "fe80::1",
            "ff02::1",
            "::ffff:192.168.1.1",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_local_address(address.parse().unwrap()), "{address}");
        }
        for address in ["93.184.216.34", "1.1.1.1", "2606:4700::1111"] {
            assert!(!is_local_address(address.parse().unwrap()), "{address}");
        }
    }

    #[tokio::test]
    async fn test_a_name_for_this_computer_is_refused_when_only_public_addresses_count() {
        let host = url::Host::Domain("localhost");
        assert!(
            udp_tracker_to_socket_addrs(host.clone(), 80, false)
                .await
                .is_ok()
        );
        assert!(udp_tracker_to_socket_addrs(host, 80, true).await.is_err());
    }
}
