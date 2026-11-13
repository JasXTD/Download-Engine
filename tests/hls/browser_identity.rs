use super::*;

const USER_AGENT: &str = "Mozilla/5.0 (Offline test browser) Chrome/130.0.0.0";

#[tokio::test]
async fn browser_identity_survives_redirect_preview_pause_and_database_reload() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    let a = ts(100, 3);
    let b = ts(200, 4);
    for (path, mut reply) in [
        ("/start.m3u8", Reply::redirect("/master.m3u8")),
        (
            "/master.m3u8",
            Reply::bytes("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000\nmedia.m3u8\n"),
        ),
        ("/media.m3u8", Reply::bytes(media(&["a.ts", "b.ts"]))),
        ("/a.ts", Reply::bytes(&a)),
        ("/b.ts", Reply::bytes(&b)),
    ] {
        reply.required_user_agent = Some(USER_AGENT.into());
        reply.required_origin = Some("http://browser.example.test".into());
        server.set(path, reply);
    }
    let url = server.url("/start.m3u8");
    let page = "http://browser.example.test/watch?session=secret";
    let cancel = CancellationToken::new();
    // Reproduce the former handoff behavior against the real local HTTP server.
    let error = hls::inspect_page_via(
        &url,
        Some(page),
        &cancel,
        download_engine::proxy::Route::direct(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        download_engine::diagnostics::Failure::error(&error).http_status,
        Some(403)
    );
    server.requests.lock().unwrap().clear();
    let inspection = hls::inspect_browser_via(
        &url,
        &[],
        Some(page),
        Some(USER_AGENT),
        false,
        &cancel,
        download_engine::proxy::Route::direct(),
    )
    .await
    .unwrap();
    let preview = hls::preview(&inspection, 0, &cancel).await.unwrap();
    let job = store
        .enqueue_inspected_hls(
            &inspection,
            &preview,
            "video.ts",
            None,
            MAIN_QUEUE,
            "browser-identity",
        )
        .unwrap();
    let paused = pause_at_part(&server, &store, job, &preview.parts[1]).await;
    assert_eq!(paused.status, Status::Paused);
    assert_eq!(paused.downloaded, a.len() as u64);
    let reopened = Store::open(store.paths.clone()).unwrap();
    let job = reopened.get(&paused.id).unwrap().unwrap();
    assert_eq!(
        job.hls.as_ref().unwrap().user_agent.as_deref(),
        Some(USER_AGENT)
    );
    let saved = run(&reopened, job).await;
    assert_eq!(saved.status, Status::Complete, "{:?}", saved.error);
    assert_eq!(
        std::fs::read(saved.saved_path.unwrap()).unwrap(),
        [a, b].concat()
    );
    assert_eq!(
        server.count("/a.ts"),
        1,
        "Resume downloaded a verified part again"
    );
    for request in server.requests.lock().unwrap().iter() {
        let request = request.to_ascii_lowercase();
        assert!(request.contains(&format!(
            "\r\nuser-agent: {}\r\n",
            USER_AGENT.to_ascii_lowercase()
        )));
        assert!(request.contains("\r\nreferer: http://browser.example.test/\r\n"));
        if !request.contains("\r\norigin: http://browser.example.test\r\n") {
            assert!(
                request.starts_with("get /start.m3u8 ")
                    || request.starts_with("get /master.m3u8 ")
                    || request.starts_with("get /media.m3u8 "),
                "Once negotiated, segments must reuse the accepted Origin"
            );
        }
        assert!(
            !request.contains("session=secret")
                && !request.contains("cookie:")
                && !request.contains("authorization:")
        );
    }
}

#[tokio::test]
async fn browser_origin_is_not_added_without_context_or_across_a_page_downgrade() {
    let server = Server::new().await;
    server.set("/index.m3u8", Reply::bytes(media(&["a.ts"])));
    let url = server.url("/index.m3u8");
    for (page, agent) in [
        (
            Some("http://browser.example.test/private?secret=value"),
            Some(USER_AGENT),
        ),
        (
            Some("http://browser.example.test/private?secret=value"),
            None,
        ),
        (
            Some("https://browser.example.test/private?secret=value"),
            Some(USER_AGENT),
        ),
        (None, Some(USER_AGENT)),
    ] {
        server.requests.lock().unwrap().clear();
        hls::inspect_browser_via(
            &url,
            &[],
            page,
            agent,
            false,
            &CancellationToken::new(),
            download_engine::proxy::Route::direct(),
        )
        .await
        .unwrap();
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request = requests[0].to_ascii_lowercase();
        assert!(!request.contains("\r\norigin:"));
        assert!(!request.contains("private") && !request.contains("secret=value"));
        if page.is_none_or(|value| value.starts_with("https:")) {
            assert!(!request.contains("\r\nreferer:"));
        }
    }
}

#[tokio::test]
async fn browser_origin_retry_is_bounded_and_requires_safe_page_context() {
    let server = Server::new().await;
    let url = server.url("/index.m3u8");
    for (status, page, agent, attempts) in [
        (
            403,
            Some("http://browser.example.test/private?secret=value"),
            Some(USER_AGENT),
            2,
        ),
        (
            401,
            Some("http://browser.example.test/private?secret=value"),
            Some(USER_AGENT),
            1,
        ),
        (
            500,
            Some("http://browser.example.test/private?secret=value"),
            Some(USER_AGENT),
            1,
        ),
        (
            403,
            Some("https://browser.example.test/private?secret=value"),
            Some(USER_AGENT),
            1,
        ),
        (
            403,
            Some("http://browser.example.test/private?secret=value"),
            None,
            1,
        ),
        (403, None, Some(USER_AGENT), 1),
    ] {
        server.set(
            "/index.m3u8",
            Reply {
                status,
                ..Reply::bytes("denied")
            },
        );
        server.requests.lock().unwrap().clear();
        let error = hls::inspect_browser_via(
            &url,
            &[],
            page,
            agent,
            false,
            &CancellationToken::new(),
            download_engine::proxy::Route::direct(),
        )
        .await
        .unwrap_err();
        if status == 401 || status == 403 {
            assert_eq!(
                download_engine::diagnostics::Failure::error(&error).http_status,
                Some(status)
            );
        }
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), attempts);
        for (index, request) in requests.iter().enumerate() {
            let request = request.to_ascii_lowercase();
            assert_eq!(request.contains("\r\norigin:"), index == 1);
            assert!(!request.contains("private") && !request.contains("secret=value"));
            assert!(!request.contains("cookie:") && !request.contains("authorization:"));
        }
    }
}

#[tokio::test]
async fn browser_identity_cannot_inject_headers_or_relax_network_policy() {
    let server = Server::new().await;
    let other = Server::new().await;
    let cancel = CancellationToken::new();
    let url = server.url("/master.m3u8");
    for invalid in [
        "".to_owned(),
        " ".into(),
        "bad\r\nCookie: secret".into(),
        "x".repeat(513),
        "hidden\u{200b}text".into(),
    ] {
        let error = hls::inspect_browser_via(
            &url,
            &[],
            None,
            Some(&invalid),
            false,
            &cancel,
            download_engine::proxy::Route::direct(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Invalid browser User-Agent"));
    }
    assert!(server.requests.lock().unwrap().is_empty());
    let error = hls::inspect_browser_via(
        &url,
        &[],
        None,
        Some(USER_AGENT),
        true,
        &cancel,
        download_engine::proxy::Route::direct(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("public internet"), "{error:#}");
    assert!(server.requests.lock().unwrap().is_empty());
    server.set("/master.m3u8", Reply::redirect(&other.url("/private.m3u8")));
    let error = hls::inspect_browser_via(
        &url,
        &[],
        None,
        Some(USER_AGENT),
        false,
        &cancel,
        download_engine::proxy::Route::direct(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("different local"), "{error:#}");
    assert!(other.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn browser_identity_defaults_to_none_for_old_jobs_and_rejects_invalid_saved_values() {
    let server = Server::new().await;
    server.set("/index.m3u8", Reply::bytes(media(&["a.ts"])));
    let download = hls::Download::new(playlist(&server, "/index.m3u8").await);
    let legacy = serde_json::to_value(&download).unwrap();
    assert!(legacy.get("user_agent").is_none());
    let mut restored: hls::Download = serde_json::from_value(legacy).unwrap();
    restored.validate().unwrap();
    assert!(restored.user_agent.is_none());
    restored.user_agent = Some("bad\r\nCookie: secret".into());
    assert!(restored.validate().is_err());
}
