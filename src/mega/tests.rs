use super::*;

const KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
fn link() -> String {
    format!("https://mega.nz/file/abcdefgh#{KEY}")
}
fn job() -> Job {
    Job::new(
        &link(),
        None,
        std::env::temp_dir(),
        uuid::Uuid::new_v4().to_string(),
    )
    .unwrap()
}

#[test]
fn links_preserve_keys_and_canonicalize_legacy_public_files() {
    for host in ["mega.nz", "www.mega.nz", "mega.co.nz", "mega.io"] {
        for path in [
            format!("/file/abcdefgh#{KEY}"),
            format!("/#!abcdefgh!{KEY}"),
        ] {
            assert_eq!(
                crate::model::validate_download_url(&format!("https://{host}{path}"))
                    .unwrap()
                    .as_str(),
                link()
            );
        }
    }
    assert_eq!(job().url, link());
    assert_eq!(
        crate::model::validate_url("https://example.com/file#secret")
            .unwrap()
            .as_str(),
        "https://example.com/file"
    );
    for bad in [
        "https://mega.nz/file/abcdefgh",
        "https://mega.nz/folder/abcdefgh#key",
        "https://mega.nz/#F!abcdefgh!key",
        "https://mega.nz/#P!locked",
        "https://mega.nz/file/a#key",
        "https://mega.nz/file/abcdefgh#short",
        "https://mega.nz/file/abcdefgh#key/file/abc",
        "https://user:password@mega.nz/file/abcdefgh#key",
        "http://mega.nz/file/abcdefgh#key",
        "https://mega.nz:444/file/abcdefgh#key",
    ] {
        assert!(canonical_url(bad).is_err(), "{bad}");
    }
    assert!(!is_url(
        "https://mega.nz.attacker.example/file/abcdefgh#key"
    ));
    assert!(!is_url(
        "https://mega.nz@attacker.example/file/abcdefgh#key"
    ));
    assert!(!is_url("https://mega.nz/MEGAsyncSetup64.exe"));
    assert_eq!(
        crate::model::validate_download_url("https://mega.nz/MEGAsyncSetup64.exe#anchor")
            .unwrap()
            .as_str(),
        "https://mega.nz/MEGAsyncSetup64.exe"
    );
}

#[test]
fn persisted_links_cannot_be_reinterpreted_as_direct_file_transfers() {
    let mut job = job();
    job.validate_persisted().unwrap();
    job.plan = Some(Plan {
        size: 10,
        etag: "not-mega".into(),
        final_url: job.url.clone(),
        segments: vec![Segment { start: 0, end: 9 }],
    });
    job.total = Some(10);
    assert!(job.validate_persisted().is_err());
    job.plan.as_mut().unwrap().etag = Link::parse(&job.url).unwrap().identity();
    job.validate_persisted().unwrap();
    job.plan.as_mut().unwrap().final_url = "https://other.example/file".into();
    assert!(job.validate_persisted().is_err());
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[derive(Clone, serde::Deserialize)]
    struct Vector {
        size: usize,
        key: String,
        attributes: String,
        sha256: String,
        encrypted_sha256: String,
    }
    fn vectors() -> Vec<Vector> {
        serde_json::from_str(include_str!("../../tests/fixtures/mega/vectors.json")).unwrap()
    }
    fn plaintext(size: usize) -> Vec<u8> {
        (0..size).map(|i| ((i * 13 + 7) % 251) as u8).collect()
    }
    fn vector_link(v: &Vector) -> String {
        format!("https://mega.nz/file/abcdefgh#{}", v.key)
    }
    fn encrypted(v: &Vector) -> Vec<u8> {
        let link = Link::parse(&vector_link(v)).unwrap();
        let cipher = crypto::Cipher::new(&link.aes_key(), link.nonce()).unwrap();
        let mut data = plaintext(v.size);
        cipher.ctr(&mut data, 0).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&data)), v.encrypted_sha256);
        data
    }
    fn store(temp: &tempfile::TempDir) -> Store {
        Store::open(crate::paths::AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        })
        .unwrap()
    }

    #[test]
    fn native_and_batch_handoffs_keep_the_key_in_the_saved_job() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let reply = crate::native::handle_request(
            &store,
            crate::native::Request::Add {
                request_id: "mega-native".into(),
                url: link(),
                filename: None,
                source_page: None,
            },
        );
        assert!(reply.ok, "{}", reply.message);
        let saved = store.get(reply.job_id.as_ref().unwrap()).unwrap().unwrap();
        assert_eq!(saved.url, link());
        saved.validate_persisted().unwrap();
        let ids = store
            .enqueue_browser_batch(
                &format!("batch:{}", uuid::Uuid::new_v4()),
                &[crate::browser_batch::Link {
                    url: link(),
                    filename: None,
                }],
                crate::queue::MAIN_QUEUE,
                crate::browser_batch::Destination::Downloads,
            )
            .unwrap();
        assert_eq!(store.get(&ids[0]).unwrap().unwrap().url, link());
    }

    #[test]
    fn independent_vectors_cover_padding_mac_chunks_and_unaligned_ctr_resume() {
        let temp = tempfile::tempdir().unwrap();
        for v in vectors() {
            let link = Link::parse(&vector_link(&v)).unwrap();
            let cipher = crypto::Cipher::new(&link.aes_key(), link.nonce()).unwrap();
            let data = encrypted(&v);
            for start in [0, 1, 15, 16, 17, 131071, 131073]
                .into_iter()
                .filter(|&n| n < data.len())
            {
                let end = (start + 53).min(data.len());
                let mut part = data[start..end].to_vec();
                cipher.ctr(&mut part, start as u64).unwrap();
                assert_eq!(part, plaintext(v.size)[start..end]);
            }
            let source = temp.path().join("cipher.part");
            std::fs::write(&source, &data).unwrap();
            let output_path = temp.path().join("plain.part");
            let output = std::fs::File::create(&output_path).unwrap();
            let hash = decrypt_file(
                link,
                vec![(source, v.size as u64)],
                output,
                v.size as u64,
                &CancellationToken::new(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(hash, v.sha256);
            assert_eq!(std::fs::read(output_path).unwrap(), plaintext(v.size));
        }
    }

    #[tokio::test]
    async fn cancelled_decryption_removes_plaintext_and_keeps_ciphertext_for_resume() {
        let vector = vectors().pop().unwrap();
        let ciphertext = encrypted(&vector);
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("cipher.part");
        let output = temp.path().join("mega-plaintext.part");
        std::fs::write(&source, &ciphertext).unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = decrypt_to_staging(
            Link::parse(&vector_link(&vector)).unwrap(),
            vec![(source.clone(), vector.size as u64)],
            &output,
            vector.size as u64,
            &cancel,
        )
        .await
        .unwrap();
        assert!(result.is_none());
        assert!(!output.exists());
        assert_eq!(std::fs::read(source).unwrap(), ciphertext);
    }

    struct Fixture {
        network: network::Network,
        requests: Arc<Mutex<Vec<String>>>,
        mode: Arc<AtomicU8>,
        server: tokio::task::JoinHandle<()>,
        gate: Arc<tokio::sync::Semaphore>,
        vector: Vector,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.server.abort();
        }
    }
    impl Fixture {
        async fn new(v: Vector) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}/", listener.local_addr().unwrap());
            let data = Arc::new(encrypted(&v));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let mode = Arc::new(AtomicU8::new(0));
            let gate = Arc::new(tokio::sync::Semaphore::new(0));
            let (r, m, g, b, vector) = (
                requests.clone(),
                mode.clone(),
                gate.clone(),
                base.clone(),
                v.clone(),
            );
            let server = tokio::spawn(async move {
                let mut tasks = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        stream = listener.accept() => {
                            let Ok((mut stream, _)) = stream else { break };
                            let (r, m, g, b, v, data) = (r.clone(), m.clone(), g.clone(), b.clone(), vector.clone(), data.clone());
                            tasks.spawn(async move {
                                let mut raw = Vec::new(); let mut byte = [0];
                                while !raw.ends_with(b"\r\n\r\n") && raw.len() < 16384 {
                                    if stream.read_exact(&mut byte).await.is_err() { return; }
                                    raw.push(byte[0]);
                                }
                                let header = String::from_utf8(raw).unwrap();
                                let length: usize = header.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length: ").map(str::to_owned)).unwrap_or("0".into()).parse().unwrap();
                                let mut body = vec![0; length];
                                if stream.read_exact(&mut body).await.is_err() { return; }
                                r.lock().unwrap().push(format!("{header}{}", String::from_utf8_lossy(&body)));
                                let path = header.split_whitespace().nth(1).unwrap();
                                let mode = m.load(Ordering::SeqCst);
                                let mut status = 200;
                                let mut extra = String::new();
                                let payload = if path.starts_with("/cs?") {
                                    match mode {
                                        4 => b"[-17]".to_vec(),
                                        5 => b"[-9]".to_vec(),
                                        6 => serde_json::json!([{"g":"http://127.0.0.1/private", "s":v.size, "at":v.attributes}]).to_string().into_bytes(),
                                        7 => { status = 302; extra = "Location: http://127.0.0.1/private\r\n".into(); Vec::new() },
                                        8 => vec![b' '; 128*1024+1],
                                        _ => serde_json::json!([{"g":format!("{b}data"), "s":v.size, "at":v.attributes}]).to_string().into_bytes(),
                                    }
                                } else {
                                    if mode == 2 { status = 403; Vec::new() }
                                    else {
                                        let (start, end) = path.strip_prefix("/data/").unwrap().split_once('-').unwrap();
                                        let (start, end): (usize, usize) = (start.parse().unwrap(), end.parse().unwrap());
                                        let mut payload = data[start..=end].to_vec();
                                        if mode == 3 && !payload.is_empty() { payload[0] ^= 1; }
                                        payload
                                    }
                                };
                                let header = format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n", payload.len());
                                if stream.write_all(header.as_bytes()).await.is_err() { return; }
                                if mode == 1 && path.starts_with("/data/") {
                                    let prefix = payload.len().min(32771);
                                    if stream.write_all(&payload[..prefix]).await.is_err() { return; }
                                    let Ok(permit) = g.acquire().await else { return }; permit.forget();
                                    let _ = stream.write_all(&payload[prefix..]).await;
                                } else { let _ = stream.write_all(&payload).await; }
                            });
                        },
                        _ = tasks.join_next(), if !tasks.is_empty() => {},
                    }
                }
            });
            let mut network = network::Network::via(crate::proxy::Route::direct(), false).unwrap();
            network.fixture = Some(Url::parse(&base).unwrap());
            Self {
                network,
                requests,
                mode,
                server,
                gate,
                vector: v,
            }
        }
        fn job(&self, store: &Store) -> Job {
            let job = store
                .new_download(
                    &vector_link(&self.vector),
                    None,
                    None,
                    uuid::Uuid::new_v4().to_string(),
                )
                .unwrap();
            store.enqueue(&job).unwrap()
        }
        async fn run(
            &self,
            store: &Store,
            job: &mut Job,
            token: &CancellationToken,
        ) -> Result<Option<FinishedFile>> {
            let root = store.paths.parts(&job.id)?;
            let _guards =
                download_file::prepare_parts(&store.paths.data.join("parts"), &root).await?;
            tokio::fs::create_dir_all(&job.directory).await?;
            let limiter = Limiter::new(&Default::default())?.with_store(store)?;
            transfer_with_network(store, job, &root, token, &limiter, self.network.clone()).await
        }
    }

    #[tokio::test]
    async fn public_file_parallel_transfer_publishes_only_verified_plaintext() {
        let fixture = Fixture::new(vectors().pop().unwrap()).await;
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let mut job = fixture.job(&store);
        let file = fixture
            .run(&store, &mut job, &CancellationToken::new())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.name, "fixture.bin");
        assert_eq!(
            job.integrity.actual.as_deref(),
            Some(fixture.vector.sha256.as_str())
        );
        assert_eq!(
            std::fs::read(&file.path).unwrap(),
            plaintext(fixture.vector.size)
        );
        assert_eq!(job.plan.as_ref().unwrap().segments.len(), 4);
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.starts_with("POST /cs?"))
                .count(),
            1
        );
        assert!(
            requests
                .iter()
                .filter(|r| r.starts_with("GET /data/"))
                .count()
                >= 4
        );
        assert!(requests.iter().all(|r| !r.contains(&fixture.vector.key)
            && !r.to_ascii_lowercase().contains("cookie:")
            && !r.to_ascii_lowercase().contains("authorization:")));
    }

    #[tokio::test]
    async fn zero_byte_and_small_files_verify_without_invalid_range_plans() {
        for v in vectors().into_iter().take(5) {
            let fixture = Fixture::new(v).await;
            let temp = tempfile::tempdir().unwrap();
            let store = store(&temp);
            let mut job = fixture.job(&store);
            let file = fixture
                .run(&store, &mut job, &CancellationToken::new())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                std::fs::read(file.path).unwrap(),
                plaintext(fixture.vector.size)
            );
            job.validate_persisted().unwrap();
        }
    }

    #[tokio::test]
    async fn pause_drains_workers_and_resume_reuses_ciphertext_at_unaligned_offsets() {
        let fixture = Fixture::new(vectors().pop().unwrap()).await;
        fixture.mode.store(1, Ordering::SeqCst);
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let mut job = fixture.job(&store);
        let _activity = store.activity.begin(&job.id);
        let job_id = job.id.clone();
        let token = CancellationToken::new();
        let paused = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let run = fixture.run(&store, &mut job, &token);
            tokio::pin!(run);
            tokio::select! {
                result = &mut run => panic!("Transfer unexpectedly ended: {:?}", result.err()),
                _ = async {
                    loop {
                        let bytes = store.get(&job_id).unwrap().unwrap().downloaded;
                        if bytes > 0 { break; }
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                } => { token.cancel(); run.await }
            }
        })
        .await
        .unwrap()
        .unwrap();
        assert!(paused.is_none());
        assert!(job.downloaded > 0 && job.downloaded < fixture.vector.size as u64);
        assert!(job.saved_path.is_none());
        let before = job.downloaded;
        let root = store.paths.parts(&job.id).unwrap();
        let lengths: Vec<_> = job
            .plan
            .as_ref()
            .unwrap()
            .segments
            .iter()
            .enumerate()
            .map(|(i, _)| {
                std::fs::metadata(engine::part_path(&root, i))
                    .unwrap()
                    .len()
            })
            .collect();
        assert_eq!(lengths.iter().sum::<u64>(), before);
        fixture.mode.store(0, Ordering::SeqCst);
        fixture.gate.add_permits(32);
        fixture.requests.lock().unwrap().clear();
        let file = fixture
            .run(&store, &mut job, &CancellationToken::new())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            std::fs::read(file.path).unwrap(),
            plaintext(fixture.vector.size)
        );
        let requests = fixture.requests.lock().unwrap();
        for (segment, done) in job.plan.as_ref().unwrap().segments.iter().zip(lengths) {
            if done > 0 {
                assert!(
                    requests
                        .iter()
                        .any(|r| r.starts_with(&format!("GET /data/{}-", segment.start + done)))
                );
            }
        }
    }

    #[tokio::test]
    async fn corruption_quota_deleted_unsafe_redirect_and_oversize_never_publish() {
        for mode in [3, 4, 5, 6, 7, 8] {
            let fixture = Fixture::new(vectors()[4].clone()).await;
            fixture.mode.store(mode, Ordering::SeqCst);
            let temp = tempfile::tempdir().unwrap();
            let store = store(&temp);
            let mut job = fixture.job(&store);
            let plaintext = store
                .paths
                .parts(&job.id)
                .unwrap()
                .join("mega-plaintext.part");
            let error = fixture
                .run(&store, &mut job, &CancellationToken::new())
                .await
                .err()
                .expect("Must fail");
            assert!(job.saved_path.is_none(), "mode {mode}: {error:#}");
            assert!(std::fs::read_dir(&job.directory).unwrap().next().is_none());
            assert!(!error.to_string().contains(&fixture.vector.key));
            if mode == 3 {
                assert!(error.to_string().contains("integrity check failed"));
            }
            if mode >= 4 {
                assert_eq!(fixture.requests.lock().unwrap().len(), 1);
            }
            assert!(
                !plaintext.exists(),
                "unverified plaintext remained in mode {mode}: {error:#}"
            );
        }
    }

    #[tokio::test]
    async fn expired_addresses_retry_with_fresh_metadata_and_keep_user_names() {
        let fixture = Fixture::new(vectors()[4].clone()).await;
        fixture.mode.store(2, Ordering::SeqCst);
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let mut job = fixture.job(&store);
        job.name = "chosen.bin".into();
        job.server_name_pending = false;
        let error = fixture
            .run(&store, &mut job, &CancellationToken::new())
            .await
            .err()
            .unwrap();
        crate::retry::failed(&mut job, &error, 0);
        assert!(job.retry.next_at.is_some());
        fixture.mode.store(0, Ordering::SeqCst);
        let file = fixture
            .run(&store, &mut job, &CancellationToken::new())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(file.path.file_name().unwrap(), "chosen.bin");
        assert_eq!(
            fixture
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.starts_with("POST /cs?"))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn wrong_link_key_and_user_checksum_never_publish() {
        for wrong_key in [true, false] {
            let fixture = Fixture::new(vectors()[4].clone()).await;
            let temp = tempfile::tempdir().unwrap();
            let store = store(&temp);
            let mut job = fixture.job(&store);
            let plaintext = store
                .paths
                .parts(&job.id)
                .unwrap()
                .join("mega-plaintext.part");
            if wrong_key {
                job.url = link();
            } else {
                job.integrity.expected = Some("0".repeat(64));
            }
            let error = fixture
                .run(&store, &mut job, &CancellationToken::new())
                .await
                .err()
                .unwrap();
            assert!(job.saved_path.is_none(), "{error:#}");
            assert!(std::fs::read_dir(&job.directory).unwrap().next().is_none());
            assert!(
                !plaintext.exists(),
                "unverified plaintext remained after {error:#}"
            );
        }
    }
}
