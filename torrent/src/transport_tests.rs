//! Real socket checks, run only in the disposable network-none Linux container.
#![cfg(unix)]
use crate::{
    mse::{self, Encryption},
    session::{AddOptions, Session, SessionOptions},
    testkit::*,
    torrent::Torrent,
    transport::Peers,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

async fn port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
async fn done(session: &Session, name: &str) {
    timeout(Duration::from_secs(30), async {
        loop {
            let stats = session.stats();
            let job = &stats.torrents[name];
            assert!(job.error.is_none(), "{:?}", job.error);
            if job.finished {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mse_over_utp_fallback_pauses_resumes_and_releases_udp_port() {
    let source = TempFolder::new();
    let output = TempFolder::new();
    let data = sample_bytes(2 * 1024 * 1024, 332);
    let torrent = single_file_torrent("test.bin", &data, 32768);
    std::fs::write(source.path().join("test.bin"), &data).unwrap();
    let meta = meta_of(&torrent);
    let seed = Torrent::from_meta(
        meta.clone(),
        options_for(source.path()),
        limited_context(30, Some(512 * 1024)),
    )
    .unwrap();
    seed.start().await;
    wait_until(&seed, "seed", is_done).await;
    let incoming = port().await;
    let cancel = CancellationToken::new();
    let server = Peers::new(
        Encryption::Required,
        true,
        Some(incoming),
        true,
        cancel.clone(),
    )
    .await
    .unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let listener = tokio::spawn({
        let server = server.clone();
        let seed = seed.clone();
        let cancel = cancel.clone();
        let accepts = accepts.clone();
        async move {
            loop {
                let (stream, addr) = tokio::select! {biased; _ = cancel.cancelled()=>break, result=server.accept_utp()=>match result {Ok(value)=>value,Err(_) if cancel.is_cancelled()=>break,Err(error)=>panic!("{error}")}};
                let result = tokio::select! {_ = cancel.cancelled()=>break,result=mse::incoming(stream,Encryption::Required,vec![seed.info_hash()])=>result};
                if let Ok((mut stream, hash)) = result {
                    let (plain_hash, hello) =
                        crate::peer::read_handshake(&mut stream).await.unwrap();
                    assert_eq!(hash, Some(plain_hash));
                    accepts.fetch_add(1, Ordering::Relaxed);
                    seed.offer(stream, addr, hello).unwrap();
                }
            }
        }
    });
    // No TCP listener exists at this port: the production connector must fall back to uTP.
    let session = Session::new(SessionOptions {
        use_utp: true,
        encryption: Encryption::Required,
        allow_loopback_peers: true,
        ..Default::default()
    })
    .await
    .unwrap();
    session
        .add_torrent_file(
            "download".into(),
            &torrent,
            AddOptions {
                output: output.path(),
                peers: vec![format!("127.0.0.1:{incoming}").parse().unwrap()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    timeout(Duration::from_secs(20), async {
        while session.stats().torrents["download"].progress_bytes == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    session.pause("download").await.unwrap();
    let paused = session.stats().torrents["download"].progress_bytes;
    assert!(paused > 0 && paused < data.len() as u64);
    let snapshot = std::fs::read(output.path().join("test.bin")).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        std::fs::read(output.path().join("test.bin")).unwrap(),
        snapshot
    );
    session.resume("download").await.unwrap();
    done(&session, "download").await;
    assert_eq!(std::fs::read(output.path().join("test.bin")).unwrap(), data);
    assert!(accepts.load(Ordering::Relaxed) >= 2);
    session.stop_all().await;
    session.shutdown().await;
    seed.pause().await;
    cancel.cancel();
    listener.await.unwrap();
    server.shutdown().await;
    let _rebound = std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, incoming)).unwrap();
}

#[tokio::test]
async fn prefer_uses_a_fresh_plain_connection_required_never_downgrades() {
    for mode in [Encryption::Prefer, Encryption::Required] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let task = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            seen.fetch_add(1, Ordering::Relaxed);
            let mut dh = [0; 96];
            first.read_exact(&mut dh).await.unwrap();
            drop(first);
            if let Ok(Ok((mut second, _))) =
                timeout(Duration::from_millis(500), listener.accept()).await
            {
                seen.fetch_add(1, Ordering::Relaxed);
                let mut text = [0; 5];
                second.read_exact(&mut text).await.unwrap();
                assert_eq!(&text, b"plain");
            }
        });
        let peers = Peers::new(mode, false, None, true, CancellationToken::new())
            .await
            .unwrap();
        let result = peers
            .connect(addr, librqbit_core::hash_id::Id20::new([8; 20]), false)
            .await;
        if mode == Encryption::Prefer {
            result.unwrap().write_all(b"plain").await.unwrap();
        } else {
            assert!(result.is_err());
        }
        task.await.unwrap();
        assert_eq!(
            count.load(Ordering::Relaxed),
            if mode == Encryption::Prefer { 2 } else { 1 }
        );
        peers.shutdown().await;
    }
}

#[tokio::test]
async fn utp_shutdown_and_restart_with_outgoing_only_policy() {
    for _ in 0..3 {
        let port = port().await;
        let session = Session::new(SessionOptions {
            use_utp: true,
            listen_port: Some(port),
            allow_loopback_peers: true,
            ..Default::default()
        })
        .await
        .unwrap();
        let mut interrupted = Box::pin(session.shutdown());
        std::future::poll_fn(|cx| {
            let _ = interrupted.as_mut().poll(cx);
            std::task::Poll::Ready(())
        })
        .await;
        drop(interrupted);
        tokio::join!(session.shutdown(), session.shutdown());
        let _udp = std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).unwrap();
        let _tcp = std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).unwrap();
    }
}

#[tokio::test]
async fn a_failed_tcp_bind_releases_the_utp_port_before_returning() {
    let occupied = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = occupied.local_addr().unwrap().port();
    assert!(
        Session::new(SessionOptions {
            use_utp: true,
            listen_port: Some(port),
            ..Default::default()
        })
        .await
        .is_err()
    );
    let _udp = std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).unwrap();
}
