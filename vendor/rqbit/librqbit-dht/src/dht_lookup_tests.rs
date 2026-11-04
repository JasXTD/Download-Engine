// Added by Download Engine: socket-free lookup security regressions. See ../../patch.md.
use super::*;

type Lookup = RecursiveRequest<RecursiveRequestCallbacksGetPeers>;

fn fixture() -> (
    Arc<Lookup>,
    Receiver<PendingNode>,
    Receiver<SocketAddr>,
    UnboundedReceiver<WorkerSendRequest>,
) {
    let (worker_tx, worker_rx) = unbounded_channel();
    let id = Id20::new([0; 20]);
    // Deliberately never construct DhtWorker, bind a socket, or resolve a host.
    let dht = Arc::new(DhtState::new_internal(
        id,
        worker_tx,
        None,
        None,
        "0.0.0.0:6881".parse().unwrap(),
        PeerStore::new(id),
        CancellationToken::new(),
    ));
    let (lookup, node_rx, peer_rx) = lookup_fixture(dht);
    (lookup, node_rx, peer_rx, worker_rx)
}

fn lookup_fixture(
    dht: Arc<DhtState>,
) -> (Arc<Lookup>, Receiver<PendingNode>, Receiver<SocketAddr>) {
    let id = Id20::new([0; 20]);
    let (peer_tx, peer_rx) = channel(MAX_QUEUED_PEERS);
    let (node_tx, node_rx) = channel(MAX_PENDING_NODES);
    let lookup = Arc::new(RecursiveRequest {
        max_depth: 4,
        useful_nodes_limit: 256,
        info_hash: id,
        request: Request::GetPeers(id),
        dht,
        useful_nodes: RwLock::new(Vec::new()),
        peer_tx,
        node_tx,
        pending_nodes: Default::default(),
        callbacks: RecursiveRequestCallbacksGetPeers {
            min_distance_to_announce: id,
            announce_port: None,
        },
    });
    (lookup, node_rx, peer_rx)
}

fn address(sequence: u16, is_v4: bool) -> SocketAddr {
    let ip = if is_v4 { "203.0.113.7" } else { "2001:db8::7" };
    SocketAddr::new(ip.parse().unwrap(), 10_000 + sequence)
}

fn closer_id(sequence: u64) -> Id20 {
    let mut id = [0; 20];
    id[12..].copy_from_slice(&(1_000_000 - sequence).to_be_bytes());
    Id20::new(id)
}

fn reply(
    dht: &Arc<DhtState>,
    outgoing: WorkerSendRequest,
    nodes: &[(Id20, SocketAddr)],
    peers: &[SocketAddr],
) {
    let response = bprotocol::Response {
        id: Id20::new([1; 20]),
        nodes: Some(CompactNodeInfo::new_from_iter(nodes.iter().filter_map(
            |&(id, addr)| match addr {
                SocketAddr::V4(addr) => Some(Node { id, addr }),
                _ => None,
            },
        ))),
        nodes6: Some(CompactNodeInfo::new_from_iter(nodes.iter().filter_map(
            |&(id, addr)| match addr {
                SocketAddr::V6(addr) => Some(Node { id, addr }),
                _ => None,
            },
        ))),
        values: (!peers.is_empty()).then(|| peers.iter().copied().map(Into::into).collect()),
        token: None,
    };
    let mut wire = Vec::new();
    bprotocol::serialize_message(
        &mut wire,
        outgoing.message.transaction_id,
        None,
        None,
        MessageKind::Response(response),
    )
    .unwrap();
    assert!(
        wire.len() < 16_384,
        "fits the production UDP receive buffer"
    );
    let decoded = bprotocol::deserialize_message::<ByteBufOwned>(&wire).unwrap();
    dht.on_received_message(decoded, outgoing.addr).unwrap();
}

#[tokio::test(start_paused = true)]
async fn decoded_response_flood_keeps_nodes_and_peers_bounded() {
    for is_v4 in [true, false] {
        let (lookup, mut nodes_rx, peers_rx, mut worker_rx) = fixture();
        assert!(lookup.enqueue_node(None, address(5000, is_v4), 0).unwrap());
        let mut target = nodes_rx.try_recv().unwrap();
        let mut sequence = 0u16;
        for round in 0..32 {
            let response = async {
                let outgoing = worker_rx.recv().await.unwrap();
                let nodes = (0..128)
                    .map(|_| {
                        let node = (closer_id(u64::from(sequence)), address(sequence, is_v4));
                        sequence += 1;
                        node
                    })
                    .collect::<Vec<_>>();
                let peers = (6000..6200).map(|n| address(n, is_v4)).collect::<Vec<_>>();
                reply(&lookup.dht, outgoing, &nodes, &peers);
            };
            let (result, ()) = tokio::join!(lookup.request_one(target), response);
            result.unwrap();
            assert!(lookup.pending_nodes.lock().len() <= MAX_PENDING_NODES);
            assert!(nodes_rx.len() <= MAX_PENDING_NODES);
            assert!(peers_rx.len() <= MAX_QUEUED_PEERS);
            assert!(lookup.dht.inflight_by_transaction_id.lock().is_empty());
            if round == 31 {
                break;
            }
            target = nodes_rx.try_recv().unwrap();
        }
        assert_eq!(nodes_rx.len(), MAX_PENDING_NODES - 1);
        assert_eq!(lookup.useful_nodes.read().len(), 256);
        assert_eq!(peers_rx.len(), MAX_QUEUED_PEERS);
        println!(
            "is_v4={is_v4}, decoded_responses=32, queued_nodes={}, cached_nodes=256, queued_peers={}, sockets=0",
            nodes_rx.len(),
            peers_rx.len()
        );
        drop(nodes_rx);
        assert!(lookup.pending_nodes.lock().is_empty());
    }
}

#[test]
fn pending_duplicates_survive_cache_eviction_and_release_on_drop() {
    let (mut lookup, mut nodes_rx, _peers_rx, _worker_rx) = fixture();
    Arc::get_mut(&mut lookup).unwrap().useful_nodes_limit = 1;
    let first = address(0, true);
    assert!(lookup.enqueue_node(Some(closer_id(0)), first, 1).unwrap());
    let active = nodes_rx.try_recv().unwrap();
    assert!(
        lookup
            .enqueue_node(Some(closer_id(1)), address(1, true), 1)
            .unwrap()
    );
    assert_eq!(lookup.useful_nodes.read()[0].id, closer_id(1));
    // Neither a changed ID at the same endpoint nor the same ID at a different
    // endpoint may create another pending request, even after cache eviction.
    assert!(!lookup.enqueue_node(Some(closer_id(2)), first, 1).unwrap());
    assert!(
        !lookup
            .enqueue_node(Some(closer_id(0)), address(2, true), 1)
            .unwrap()
    );
    assert!(!lookup.enqueue_node(None, first, 0).unwrap());
    assert_eq!(lookup.pending_nodes.lock().len(), 2);
    drop(active);
    assert!(lookup.enqueue_node(Some(closer_id(0)), first, 0).unwrap());
    drop(nodes_rx);
    assert!(lookup.pending_nodes.lock().is_empty());
}

#[tokio::test(start_paused = true)]
async fn decoded_discoveries_reject_local_use_translation_before_admission() {
    let (lookup, mut nodes_rx, mut peers_rx, mut worker_rx) = fixture();
    assert!(lookup.enqueue_node(None, address(0, false), 0).unwrap());
    let target = nodes_rx.try_recv().unwrap();
    let public = "[2606:4700::1111]:6881".parse().unwrap();
    let local = "[64:ff9b:1::c0a8:101]:6881".parse().unwrap();
    let response = async {
        let outgoing = worker_rx.recv().await.unwrap();
        reply(
            &lookup.dht,
            outgoing,
            &[(closer_id(0), local), (closer_id(1), public)],
            &[local, public],
        );
    };
    let (result, ()) = tokio::join!(lookup.request_one(target), response);
    result.unwrap();
    assert_eq!(peers_rx.try_recv().unwrap(), public);
    assert!(peers_rx.try_recv().is_err());
    assert_eq!(nodes_rx.try_recv().unwrap().addr, public);
    assert!(nodes_rx.try_recv().is_err());
    assert!(lookup.pending_nodes.lock().is_empty());
}

#[test]
fn admission_counts_active_work_and_does_not_poison_retries_when_full() {
    let (lookup, mut nodes_rx, _peers_rx, _worker_rx) = fixture();
    for sequence in 0..MAX_PENDING_NODES as u16 {
        assert!(
            lookup
                .enqueue_node(None, address(sequence, true), 0)
                .unwrap()
        );
    }
    let candidate = (Some(closer_id(1000)), address(1000, true), 1);
    assert!(
        !lookup
            .enqueue_node(candidate.0, candidate.1, candidate.2)
            .unwrap()
    );
    let active = nodes_rx.try_recv().unwrap();
    // Dequeueing alone does not free the admission slot.
    assert!(
        !lookup
            .enqueue_node(candidate.0, candidate.1, candidate.2)
            .unwrap()
    );
    assert!(lookup.useful_nodes.read().is_empty());
    drop(active);
    assert!(
        lookup
            .enqueue_node(candidate.0, candidate.1, candidate.2)
            .unwrap()
    );
    assert_eq!(lookup.useful_nodes.read().len(), 1);
    drop(nodes_rx);
    assert!(lookup.pending_nodes.lock().is_empty());
    assert!(matches!(
        lookup.enqueue_node(None, address(2000, true), 0),
        Err(Error::ReceiverDead)
    ));
}

#[test]
fn concurrent_lookups_share_admission_and_return_it_when_dropped() {
    let (first, _nodes_rx, _peers_rx, _worker_rx) = fixture();
    let dht = &first.dht;
    let mut lookups = Vec::new();
    for _ in 0..MAX_PENDING_DHT_NODES / MAX_PENDING_NODES {
        let (lookup, nodes_rx, peers_rx) = lookup_fixture(dht.clone());
        for sequence in 0..MAX_PENDING_NODES as u16 {
            assert!(
                lookup
                    .enqueue_node(None, address(sequence, true), 0)
                    .unwrap()
            );
        }
        lookups.push((lookup, nodes_rx, peers_rx));
    }
    assert_eq!(dht.lookup_budget.available_permits(), 0);
    assert!(
        !first
            .enqueue_node(Some(closer_id(0)), address(0, true), 1)
            .unwrap()
    );
    assert!(first.useful_nodes.read().is_empty());
    drop(lookups.pop());
    assert_eq!(dht.lookup_budget.available_permits(), MAX_PENDING_NODES);
    assert!(
        first
            .enqueue_node(Some(closer_id(0)), address(0, true), 1)
            .unwrap()
    );
    drop(lookups);
    drop(_nodes_rx);
    assert_eq!(dht.lookup_budget.available_permits(), MAX_PENDING_DHT_NODES);
}

#[tokio::test(start_paused = true)]
async fn cancellation_releases_pending_and_response_tracking() {
    for wait_for_rate_limit in [false, true] {
        let (lookup, mut nodes_rx, _peers_rx, mut worker_rx) = fixture();
        if wait_for_rate_limit {
            // Exhaust the initial refill so cancellation also covers requests
            // waiting before they register a transaction or send a packet.
            for _ in 0..25 {
                lookup.dht.rate_limiter.acquire_one().await;
            }
        }
        assert!(lookup.enqueue_node(None, address(0, true), 0).unwrap());
        let mut request = Box::pin(lookup.request_one(nodes_rx.try_recv().unwrap()));
        assert!(request.as_mut().now_or_never().is_none());
        assert_eq!(worker_rx.try_recv().is_ok(), !wait_for_rate_limit);
        assert_eq!(lookup.pending_nodes.lock().len(), 1);
        assert_eq!(
            lookup.dht.inflight_by_transaction_id.lock().len(),
            usize::from(!wait_for_rate_limit)
        );
        drop(request);
        assert!(lookup.pending_nodes.lock().is_empty());
        assert!(lookup.dht.inflight_by_transaction_id.lock().is_empty());
        assert_eq!(
            lookup.dht.lookup_budget.available_permits(),
            MAX_PENDING_DHT_NODES
        );
        assert!(lookup.enqueue_node(None, address(0, true), 0).unwrap());
    }
}

#[tokio::test(start_paused = true)]
async fn timeout_and_send_failure_release_admission() {
    let (lookup, mut nodes_rx, _peers_rx, worker_rx) = fixture();
    assert!(lookup.enqueue_node(None, address(0, true), 0).unwrap());
    let result = lookup.request_one(nodes_rx.try_recv().unwrap()).await;
    assert!(matches!(result, Err(Error::ResponseTimeout(_))));
    assert!(lookup.pending_nodes.lock().is_empty());
    assert!(lookup.dht.inflight_by_transaction_id.lock().is_empty());
    drop(worker_rx);
    assert!(lookup.enqueue_node(None, address(0, true), 0).unwrap());
    let result = lookup.request_one(nodes_rx.try_recv().unwrap()).await;
    assert!(matches!(result, Err(Error::DhtDead)));
    assert!(lookup.pending_nodes.lock().is_empty());
    assert!(lookup.dht.inflight_by_transaction_id.lock().is_empty());
}

#[tokio::test(start_paused = true)]
async fn peer_lookup_scheduler_limits_futures_and_cleans_up_when_aborted() {
    for is_v4 in [true, false] {
        let (lookup, nodes_rx, _peers_rx, mut worker_rx) = fixture();
        for sequence in 0..MAX_PENDING_NODES as u16 {
            assert!(
                lookup
                    .enqueue_node(None, address(sequence, is_v4), 0)
                    .unwrap()
            );
        }
        let handle = lookup.request_peers_forever(nodes_rx, is_v4);
        let mut outgoing = Vec::new();
        for _ in 0..MAX_CONCURRENT_LOOKUP_REQUESTS {
            outgoing.push(worker_rx.recv().await.unwrap());
        }
        tokio::task::yield_now().await;
        assert!(worker_rx.try_recv().is_err());
        assert_eq!(
            lookup.dht.inflight_by_transaction_id.lock().len(),
            MAX_CONCURRENT_LOOKUP_REQUESTS
        );
        assert_eq!(lookup.pending_nodes.lock().len(), MAX_PENDING_NODES);
        assert_eq!(lookup.node_tx.capacity(), MAX_CONCURRENT_LOOKUP_REQUESTS);
        reply(&lookup.dht, outgoing.remove(0), &[], &[]);
        // Completing a request permits another queued node to run.
        worker_rx.recv().await.unwrap();
        handle.abort();
        assert!(handle.await.unwrap_err().is_cancelled());
        assert!(lookup.pending_nodes.lock().is_empty());
        assert!(lookup.dht.inflight_by_transaction_id.lock().is_empty());
        assert_eq!(
            lookup.dht.lookup_budget.available_permits(),
            MAX_PENDING_DHT_NODES
        );
    }
}

#[tokio::test(start_paused = true)]
async fn bootstrap_responder_enters_the_table_without_a_duplicate_request() {
    let (lookup, _nodes_rx, _peers_rx, mut worker_rx) = fixture();
    let bootstrap = address(0, true);
    let responder_id = Id20::new([1; 20]);
    let request = RecursiveRequest::find_node_for_routing_table(
        lookup.dht.clone(),
        Id20::new([0; 20]),
        std::iter::once(bootstrap),
    );
    let response = async {
        let outgoing = worker_rx.recv().await.unwrap();
        reply(&lookup.dht, outgoing, &[(responder_id, bootstrap)], &[]);
    };
    let (result, ()) = tokio::join!(request, response);
    result.unwrap();
    assert!(worker_rx.try_recv().is_err());
    assert!(
        lookup
            .dht
            .routing_table_v4
            .read()
            .sorted_by_distance_from(responder_id, now())
            .iter()
            .any(|node| node.id() == responder_id && node.addr() == bootstrap)
    );
    assert!(lookup.dht.inflight_by_transaction_id.lock().is_empty());
    assert_eq!(
        lookup.dht.lookup_budget.available_permits(),
        MAX_PENDING_DHT_NODES
    );
}

#[tokio::test(start_paused = true)]
async fn routing_lookup_bounds_initial_work_and_drains_queued_nodes() {
    let (lookup, _nodes_rx, _peers_rx, mut worker_rx) = fixture();
    let addrs = (0..MAX_PENDING_NODES as u16 + 40).map(|n| address(n, true));
    let mut request = Box::pin(RecursiveRequest::find_node_for_routing_table(
        lookup.dht.clone(),
        Id20::new([0; 20]),
        addrs,
    ));
    let mut count = 0;
    let mut peak_active = 0;
    loop {
        tokio::select! {
            result = &mut request => { result.unwrap(); break; }
            Some(outgoing) = worker_rx.recv() => {
                peak_active = peak_active.max(lookup.dht.inflight_by_transaction_id.lock().len());
                assert!(peak_active <= MAX_CONCURRENT_LOOKUP_REQUESTS);
                reply(&lookup.dht, outgoing, &[], &[]);
                count += 1;
            }
        }
    }
    assert_eq!(count, MAX_PENDING_NODES);
    assert_eq!(peak_active, MAX_CONCURRENT_LOOKUP_REQUESTS);
    assert!(lookup.dht.inflight_by_transaction_id.lock().is_empty());
}

#[tokio::test(start_paused = true)]
async fn routing_lookup_finishes_after_discovered_nodes_and_depth_limit() {
    let (lookup, _nodes_rx, _peers_rx, mut worker_rx) = fixture();
    let mut request = Box::pin(RecursiveRequest::find_node_for_routing_table(
        lookup.dht.clone(),
        Id20::new([0; 20]),
        std::iter::once(address(0, true)),
    ));
    let mut count = 0;
    loop {
        tokio::select! {
            result = &mut request => { result.unwrap(); break; }
            Some(outgoing) = worker_rx.recv() => {
                count += 1;
                reply(&lookup.dht, outgoing, &[(closer_id(count), address(count as u16, true))], &[]);
            }
        }
    }
    assert_eq!(count, 5, "root and four recursive levels, then stop");
    assert!(lookup.dht.inflight_by_transaction_id.lock().is_empty());
}
