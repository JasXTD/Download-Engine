use super::*;

// Each accepted request receives the next response. Real HTTP responses make
// these checks cover the engine's probe, fallback and range dispatch paths.
async fn download_responses(responses: Vec<String>) -> (TempDir, Store, Job) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for response in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
                assert!(request.len() < 32768);
            }
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let (temp, store) = setup();
    let job = enqueue(&store, &url);
    let outcome = until_stalled(&store, run(&store, job)).await;
    if outcome.is_err() {
        server.abort();
    }
    let job = outcome.expect("HTTP response fixture must finish");
    tokio::time::timeout(PROGRESS_LIMIT, server)
        .await
        .expect("Engine must request every fixture response")
        .unwrap();
    (temp, store, job)
}

fn response(status: &str, headers: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    )
}

fn assert_preview_required(store: &Store, job: &Job) {
    assert_eq!(job.status, Status::Failed);
    assert!(job.error.as_deref().unwrap().contains("Preview video"));
    assert!(job.saved_path.is_none());
    assert_eq!(job.downloaded, 0);
    assert!(
        std::fs::read_dir(&store.paths.downloads)
            .unwrap()
            .next()
            .is_none()
    );
}

#[tokio::test]
async fn unknown_length_probe_falls_back_to_a_complete_download_without_resuming() {
    for validator in ["", "ETag: \"v1\"\r\n"] {
        let (_temp, _store, job) = download_responses(vec![
            response(
                "206 Partial Content",
                &format!("Content-Range: bytes 0-0/*\r\n{validator}"),
                "h",
            ),
            response(
                "200 OK",
                "Content-Type: application/octet-stream\r\n",
                "hello world",
            ),
        ])
        .await;
        assert_eq!(job.status, Status::Complete, "{:?}", job.error);
        assert!(
            job.plan.is_none(),
            "An unknown size cannot create a resumable plan"
        );
        assert_eq!(
            std::fs::read(job.saved_path.unwrap()).unwrap(),
            b"hello world"
        );
        assert_eq!(job.downloaded, 11);
    }
}

#[tokio::test]
async fn unknown_length_probe_still_rejects_wrong_range_and_unsafe_fallback_content() {
    let (_temp, _store, job) = download_responses(vec![response(
        "206 Partial Content",
        "Content-Range: bytes 1-1/*\r\n",
        "x",
    )])
    .await;
    assert_eq!(job.status, Status::Failed);
    assert!(job.saved_path.is_none());
    let (_temp, store, job) = download_responses(vec![
        response("206 Partial Content", "Content-Range: bytes 0-0/*\r\n", "#"),
        response(
            "200 OK",
            "Content-Type: application/vnd.apple.mpegurl\r\n",
            "#EXTM3U\n",
        ),
    ])
    .await;
    assert_preview_required(&store, &job);
}

#[tokio::test]
async fn fallback_get_rejects_video_manifests_before_writing() {
    for mime in ["application/vnd.apple.mpegurl", "application/dash+xml"] {
        for probe in [
            response("206 Partial Content", "Content-Range: bytes 0-0/8\r\n", "#"),
            response(
                "416 Range Not Satisfiable",
                "Content-Range: bytes */8\r\n",
                "",
            ),
        ] {
            let (_temp, store, job) = download_responses(vec![
                probe,
                response("200 OK", &format!("Content-Type: {mime}\r\n"), "#EXTM3U\n"),
            ])
            .await;
            assert_preview_required(&store, &job);
        }
    }
}

#[tokio::test]
async fn range_get_rejects_video_manifests_before_appending() {
    for mime in ["application/vnd.apple.mpegurl", "application/dash+xml"] {
        let (_temp, store, job) = download_responses(vec![
            response(
                "206 Partial Content",
                "Content-Range: bytes 0-0/8\r\nETag: \"v1\"\r\n",
                "#",
            ),
            response(
                "206 Partial Content",
                &format!("Content-Range: bytes 0-7/8\r\nETag: \"v1\"\r\nContent-Type: {mime}\r\n"),
                "#EXTM3U\n",
            ),
        ])
        .await;
        assert_preview_required(&store, &job);
    }
}

#[tokio::test]
async fn manifest_error_response_preserves_http_retry_policy() {
    let (_temp, _store, job) = download_responses(vec![response(
        "503 Service Unavailable",
        "Content-Type: application/dash+xml\r\nRetry-After: 5\r\n",
        "",
    )])
    .await;
    assert_eq!(job.status, Status::Queued);
    assert_eq!(job.retry.attempts, 1);
    assert!(job.retry.next_at.is_some());
    assert!(job.saved_path.is_none());
}
