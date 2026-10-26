use super::*;
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn player() -> Value {
    json!({"playabilityStatus":{"status":"OK"},"videoDetails":{"videoId":"YE7VzlLtp-4","title":"Big Buck Bunny","lengthSeconds":"597"},
        "streamingData":{"hlsManifestUrl":"https://manifest.googlevideo.com/api/manifest/hls_variant/id/123/file/index.m3u8"}})
}

#[test]
fn player_validation_rejects_wrong_video_live_login_drm_and_missing_formats() {
    assert_eq!(
        parse_player(&player(), "YE7VzlLtp-4").unwrap().title,
        "Big Buck Bunny"
    );
    for status in [
        "LOGIN_REQUIRED",
        "AGE_CHECK_REQUIRED",
        "AGE_VERIFICATION_REQUIRED",
        "LIVE_STREAM_OFFLINE",
        "UNPLAYABLE",
        "ERROR",
        "NEW_STATUS",
    ] {
        let mut value = player();
        value["playabilityStatus"]["status"] = json!(status);
        assert!(parse_player(&value, "YE7VzlLtp-4").is_err());
    }
    for case in 0..6 {
        let mut value = player();
        match case {
            0 => value["videoDetails"]["videoId"] = json!("abcdefghijk"),
            1 => value["videoDetails"]["isLive"] = json!(true),
            2 => value["videoDetails"]["isUpcoming"] = json!(true),
            3 => value["videoDetails"]["lengthSeconds"] = json!("0"),
            4 => value["streamingData"]["licenseInfos"] = json!([{}]),
            _ => value["streamingData"]
                .as_object_mut()
                .unwrap()
                .remove("hlsManifestUrl")
                .map(|_| ())
                .unwrap(),
        }
        assert!(parse_player(&value, "YE7VzlLtp-4").is_err(), "case {case}");
    }
}

#[test]
fn embedded_json_handles_strings_braces_and_trailing_script_without_execution() {
    let html = "ytcfg.set(function(){bad()}); ytcfg.set({\"VISITOR_DATA\":\"a}b\\\"c\",\"nested\":{\"x\":1}});throw new Error('must not run')";
    let cfg = embedded(html, "ytcfg.set(").unwrap().unwrap();
    assert_eq!(cfg["VISITOR_DATA"], "a}b\"c");
    assert!(embedded("ytcfg.set({\"bad\":", "ytcfg.set(").is_err());
    assert!(embedded(&"x".repeat(MAX_BODY + 1), "ytcfg.set(").is_err());
    assert!(embedded("no data", "ytcfg.set(").unwrap().is_none());
}

#[tokio::test]
async fn watch_visitor_is_sent_in_both_player_header_and_body_without_cookies() {
    for via_proxy in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let captured = requests.clone();
        let server = tokio::spawn(async move {
            for body in [
                "<script>ytcfg.set({\"VISITOR_DATA\":\"visitor-fixture\"});</script>".into(),
                player().to_string(),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:").map(str::trim))
                            .unwrap_or("0")
                            .parse::<usize>()
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(request).unwrap());
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let route = if via_proxy {
            crate::proxy::Route {
                settings: crate::proxy::Settings {
                    mode: crate::proxy::Mode::Custom,
                    endpoint: base.clone(),
                    ..Default::default()
                },
                login: Some(Arc::new(crate::proxy::Login::new(
                    "fixture-proxy".into(),
                    "fixture-secret".into(),
                ))),
            }
        } else {
            crate::proxy::Route::direct()
        };
        let client = route
            .configure(
                Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none()),
            )
            .unwrap()
            .build()
            .unwrap();
        let result = extract_with(
            &client,
            &format!("{base}/watch"),
            &format!("{base}/player"),
            "YE7VzlLtp-4",
            &CancellationToken::new(),
            &route,
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(result.title, "Big Buck Bunny");
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let request = &requests[1];
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        let headers = headers.to_ascii_lowercase();
        assert!(headers.contains("x-goog-visitor-id: visitor-fixture"));
        assert!(headers.contains("x-youtube-client-name: 101"));
        assert!(headers.contains("x-youtube-client-version: 1.02"));
        assert!(
            !headers.contains("cookie:")
                && !headers
                    .lines()
                    .any(|line| line.starts_with("authorization:"))
        );
        assert_eq!(headers.contains("proxy-authorization: basic "), via_proxy);
        assert_eq!(headers.starts_with("post http://"), via_proxy);
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["videoId"], "YE7VzlLtp-4");
        assert_eq!(body["context"]["client"]["visitorData"], "visitor-fixture");
        assert_eq!(body["context"]["client"]["clientName"], "VISIONOS");
        assert_eq!(
            body["playbackContext"]["contentPlaybackContext"]["html5Preference"],
            "HTML5_PREF_WANTS"
        );
    }
}

#[tokio::test]
async fn requests_reject_redirects_oversized_bodies_and_cancel_during_a_body() {
    for (headers, expected) in [
        (
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\n\r\n",
            "redirected",
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 8388609\r\n\r\n",
            "exceeds",
        ),
        (
            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n",
            "refused",
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 4096];
            assert!(socket.read(&mut buf).await.unwrap() > 0);
            socket.write_all(headers.as_bytes()).await.unwrap();
        });
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        assert!(
            text(
                client.get(url),
                &CancellationToken::new(),
                &crate::proxy::Route::direct()
            )
            .await
            .unwrap_err()
            .to_string()
            .contains(expected)
        );
        server.await.unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0; 4096];
        assert!(socket.read(&mut buf).await.unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{")
            .await
            .unwrap();
        token.cancel();
        let _ = socket.read(&mut buf).await;
    });
    let client = Client::builder().no_proxy().build().unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        text(client.get(url), &cancel, &crate::proxy::Route::direct()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    server.await.unwrap();
}
