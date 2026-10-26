use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

async fn headers(socket: &mut TcpStream) -> String {
    tokio::time::timeout(Duration::from_secs(8), async {
        let mut data = Vec::new();
        while !data.ends_with(b"\r\n\r\n") {
            data.push(socket.read_u8().await.unwrap());
            assert!(data.len() < 64 * 1024);
        }
        String::from_utf8(data).unwrap()
    })
    .await
    .unwrap()
}
async fn response(socket: &mut TcpStream, status: u16, challenge: &str, body: &[u8]) {
    socket
        .write_all(
            format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\n{challenge}\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    socket.write_all(body).await.unwrap();
}
// MS-NLMP 2.2.1.2: synthetic server challenge, not a real account or credential.
fn challenge() -> Vec<u8> {
    let target: Vec<u8> = "FIXTURE"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut info = Vec::new();
    for id in [1u16, 2] {
        info.extend(id.to_le_bytes());
        info.extend((target.len() as u16).to_le_bytes());
        info.extend(&target);
    }
    info.extend([0u8; 4]);
    let mut bytes = b"NTLMSSP\0".to_vec();
    bytes.extend(2u32.to_le_bytes());
    bytes.extend((target.len() as u16).to_le_bytes());
    bytes.extend((target.len() as u16).to_le_bytes());
    bytes.extend(56u32.to_le_bytes());
    bytes.extend(0xa08a8205u32.to_le_bytes());
    bytes.extend([1, 2, 3, 4, 5, 6, 7, 8]);
    bytes.extend([0; 8]);
    bytes.extend((info.len() as u16).to_le_bytes());
    bytes.extend((info.len() as u16).to_le_bytes());
    bytes.extend((56 + target.len() as u32).to_le_bytes());
    bytes.extend([0; 8]);
    bytes.extend(target);
    bytes.extend(info);
    bytes
}
fn ntlm_type(request: &str, header: &str) -> Option<u32> {
    request.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.eq_ignore_ascii_case(header) {
            return None;
        }
        let value = value.trim();
        let value = value
            .strip_prefix("NTLM ")
            .or_else(|| value.strip_prefix("Negotiate "))?;
        let bytes = STANDARD.decode(value).unwrap();
        assert_eq!(&bytes[..8], b"NTLMSSP\0");
        Some(u32::from_le_bytes(bytes[8..12].try_into().unwrap()))
    })
}
fn assert_ntlm_user(request: &str, header: &str, username: &str) {
    let token = request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case(header)
                .then_some(value.trim().split_once(' ')?.1)
        })
        .unwrap();
    let token = STANDARD.decode(token).unwrap();
    let length = u16::from_le_bytes(token[36..38].try_into().unwrap()) as usize;
    let offset = u32::from_le_bytes(token[40..44].try_into().unwrap()) as usize;
    let user: Vec<_> = token[offset..offset + length]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes(*b))
        .collect();
    assert!(
        String::from_utf16(&user)
            .unwrap()
            .eq_ignore_ascii_case(username),
        "The handshake must use only the supplied synthetic account"
    );
}
fn login(kind: Kind) -> Arc<Credentials> {
    Arc::new(Credentials::enterprise(
        kind,
        "FIXTURE\\synthetic-user".into(),
        "test-only-password".into(),
    ))
}
fn client() -> reqwest::Client {
    crate::engine::client_builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

#[tokio::test]
async fn ntlm_origin_and_proxy_handshakes_stream_the_requested_range() {
    for kind in [Kind::Ntlm, Kind::Negotiate] {
        for proxy in [false, true] {
            let method = if kind == Kind::Ntlm {
                "NTLM"
            } else {
                "Negotiate"
            };
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (status, challenge_name, auth) = if proxy {
                    (407, "Proxy-Authenticate", "Proxy-Authorization")
                } else {
                    (401, "WWW-Authenticate", "Authorization")
                };
                let first = headers(&mut socket).await;
                if ntlm_type(&first, auth).is_none() {
                    response(
                        &mut socket,
                        status,
                        &format!("{challenge_name}: {method}\r\n"),
                        b"",
                    )
                    .await;
                    let second = headers(&mut socket).await;
                    assert_eq!(ntlm_type(&second, auth), Some(1));
                } else {
                    assert_eq!(ntlm_type(&first, auth), Some(1));
                }
                response(
                    &mut socket,
                    status,
                    &format!(
                        "{challenge_name}: {method} {}\r\n",
                        STANDARD.encode(challenge())
                    ),
                    b"",
                )
                .await;
                let third = headers(&mut socket).await;
                assert_eq!(ntlm_type(&third, auth), Some(3));
                assert_ntlm_user(&third, auth, "synthetic-user");
                assert!(third.to_ascii_lowercase().contains("range: bytes=7-11"));
                assert_eq!(third.to_ascii_lowercase().matches("range:").count(), 1);
                assert!(!third.contains("test-only-password"));
                if proxy {
                    assert!(
                        !third
                            .lines()
                            .any(|l| l.to_ascii_lowercase().starts_with("authorization:"))
                    );
                }
                response(
                    &mut socket,
                    206,
                    "Content-Range: bytes 7-11/12\r\n",
                    b"hello",
                )
                .await;
            });
            let route = if proxy {
                Route {
                    settings: crate::proxy::Settings {
                        mode: Mode::Custom,
                        endpoint: base.clone(),
                        authentication: if kind == Kind::Ntlm {
                            Authentication::Ntlm
                        } else {
                            Authentication::Negotiate
                        },
                        ..Default::default()
                    },
                    login: Some(Arc::new(crate::proxy::Login::new(
                        "FIXTURE\\synthetic-user".into(),
                        "test-only-password".into(),
                    ))),
                }
            } else {
                Route::direct()
            };
            let url = if proxy {
                "http://public.invalid/file?kept=%2F".to_string()
            } else {
                format!("{base}/file?kept=%2F")
            };
            let http = client();
            let request = http
                .get(&url)
                .header("range", "bytes=7-11")
                .build()
                .unwrap();
            let mut result = route
                .execute(&http, request, (!proxy).then(|| login(kind)))
                .await
                .unwrap();
            assert_eq!(result.status(), 206);
            assert_eq!(result.url().as_str(), url);
            assert_eq!(result.content_length(), Some(5));
            assert_eq!(result.chunk().await.unwrap().unwrap(), b"hello"[..]);
            assert!(result.chunk().await.unwrap().is_none());
            server.await.unwrap();
        }
    }
}

#[tokio::test]
async fn enterprise_auth_does_not_downgrade_to_basic_or_follow_redirects() {
    for kind in [Kind::Ntlm, Kind::Negotiate] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = headers(&mut socket).await;
            assert!(
                !request
                    .to_ascii_lowercase()
                    .contains("authorization: basic")
            );
            response(
                &mut socket,
                401,
                "WWW-Authenticate: Basic realm=fixture\r\n",
                b"",
            )
            .await;
        });
        let http = client();
        let error = Route::direct()
            .execute(&http, http.get(url).build().unwrap(), Some(login(kind)))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("selected authentication method"));
        task.await.unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        headers(&mut socket).await;
        response(
            &mut socket,
            302,
            "Location: https://another.invalid/file\r\n",
            b"",
        )
        .await;
    });
    let http = client();
    let response = Route::direct()
        .execute(
            &http,
            http.get(url).build().unwrap(),
            Some(login(Kind::Ntlm)),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 302);
    task.await.unwrap();
}

#[tokio::test]
async fn enterprise_stream_reports_truncation_and_drop_closes_its_connection() {
    for truncate in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            headers(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\n\r\n")
                .await
                .unwrap();
            if truncate {
                socket.write_all(b"short").await.unwrap();
            } else {
                for _ in 0..32 {
                    if socket.write_all(&vec![1u8; 65536]).await.is_err() {
                        break;
                    }
                }
                let closed = tokio::time::timeout(Duration::from_secs(3), socket.read_u8()).await;
                assert!(closed.unwrap().is_err());
            }
        });
        let http = client();
        let response = Route::direct()
            .execute(
                &http,
                http.get(url).build().unwrap(),
                Some(login(Kind::Ntlm)),
            )
            .await
            .unwrap();
        if truncate {
            assert!(response.bytes().await.is_err());
        } else {
            drop(response);
        }
        task.await.unwrap();
    }
}

#[tokio::test]
async fn credentials_are_not_reused_by_another_session_at_the_same_origin() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for username in [Some("first-user"), Some("second-user"), None] {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let first = headers(&mut socket).await;
            if let Some(username) = username {
                assert_eq!(ntlm_type(&first, "Authorization"), Some(1));
                response(
                    &mut socket,
                    401,
                    &format!(
                        "WWW-Authenticate: NTLM {}\r\n",
                        STANDARD.encode(challenge())
                    ),
                    b"",
                )
                .await;
                let third = headers(&mut socket).await;
                assert_eq!(ntlm_type(&third, "Authorization"), Some(3));
                assert_ntlm_user(&third, "Authorization", username);
                response(&mut socket, 200, "", b"ok").await;
            } else {
                assert!(!first.to_ascii_lowercase().contains("authorization:"));
                response(&mut socket, 401, "WWW-Authenticate: NTLM\r\n", b"").await;
            }
            // Session closure must close its socket, even though the server
            // offered keep-alive. An anonymous request cannot inherit it.
            assert!(
                tokio::time::timeout(Duration::from_secs(3), socket.read_u8())
                    .await
                    .unwrap()
                    .is_err()
            );
        }
    });
    let http = client();
    let root = tempfile::tempdir().unwrap();
    let store = crate::store::Store::open(crate::paths::AppPaths {
        data: root.path().join("data"),
        downloads: root.path().join("files"),
    })
    .unwrap();
    store
        .save_proxy_settings(
            &crate::proxy::Settings::default(),
            crate::proxy::Settings {
                mode: Mode::Direct,
                ..Default::default()
            },
            crate::proxy::LoginChange::Remove,
        )
        .unwrap();
    struct Cleanup(crate::store::Store, String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = self.0.remove(&self.1);
        }
    }
    for username in [Some("first-user"), Some("second-user"), None] {
        let result = if let Some(username) = username {
            let mut job = crate::model::Job::new(
                &url,
                None,
                store.paths.downloads.clone(),
                uuid::Uuid::new_v4().to_string(),
            )
            .unwrap();
            job.status = crate::model::Status::Paused;
            store.enqueue(&job).unwrap();
            let cleanup = Cleanup(store.clone(), job.id.clone());
            crate::auth::apply(
                &store,
                crate::auth::Update {
                    request: uuid::Uuid::new_v4(),
                    job: job.id.clone(),
                    expected_url: url.clone(),
                    credentials: Some(Credentials::enterprise(
                        Kind::Ntlm,
                        format!("FIXTURE\\{username}"),
                        "test-only-password".into(),
                    )),
                    restart_confirmed: false,
                    resume: false,
                },
            )
            .unwrap();
            let reopened = crate::store::Store::open(store.paths.clone()).unwrap();
            let job = reopened.get(&job.id).unwrap().unwrap();
            let bound = crate::auth::DownloadClient::for_job(&reopened, &job).unwrap();
            assert!(bound.get("https://different-origin.invalid/file").is_err());
            let response = bound.get(&url).unwrap().send().await.unwrap();
            drop(cleanup);
            assert!(!crate::auth::credentials_present(job.authentication.unwrap().id).unwrap());
            response
        } else {
            send(Route::direct(), http.get(&url).build().unwrap(), None)
                .await
                .unwrap()
        };
        assert_eq!(
            result.status().as_u16(),
            if username.is_some() { 200 } else { 401 }
        );
        let _ = result.bytes().await.unwrap();
    }
    server.await.unwrap();
}

#[tokio::test]
async fn ntlm_proxy_tunnels_https_without_relaxing_certificate_validation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let first = headers(&mut socket).await;
        assert!(first.starts_with("CONNECT "));
        assert_eq!(ntlm_type(&first, "Proxy-Authorization"), Some(1));
        response(
            &mut socket,
            407,
            &format!(
                "Proxy-Authenticate: NTLM {}\r\n",
                STANDARD.encode(challenge())
            ),
            b"",
        )
        .await;
        let third = headers(&mut socket).await;
        assert_eq!(ntlm_type(&third, "Proxy-Authorization"), Some(3));
        assert!(
            !third
                .lines()
                .any(|l| l.to_ascii_lowercase().starts_with("authorization:"))
        );
        response(&mut socket, 200, "", b"").await;
        let identity = native_tls::Identity::from_pkcs12(
            include_bytes!("../../tests/fixtures/ftp/server.pfx"),
            "fixture",
        )
        .unwrap();
        let tls =
            tokio_native_tls::TlsAcceptor::from(native_tls::TlsAcceptor::new(identity).unwrap());
        // The self-signed test identity is deliberately not trusted by Windows.
        if let Ok(mut stream) = tokio::time::timeout(Duration::from_secs(5), tls.accept(socket))
            .await
            .unwrap()
        {
            // Some TLS backends finish their side of the handshake before
            // receiving the client's certificate rejection. No HTTP bytes may
            // follow it in either case.
            let mut byte = [0];
            assert!(!matches!(
                tokio::time::timeout(Duration::from_secs(5), stream.read(&mut byte))
                    .await
                    .unwrap(),
                Ok(1..)
            ));
        }
    });
    let route = Route {
        settings: crate::proxy::Settings {
            mode: Mode::Custom,
            endpoint,
            authentication: Authentication::Ntlm,
            ..Default::default()
        },
        login: Some(Arc::new(crate::proxy::Login::new(
            "FIXTURE\\synthetic-user".into(),
            "test-only-password".into(),
        ))),
    };
    let http = client();
    let error = route
        .execute(
            &http,
            http.get("https://secure-fixture.invalid/file")
                .build()
                .unwrap(),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("12175"), "{error:#}");
    server.await.unwrap();
}

#[tokio::test]
async fn concurrent_range_headers_do_not_wait_for_other_download_bodies_to_finish() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let release = Arc::new(tokio::sync::Notify::new());
    let signal = release.clone();
    let server = tokio::spawn(async move {
        let mut bodies = tokio::task::JoinSet::new();
        // More than one file's maximum number of ranges can run concurrently.
        for _ in 0..40 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let signal = signal.clone();
            bodies.spawn(async move {
                let ready = signal.notified();
                tokio::pin!(ready);
                ready.as_mut().enable();
                headers(&mut socket).await;
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n")
                    .await
                    .unwrap();
                ready.await;
                socket.write_all(b"data").await.unwrap();
            });
        }
        while let Some(body) = bodies.join_next().await {
            body.unwrap();
        }
    });
    let http = client();
    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..40 {
        let request = http.get(&url).build().unwrap();
        requests.spawn(send(Route::direct(), request, Some(login(Kind::Ntlm))));
    }
    let responses = tokio::time::timeout(Duration::from_secs(5), async {
        let mut responses = Vec::new();
        while let Some(response) = requests.join_next().await {
            responses.push(response.unwrap().unwrap());
        }
        responses
    })
    .await
    .unwrap();
    release.notify_waiters();
    for response in responses {
        assert_eq!(response.bytes().await.unwrap(), b"data"[..]);
    }
    server.await.unwrap();
}
