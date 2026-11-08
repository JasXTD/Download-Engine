//! Harmless local sockets only; executed inside the disposable Docker lab.
use crate::{
    details::PeerTransport,
    mse::Encryption,
    network::Binding,
    session::{AddOptions, Session, SessionOptions},
    testkit::*,
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::time::timeout;

fn loopback() -> Binding {
    Binding {
        interface: if cfg!(target_os = "macos") {
            "lo0"
        } else {
            "lo"
        }
        .into(),
        address: "127.0.0.1".parse().unwrap(),
        dns: "127.0.0.1".parse().unwrap(),
    }
}

#[tokio::test]
async fn missing_or_mismatched_interface_is_rejected_before_session_start() {
    for binding in [
        Binding {
            interface: "missing-de-test".into(),
            ..loopback()
        },
        Binding {
            address: "192.0.2.99".parse().unwrap(),
            ..loopback()
        },
    ] {
        assert!(
            Session::new(SessionOptions {
                binding: Some(binding),
                use_dht: true,
                use_trackers: true,
                use_utp: true,
                web_seeds: true,
                ..Default::default()
            })
            .await
            .is_err()
        );
    }
    assert!(
        Session::new(SessionOptions {
            binding: Some(loopback()),
            upnp: true,
            listen_port: Some(6881),
            ..Default::default()
        })
        .await
        .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bound_tcp_reports_negotiation_and_keeps_saved_progress() {
    for (address, encryption) in [
        ("127.0.0.1", Encryption::Disabled),
        ("127.0.0.1", Encryption::Required),
        ("::1", Encryption::Required),
    ] {
        let address: IpAddr = address.parse().unwrap();
        let binding = Binding {
            address,
            dns: address,
            ..loopback()
        };
        let source = TempFolder::new();
        let output = TempFolder::new();
        let data = sample_bytes(512 * 1024, 912);
        let metainfo = single_file_torrent("harmless.fixture", &data, 16384);
        std::fs::write(source.path().join("harmless.fixture"), &data).unwrap();
        let port = {
            let listener = tokio::net::TcpListener::bind(SocketAddr::new(address, 0))
                .await
                .unwrap();
            listener.local_addr().unwrap().port()
        };
        let seed = Session::new(SessionOptions {
            binding: Some(binding.clone()),
            encryption,
            seed: true,
            listen_port: Some(port),
            upload_limit: Some(128 * 1024),
            allow_loopback_peers: true,
            ..Default::default()
        })
        .await
        .unwrap();
        seed.add_torrent_file(
            "seed".into(),
            &metainfo,
            AddOptions {
                output: source.path(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        wait_until(&seed.torrent("seed").unwrap(), "seed", is_done).await;
        let client = Session::new(SessionOptions {
            binding: Some(binding),
            encryption,
            allow_loopback_peers: true,
            state_dir: Some(output.path().join("resume")),
            ..Default::default()
        })
        .await
        .unwrap();
        client
            .add_torrent_file(
                "download".into(),
                &metainfo,
                AddOptions {
                    output: output.path(),
                    peers: vec![SocketAddr::new(address, port)],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        timeout(Duration::from_secs(20), async {
            loop {
                let progress = client.stats().torrents["download"].progress_bytes;
                if progress > 0 && progress < data.len() as u64 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        for (session, name) in [(&client, "download"), (&seed, "seed")] {
            let details = session.torrent(name).unwrap().details();
            assert!(!details.peers.is_empty());
            for peer in details.peers {
                assert_eq!(peer.connection.transport, PeerTransport::Tcp);
                assert_eq!(
                    peer.connection.encrypted,
                    Some(encryption == Encryption::Required)
                );
            }
        }
        client.pause("download").await.unwrap();
        let progress = client.stats().torrents["download"].progress_bytes;
        let paused = std::fs::read(output.path().join("harmless.fixture")).unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            std::fs::read(output.path().join("harmless.fixture")).unwrap(),
            paused
        );
        client.resume("download").await.unwrap();
        // This client does not seed: completion can transition from live to
        // finished between polls. Wait for verified completion, not that instant.
        wait_until(&client.torrent("download").unwrap(), "resumed", |stats| {
            stats.finished
        })
        .await;
        assert!(progress > 0);
        assert_eq!(
            std::fs::read(output.path().join("harmless.fixture")).unwrap(),
            data
        );
        client.stop_all().await;
        client.shutdown().await;
        seed.shutdown().await;
    }
}

/// Opt-in real two-interface test; the separate peer container records every
/// request. These addresses are created by the checked-in lab runner only.
#[tokio::test]
async fn bound_http_dns_udp_and_no_secondary_interface_fallback() {
    let Ok(interface) = std::env::var("DE_BINDING_LAB_INTERFACE") else {
        return;
    };
    let binding = Binding {
        interface,
        address: "172.30.241.2".parse().unwrap(),
        dns: "172.30.241.3".parse().unwrap(),
    };
    let device = Arc::new(binding.open().unwrap());
    let http = crate::network::BoundHttp::new(device.clone(), true).unwrap();
    let request = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get("http://fixture.test:8080/data")
        .build()
        .unwrap();
    let response = http.send(request).await.unwrap();
    assert_eq!(
        response.bytes().await.unwrap().as_ref(),
        b"Download Engine harmless network fixture\n"
    );
    // The lab supplies its one-day certificate via SSL_CERT_FILE for this process
    // only. Hostname verification must still reject a different DNS name.
    if std::env::var_os("SSL_CERT_FILE").is_some() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        assert!(
            http.send(
                client
                    .get("https://fixture.test:8443/data")
                    .build()
                    .unwrap()
            )
            .await
            .unwrap()
            .status()
            .is_success()
        );
        assert!(
            http.send(
                client
                    .get("https://wrong.fixture.test:8443/data")
                    .build()
                    .unwrap()
            )
            .await
            .is_err()
        );
    }
    // The DNS fixture forces truncation for this name; TCP DNS must stay bound too.
    let answers = librqbit_dualstack_sockets::dns::lookup(Some(&device), "tcp.fixture.test", 8080)
        .await
        .unwrap();
    assert_eq!(
        answers,
        vec!["172.30.241.3:8080".parse::<SocketAddr>().unwrap()]
    );
    let udp = librqbit_dualstack_sockets::UdpSocket::bind_udp(
        device.address(0),
        librqbit_dualstack_sockets::BindOpts {
            device: Some(&device),
            ..Default::default()
        },
    )
    .unwrap();
    udp.send_to(b"harmless", "172.30.241.3:9000".parse().unwrap())
        .await
        .unwrap();
    let mut bytes = [0; 32];
    let (len, addr) = timeout(Duration::from_secs(2), udp.recv_from(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes[..len], b"harmless");
    assert_eq!(addr.ip(), "172.30.241.3".parse::<IpAddr>().unwrap());
    // An unbound control proves the other interface and endpoint really are reachable.
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    assert!(
        client
            .get("http://172.30.240.3:8080/control")
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let request = client
        .get("http://172.30.240.3:8080/forbidden")
        .build()
        .unwrap();
    assert!(
        timeout(Duration::from_secs(8), http.send(request))
            .await
            .unwrap()
            .is_err()
    );
    let other_dns = librqbit_dualstack_sockets::BindDevice::new(
        &binding.interface,
        binding.address,
        Some("172.30.240.3:53".parse().unwrap()),
    )
    .unwrap();
    assert!(
        librqbit_dualstack_sockets::dns::lookup(Some(&other_dns), "no-fallback.test", 80)
            .await
            .is_err()
    );
    assert!(
        udp.send_to(b"forbidden", "172.30.240.3:9000".parse().unwrap())
            .await
            .is_err()
            || timeout(Duration::from_millis(200), udp.recv_from(&mut bytes))
                .await
                .is_err()
    );
    let folder = TempFolder::new();
    let session = Session::new(SessionOptions {
        binding: Some(binding),
        use_dht: true,
        dht_bootstrap: Some(vec!["fixture.test:9001".into()]),
        allow_loopback_peers: true,
        ..Default::default()
    })
    .await
    .unwrap();
    session
        .add_torrent_file(
            "dht".into(),
            &single_file_torrent("dht.fixture", &sample_bytes(32768, 4), 16384),
            AddOptions {
                output: folder.path(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    session.shutdown().await;
}
