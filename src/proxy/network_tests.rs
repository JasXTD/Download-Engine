use super::*;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

struct Fixture {
    _root: tempfile::TempDir,
    store: Store,
}
impl Fixture {
    fn new(endpoint: &str, login: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(crate::paths::AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("files"),
        })
        .unwrap();
        store
            .save_proxy_settings(
                &Settings::default(),
                Settings {
                    mode: Mode::Custom,
                    endpoint: endpoint.into(),
                    ..Default::default()
                },
                if login {
                    LoginChange::Replace(Login::new("proxy-user".into(), "proxy-secret".into()))
                } else {
                    LoginChange::Remove
                },
            )
            .unwrap();
        Self { _root: root, store }
    }
    fn client(&self) -> reqwest::Client {
        let certificate =
            reqwest::Certificate::from_der(include_bytes!("../../tests/fixtures/ftp/server.cer"))
                .unwrap();
        configure(
            &self.store,
            crate::engine::client_builder()
                .add_root_certificate(certificate)
                .timeout(Duration::from_secs(5)),
        )
        .unwrap()
        .build()
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let settings = self.store.proxy_settings().unwrap();
        self.store
            .save_proxy_settings(&settings, Settings::default(), LoginChange::Remove)
            .unwrap();
    }
}
async fn headers(stream: &mut (impl AsyncRead + Unpin)) -> String {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        bytes.push(stream.read_u8().await.unwrap());
        assert!(bytes.len() < 16384);
    }
    String::from_utf8(bytes).unwrap()
}
fn tls() -> tokio_native_tls::TlsAcceptor {
    native_tls::TlsAcceptor::new(
        native_tls::Identity::from_pkcs12(
            include_bytes!("../../tests/fixtures/ftp/server.pfx"),
            "fixture",
        )
        .unwrap(),
    )
    .unwrap()
    .into()
}

#[cfg(windows)]
#[tokio::test]
async fn https_connect_keeps_proxy_credentials_outside_the_origin_tunnel() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture = Fixture::new(&format!("http://{}", listener.local_addr().unwrap()), true);
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let connect = headers(&mut socket).await;
        assert!(connect.starts_with("CONNECT localhost:9443 HTTP/1.1"));
        assert!(
            connect
                .to_lowercase()
                .contains("proxy-authorization: basic ")
        );
        assert!(!connect.contains("origin-secret"));
        socket
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        let mut origin = tls().accept(socket).await.unwrap();
        let request = headers(&mut origin).await.to_lowercase();
        assert!(request.starts_with("get /archive.zip"));
        assert!(request.contains("authorization: bearer origin-secret"));
        assert!(!request.contains("proxy-authorization") && !request.contains("proxy-secret"));
        origin
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    let response = fixture
        .client()
        .get("https://localhost:9443/archive.zip")
        .bearer_auth("origin-secret")
        .send()
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), "ok");
    server.await.unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn tls_proxy_encrypts_http_proxy_login_and_validates_certificates() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!(
        "https://localhost:{}",
        listener.local_addr().unwrap().port()
    );
    let fixture = Fixture::new(&address, true);
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tls().accept(socket).await.unwrap();
        let request = headers(&mut socket).await.to_lowercase();
        assert!(request.starts_with("get http://download.invalid/file"));
        assert!(request.contains("proxy-authorization: basic "));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
        // A client without the fixture-only root must reject the proxy cert.
        let (socket, _) = listener.accept().await.unwrap();
        if let Ok(mut socket) = tls().accept(socket).await {
            assert!(socket.read_u8().await.is_err());
        }
    });
    assert_eq!(
        fixture
            .client()
            .get("http://download.invalid/file")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    let client = configure(
        &fixture.store,
        crate::engine::client_builder().timeout(Duration::from_secs(3)),
    )
    .unwrap()
    .build()
    .unwrap();
    assert!(
        client
            .get("http://download.invalid/file")
            .send()
            .await
            .is_err()
    );
    server.await.unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn socks5h_uses_proxy_dns_and_username_password_without_http_auth_leaks() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture = Fixture::new(
        &format!("socks5h://{}", listener.local_addr().unwrap()),
        true,
    );
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        assert_eq!(socket.read_u8().await.unwrap(), 5);
        let count = socket.read_u8().await.unwrap();
        let mut methods = vec![0; count as usize];
        socket.read_exact(&mut methods).await.unwrap();
        assert!(methods.contains(&2));
        socket.write_all(&[5, 2]).await.unwrap();
        assert_eq!(socket.read_u8().await.unwrap(), 1);
        let len = socket.read_u8().await.unwrap();
        let mut user = vec![0; len as usize];
        socket.read_exact(&mut user).await.unwrap();
        let len = socket.read_u8().await.unwrap();
        let mut pass = vec![0; len as usize];
        socket.read_exact(&mut pass).await.unwrap();
        assert_eq!(user, b"proxy-user");
        assert_eq!(pass, b"proxy-secret");
        socket.write_all(&[1, 0]).await.unwrap();
        let mut intro = [0; 4];
        socket.read_exact(&mut intro).await.unwrap();
        assert_eq!(intro, [5, 1, 0, 3]);
        let len = socket.read_u8().await.unwrap();
        let mut host = vec![0; len as usize];
        socket.read_exact(&mut host).await.unwrap();
        assert_eq!(host, b"only-proxy-can-resolve.invalid");
        assert_eq!(socket.read_u16().await.unwrap(), 80);
        socket
            .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
            .await
            .unwrap();
        let request = headers(&mut socket).await.to_lowercase();
        assert!(!request.contains("proxy-authorization") && !request.contains("proxy-secret"));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    assert_eq!(
        fixture
            .client()
            .get("http://only-proxy-can-resolve.invalid/file")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn custom_route_cannot_fall_back_to_direct_and_direct_mode_removes_it() {
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unused = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = unused.local_addr().unwrap();
    drop(unused);
    let fixture = Fixture::new(&format!("http://{proxy}"), false);
    let url = format!("http://{}/file", origin.local_addr().unwrap());
    assert!(fixture.client().get(&url).send().await.is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), origin.accept())
            .await
            .is_err()
    );
    let saved = fixture.store.proxy_settings().unwrap();
    fixture
        .store
        .save_proxy_settings(
            &saved,
            Settings {
                mode: Mode::Direct,
                ..Default::default()
            },
            LoginChange::Remove,
        )
        .unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = origin.accept().await.unwrap();
        let request = headers(&mut socket).await;
        assert!(!request.to_lowercase().contains("proxy-authorization"));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    assert_eq!(
        fixture
            .client()
            .get(url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    server.await.unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn proxy_407_stops_the_engine_with_a_settings_hint_and_no_secret_in_history() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture = Fixture::new(&format!("http://{}", listener.local_addr().unwrap()), true);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = headers(&mut socket).await;
        seen.lock().unwrap().push(request);
        socket.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
    });
    let job = fixture
        .store
        .new_download(
            "http://only-proxy.invalid/file",
            None,
            None,
            Uuid::new_v4().to_string(),
        )
        .unwrap();
    fixture.store.enqueue(&job).unwrap();
    crate::engine::run(
        fixture.store.clone(),
        job.clone(),
        tokio_util::sync::CancellationToken::new(),
        crate::bandwidth::Limiter::new(&Default::default()).unwrap(),
    )
    .await
    .unwrap();
    let job = fixture.store.get(&job.id).unwrap().unwrap();
    assert_eq!(job.status, crate::model::Status::Failed);
    assert!(
        job.error
            .as_ref()
            .unwrap()
            .contains("Preferences → Connection → Proxy")
    );
    assert!(job.retry.next_at.is_none());
    assert!(
        !serde_json::to_string(&job)
            .unwrap()
            .contains("proxy-secret")
    );
    server.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
}
