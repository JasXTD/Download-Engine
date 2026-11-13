use super::*;
use serde_json::Value;
use std::time::Duration;

// Exercise the real native framing, TCP authentication and desktop body adapter.
// Only the browser's replies are supplied by this fixture; live browser coverage
// is separate so these tests can also run on headless CI machines.
struct Writer {
    bytes: Vec<u8>,
    messages: mpsc::UnboundedSender<Value>,
}
impl io::Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        while self.bytes.len() >= 4 {
            let n = u32::from_le_bytes(self.bytes[..4].try_into().unwrap()) as usize;
            if self.bytes.len() < n + 4 {
                break;
            }
            let message = serde_json::from_slice(&self.bytes[4..4 + n]).unwrap();
            self.messages.send(message).map_err(io::Error::other)?;
            self.bytes.drain(..4 + n);
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Fixture {
    session: Session,
    input: mpsc::Sender<Reply>,
    commands: mpsc::UnboundedReceiver<Value>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let session = Session {
            port: listener.local_addr().unwrap().port(),
            token: Uuid::new_v4(),
            expires: chrono::Utc::now().timestamp() + 3600,
            page: "http://127.0.0.1:34567/watch".into(),
            origins: vec!["http://127.0.0.1:34567".into()],
        };
        let (input, replies) = mpsc::channel(1);
        let (messages, commands) = mpsc::unbounded_channel();
        let serving = session.clone();
        let task = tokio::spawn(async move {
            run(
                listener,
                serving,
                replies,
                &mut Writer {
                    bytes: vec![],
                    messages,
                },
            )
            .await
            .unwrap();
        });
        Self {
            session,
            input,
            commands,
            task,
        }
    }
    async fn command(&mut self, kind: &str) -> Uuid {
        let message = tokio::time::timeout(Duration::from_secs(2), self.commands.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(message["kind"], kind, "{message}");
        message["id"].as_str().unwrap().parse().unwrap()
    }
    fn get(&self, cancel: CancellationToken) -> tokio::task::JoinHandle<Result<Response>> {
        let session = self.session.clone();
        tokio::spawn(async move {
            super::super::get(
                &session,
                &Url::parse("http://127.0.0.1:34567/video.ts").unwrap(),
                None,
                &cancel,
            )
            .await
        })
    }
    async fn head(
        &mut self,
        request: tokio::task::JoinHandle<Result<Response>>,
    ) -> (Uuid, Response) {
        let id = self.command("fetch").await;
        self.input
            .send(Reply::Head {
                id,
                status: 200,
                headers: vec![("content-type".into(), "video/mp2t".into())],
            })
            .await
            .unwrap();
        (id, request.await.unwrap().unwrap())
    }
}

#[tokio::test]
async fn authenticated_transfer_preserves_bytes_and_clean_end() {
    use sha2::{Digest, Sha256};
    let mut f = Fixture::new().await;
    let request = f.get(CancellationToken::new());
    let (id, response) = f.head(request).await;
    let result = tokio::spawn(async move { response.bytes().await.unwrap() });
    let data: Vec<u8> = (0..CHUNK_BYTES * 3 + 17).map(|n| (n % 251) as u8).collect();
    for chunk in data.chunks(CHUNK_BYTES) {
        assert_eq!(f.command("read").await, id);
        f.input
            .send(Reply::Chunk {
                id,
                data: STANDARD.encode(chunk),
            })
            .await
            .unwrap();
    }
    assert_eq!(f.command("read").await, id);
    f.input.send(Reply::End { id }).await.unwrap();
    assert_eq!(Sha256::digest(result.await.unwrap()), Sha256::digest(data));
}

#[tokio::test]
async fn cancelling_headers_and_dropping_body_abort_browser_requests() {
    let mut f = Fixture::new().await;
    let stop = CancellationToken::new();
    let request = f.get(stop.clone());
    let id = f.command("fetch").await;
    stop.cancel();
    assert!(request.await.unwrap().is_err());
    assert_eq!(f.command("cancel").await, id);
    // The connection remains reusable after a paused request.
    let request = f.get(CancellationToken::new());
    let (id, response) = f.head(request).await;
    assert_eq!(f.command("read").await, id);
    drop(response);
    assert_eq!(f.command("cancel").await, id);
}

#[tokio::test]
async fn browser_failures_preserve_safe_reasons_before_headers_and_during_body() {
    for body in [false, true] {
        for reason in [
            Failure::Network,
            Failure::Timeout,
            Failure::TooLarge,
            Failure::Unsupported,
            Failure::Cancelled,
            Failure::Protocol,
            Failure::Http,
        ] {
            let mut f = Fixture::new().await;
            let request = f.get(CancellationToken::new());
            if body {
                let (id, response) = f.head(request).await;
                f.command("read").await;
                f.input.send(Reply::Failed { id, reason }).await.unwrap();
                let error = response.bytes().await.unwrap_err();
                assert!(format!("{:#}", anyhow::Error::from(error)).contains(reason.message()));
                assert_eq!(f.command("cancel").await, id);
            } else {
                let id = f.command("fetch").await;
                f.input.send(Reply::Failed { id, reason }).await.unwrap();
                assert!(
                    request
                        .await
                        .unwrap()
                        .unwrap_err()
                        .to_string()
                        .contains(reason.message())
                );
                assert_eq!(f.command("cancel").await, id);
            }
            // Failure closes only this request; the reviewed connection survives.
            let request = f.get(CancellationToken::new());
            let (id, response) = f.head(request).await;
            f.command("read").await;
            f.input.send(Reply::End { id }).await.unwrap();
            assert!(response.bytes().await.unwrap().is_empty());
        }
    }
}

#[test]
fn browser_failure_codes_reject_unknown_or_free_form_data() {
    let id = Uuid::new_v4();
    let old: Reply = serde_json::from_value(serde_json::json!({"kind":"failed","id":id})).unwrap();
    assert!(matches!(
        old,
        Reply::Failed {
            reason: Failure::Network,
            ..
        }
    ));
    assert!(
        serde_json::from_value::<Reply>(
            serde_json::json!({"kind":"failed","id":id,"reason":"secret url"})
        )
        .is_err()
    );
    assert!(Failure::from_code(0).is_err());
    assert!(Failure::from_code(255).is_err());
}

#[tokio::test]
async fn malformed_or_missing_body_end_never_reports_completion() {
    for failure in ["oversized", "invalid_base64", "disconnect"] {
        let mut f = Fixture::new().await;
        let request = f.get(CancellationToken::new());
        let (id, response) = f.head(request).await;
        let result = tokio::spawn(async move { response.bytes().await });
        f.command("read").await;
        if failure == "disconnect" {
            f.task.abort();
        } else {
            f.input
                .send(Reply::Chunk {
                    id,
                    data: if failure == "oversized" {
                        STANDARD.encode(vec![1; CHUNK_BYTES + 1])
                    } else {
                        "!bad!".into()
                    },
                })
                .await
                .unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_secs(2), result)
                .await
                .unwrap()
                .unwrap()
                .is_err(),
            "{failure}"
        );
    }
}

#[tokio::test]
async fn wrong_capabilities_and_unreviewed_servers_never_reach_browser() {
    let mut f = Fixture::new().await;
    for (token, url, range) in [
        (Uuid::new_v4(), "http://127.0.0.1:34567/video.ts", None),
        (f.session.token, "http://127.0.0.1:34568/private", None),
        (
            f.session.token,
            "http://127.0.0.1:34567/video.ts",
            Some(crate::hls::Range {
                start: u64::MAX,
                length: 2,
            }),
        ),
    ] {
        let mut socket = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, f.session.port))
            .await
            .unwrap();
        let bytes = serde_json::to_vec(&Request {
            token,
            url: url.into(),
            range,
        })
        .unwrap();
        socket.write_u32_le(bytes.len() as u32).await.unwrap();
        socket.write_all(&bytes).await.unwrap();
        let mut one = [0];
        let result = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut one))
            .await
            .unwrap();
        assert!(result.is_err() || result.unwrap() == 0);
        assert!(f.commands.try_recv().is_err());
    }
    f.session.expires = chrono::Utc::now().timestamp() - 1;
    f.session.validate().unwrap(); // Old saved jobs still load and display.
    assert!(f.get(CancellationToken::new()).await.unwrap().is_err());
}

#[test]
fn only_https_public_servers_or_same_loopback_origin_are_eligible() {
    for (page, origin) in [
        ("https://example.com/watch", "https://127.0.0.1"),
        ("https://example.com/watch", "http://example.com"),
        ("http://127.0.0.1:81/watch", "http://127.0.0.1:82"),
        ("http://example.com/watch", "http://example.com"),
        ("https://example.com/watch", "https://192.168.1.1"),
    ] {
        assert!(
            validate_scope(page, &[origin.into()]).is_err(),
            "{page} {origin}"
        );
    }
    validate_scope(
        "https://example.com/watch",
        &["https://cdn.example.com".into()],
    )
    .unwrap();
    validate_scope("http://127.0.0.1:81/watch", &["http://127.0.0.1:81".into()]).unwrap();
    assert!(
        validate_scope(
            "https://example.com",
            &vec!["https://example.com".into(); 17]
        )
        .is_err()
    );
}
