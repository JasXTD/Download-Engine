use super::*;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

static CANCEL_AFTER_CHUNK: OnceLock<Mutex<HashMap<PathBuf, CancellationToken>>> = OnceLock::new();
pub(super) fn after_chunk(path: &Path) {
    if let Some(tokens) = CANCEL_AFTER_CHUNK.get()
        && let Some(token) = tokens.lock().unwrap().remove(path)
    {
        token.cancel();
    }
}

struct Fixture {
    store: Store,
    job: Job,
    sources: Vec<PathBuf>,
    hash: String,
    // Drop the cached SQLite connection before deleting its directory on Windows.
    _temp: tempfile::TempDir,
}

async fn finish(fixture: &mut Fixture, cancel: &CancellationToken) -> Result<Option<FinishedFile>> {
    finalize_sources(
        &fixture.store,
        &mut fixture.job,
        cancel,
        fixture.sources.clone(),
    )
    .await
}

#[tokio::test]
async fn single_part_publishes_the_verified_file_without_copying_and_keeps_collision_safe() {
    let mut fixture = Fixture::new(1, 1024 * 1024);
    let taken = fixture.job.directory.join(&fixture.job.name);
    std::fs::write(&taken, b"keep existing file").unwrap();
    let finished = finish(&mut fixture, &CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(finished.path, taken);
    assert!(same_file::is_same_file(&fixture.sources[0], &finished.path).unwrap());
    assert_eq!(std::fs::read(&taken).unwrap(), b"keep existing file");
    assert_eq!(fixture.job.integrity.actual.as_ref(), Some(&fixture.hash));
    assert!(
        !fixture
            .job
            .directory
            .join(format!(".idm-ex-{}.assembling", fixture.job.id))
            .exists()
    );
    #[cfg(windows)]
    {
        assert!(
            std::fs::OpenOptions::new()
                .write(true)
                .open(&finished.path)
                .is_err()
        );
        let marker =
            std::fs::read_to_string(format!("{}:Zone.Identifier", finished.path.display()))
                .unwrap();
        assert!(marker.contains("ZoneId=3"));
    }
    drop(finished.guard);
}

#[tokio::test]
async fn multipart_assembly_uses_byte_order_and_does_not_mutate_parts() {
    let mut fixture = Fixture::new(4, 1024 * 1024 + 7);
    let originals: Vec<_> = fixture
        .sources
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    fixture.job.plan.as_mut().unwrap().segments.swap(0, 3);
    std::fs::write(&fixture.sources[0], &originals[3]).unwrap();
    std::fs::write(&fixture.sources[3], &originals[0]).unwrap();
    let root = fixture.store.paths.parts(&fixture.job.id).unwrap();
    let result = finalize(
        &fixture.store,
        &mut fixture.job,
        &root,
        &CancellationToken::new(),
        true,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(std::fs::read(&result.path).unwrap(), originals.concat());
    for path in &fixture.sources {
        assert!(!same_file::is_same_file(path, &result.path).unwrap());
    }
    assert_eq!(std::fs::read(&fixture.sources[0]).unwrap(), originals[3]);
    drop(result.guard);
}

#[tokio::test]
async fn cancel_during_hashing_or_copying_joins_io_and_preserves_every_part_for_retry() {
    for parts in [1, 4] {
        let mut fixture = Fixture::new(parts, 2 * 1024 * 1024);
        let original: Vec<_> = fixture
            .sources
            .iter()
            .map(|p| std::fs::read(p).unwrap())
            .collect();
        let token = CancellationToken::new();
        CANCEL_AFTER_CHUNK
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(fixture.sources[0].clone(), token.clone());
        assert!(finish(&mut fixture, &token).await.unwrap().is_none());
        assert!(token.is_cancelled());
        assert!(fixture.job.integrity.actual.is_none());
        assert!(fixture.job.saved_path.is_none());
        assert!(
            !fixture
                .job
                .directory
                .join(format!(".idm-ex-{}.assembling", fixture.job.id))
                .exists()
        );
        for (path, expected) in fixture.sources.iter().zip(&original) {
            assert_eq!(&std::fs::read(path).unwrap(), expected);
            // No blocking-worker or file guard survives the cancelled future.
            std::fs::OpenOptions::new().append(true).open(path).unwrap();
        }
        let result = finish(&mut fixture, &CancellationToken::new())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read(&result.path).unwrap(), original.concat());
        drop(result.guard);
    }
}

#[tokio::test]
async fn checksum_or_size_failure_never_publishes_or_changes_saved_parts() {
    for parts in [1, 4] {
        for wrong_size in [false, true] {
            let mut fixture = Fixture::new(parts, 1024 * 1024);
            let original: Vec<_> = fixture
                .sources
                .iter()
                .map(|p| std::fs::read(p).unwrap())
                .collect();
            if wrong_size {
                fixture.job.downloaded -= 1;
                fixture.job.total = Some(fixture.job.downloaded);
            } else {
                fixture.job.integrity.expected = Some("0".repeat(64));
            }
            let error = finish(&mut fixture, &CancellationToken::new())
                .await
                .err()
                .unwrap();
            assert!(
                error.to_string().contains(if wrong_size {
                    "size mismatch"
                } else {
                    "SHA-256 mismatch"
                }),
                "{error:#}"
            );
            assert!(fixture.job.saved_path.is_none());
            assert!(!fixture.job.directory.join(&fixture.job.name).exists());
            for (path, expected) in fixture.sources.iter().zip(original) {
                assert_eq!(std::fs::read(path).unwrap(), expected);
            }
        }
    }
}

#[tokio::test]
async fn publication_intent_failure_leaves_a_reusable_single_part() {
    let mut fixture = Fixture::new(1, 1024 * 1024);
    let conn = fixture.store.connect().unwrap();
    conn.execute_batch("CREATE TRIGGER reject_intent BEFORE UPDATE ON jobs WHEN json_extract(NEW.data, '$.saved_path') IS NOT NULL BEGIN SELECT RAISE(FAIL, 'test publication intent failure'); END;").unwrap();
    assert!(
        finish(&mut fixture, &CancellationToken::new())
            .await
            .is_err()
    );
    assert!(!fixture.job.directory.join(&fixture.job.name).exists());
    download_file::read_part_sync(&fixture.sources[0]).unwrap();
    conn.execute_batch("DROP TRIGGER reject_intent;").unwrap();
    let result = finish(&mut fixture, &CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    assert!(same_file::is_same_file(&fixture.sources[0], &result.path).unwrap());
    drop(result.guard);
}

#[tokio::test]
async fn interrupted_single_part_publication_recovers_without_network_or_duplicate() {
    for ranged in [false, true] {
        let mut fixture = Fixture::new(1, 1024 * 1024);
        if ranged {
            fixture.job.plan = Some(Plan {
                size: fixture.job.downloaded,
                etag: "\"fixture\"".into(),
                final_url: fixture.job.url.clone(),
                segments: vec![Segment {
                    start: 0,
                    end: fixture.job.downloaded - 1,
                }],
            });
            let root = fixture.store.paths.parts(&fixture.job.id).unwrap();
            let path = part_path(&root, 0);
            std::fs::rename(&fixture.sources[0], &path).unwrap();
            fixture.sources[0] = path;
        }
        let result = finish(&mut fixture, &CancellationToken::new())
            .await
            .unwrap()
            .unwrap();
        drop(result.guard);
        let pending = fixture.store.get(&fixture.job.id).unwrap().unwrap();
        assert_eq!(pending.status, Status::Downloading);
        run(
            fixture.store.clone(),
            pending,
            CancellationToken::new(),
            Limiter::default(),
        )
        .await
        .unwrap();
        let recovered = fixture.store.get(&fixture.job.id).unwrap().unwrap();
        assert_eq!(recovered.status, Status::Complete, "{:?}", recovered.error);
        assert_eq!(recovered.saved_path, Some(result.path));
        assert_eq!(
            std::fs::read_dir(&fixture.job.directory).unwrap().count(),
            1
        );
        assert!(!fixture.sources[0].exists());
    }
}

#[tokio::test]
async fn recovery_rejects_same_size_changes_after_single_part_publication() {
    let mut fixture = Fixture::new(1, 1024);
    let result = finish(&mut fixture, &CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    drop(result.guard);
    std::fs::write(&result.path, vec![0; 1024]).unwrap();
    let pending = fixture.store.get(&fixture.job.id).unwrap().unwrap();
    run(
        fixture.store.clone(),
        pending,
        CancellationToken::new(),
        Limiter::default(),
    )
    .await
    .unwrap();
    let failed = fixture.store.get(&fixture.job.id).unwrap().unwrap();
    assert_eq!(failed.status, Status::Failed);
    assert!(failed.error.unwrap().contains("checksum changed"));
    assert_eq!(
        std::fs::read_dir(&fixture.job.directory).unwrap().count(),
        1
    );
}

#[tokio::test]
async fn recovery_ignores_an_unrelated_name_collision_still_open_for_writing() {
    let mut fixture = Fixture::new(1, 1024);
    let taken = fixture.job.directory.join(&fixture.job.name);
    std::fs::write(&taken, b"unrelated existing file").unwrap();
    let _editor = std::fs::OpenOptions::new()
        .write(true)
        .open(&taken)
        .unwrap();
    fixture.job.saved_path = Some(taken.clone());
    let root = fixture.store.paths.parts(&fixture.job.id).unwrap();
    assert!(
        finalization::recover(
            &fixture.store,
            &mut fixture.job,
            &root,
            &CancellationToken::new()
        )
        .await
        .unwrap()
        .is_none()
    );
    assert_eq!(std::fs::read(taken).unwrap(), b"unrelated existing file");
}

#[cfg(windows)]
#[tokio::test]
async fn finishing_preserves_support_for_a_redirected_download_folder() {
    use std::os::windows::process::CommandExt;
    let mut fixture = Fixture::new(1, 1024);
    let alias = fixture._temp.path().join("selected-folder");
    let output = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&alias)
        .arg(&fixture.job.directory)
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fixture.job.directory = alias.clone();
    let result = finish(&mut fixture, &CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    assert!(same_file::is_same_file(&fixture.sources[0], &result.path).unwrap());
    drop(result.guard);
    let pending = fixture.store.get(&fixture.job.id).unwrap().unwrap();
    run(
        fixture.store.clone(),
        pending,
        CancellationToken::new(),
        Limiter::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        fixture.store.get(&fixture.job.id).unwrap().unwrap().status,
        Status::Complete
    );
    std::fs::remove_dir(alias).unwrap();
}

#[tokio::test]
#[ignore = "requires an explicitly supplied isolated directory on another local volume"]
async fn other_volume_copies_and_recovers_without_duplicate() {
    let other = std::env::var_os("DOWNLOAD_ENGINE_TEST_OTHER_VOLUME")
        .expect("Set an isolated second-volume directory");
    for parts in [1, 4] {
        let mut fixture = Fixture::new(parts, 2 * 1024 * 1024);
        let other_temp = tempfile::tempdir_in(&other).unwrap();
        fixture.job.directory = other_temp.path().to_owned();
        fixture.store.save(&fixture.job).unwrap();
        let source = download_file::read_part_sync(&fixture.sources[0]).unwrap();
        let parents =
            download_file::guard_parents(&fixture.job.directory.join("finished.bin")).unwrap();
        assert!(
            !download_file::same_volume(&source, parents.last().unwrap()).unwrap(),
            "The fixture must use a different volume"
        );
        drop(source);
        drop(parents);
        let result = finish(&mut fixture, &CancellationToken::new())
            .await
            .unwrap()
            .unwrap();
        assert!(!same_file::is_same_file(&fixture.sources[0], &result.path).unwrap());
        let expected = fixture
            .sources
            .iter()
            .flat_map(|p| std::fs::read(p).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(std::fs::read(&result.path).unwrap(), expected);
        drop(result.guard);
        let pending = fixture.store.get(&fixture.job.id).unwrap().unwrap();
        run(
            fixture.store.clone(),
            pending,
            CancellationToken::new(),
            Limiter::default(),
        )
        .await
        .unwrap();
        let recovered = fixture.store.get(&fixture.job.id).unwrap().unwrap();
        assert_eq!(recovered.status, Status::Complete, "{:?}", recovered.error);
        assert_eq!(recovered.saved_path, Some(result.path));
        assert_eq!(
            std::fs::read_dir(&fixture.job.directory).unwrap().count(),
            1
        );
        assert!(!fixture.sources[0].exists());
    }
}
impl Fixture {
    fn new(parts: usize, bytes: usize) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(crate::paths::AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        })
        .unwrap();
        let mut job = Job::new(
            "http://127.0.0.1:1/no-network",
            Some("finished.bin"),
            store.paths.downloads.clone(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        job.downloaded = bytes as u64;
        job.total = Some(bytes as u64);
        job.status = Status::Downloading;
        if parts > 1 {
            job.plan = Some(Plan {
                size: bytes as u64,
                etag: "\"fixture\"".into(),
                final_url: job.url.clone(),
                segments: (0..parts)
                    .map(|i| Segment {
                        start: (i * bytes / parts) as u64,
                        end: ((i + 1) * bytes / parts - 1) as u64,
                    })
                    .collect(),
            });
        }
        let root = store.paths.parts(&job.id).unwrap();
        std::fs::create_dir(&root).unwrap();
        let mut hash = Sha256::new();
        let mut sources = Vec::new();
        let mut buffer: Vec<u8> = (0..256 * 1024)
            .map(|i| ((i ^ (i >> 9)) % 251) as u8)
            .collect();
        for index in 0..parts {
            let path = if parts == 1 {
                root.join("whole.part")
            } else {
                part_path(&root, index)
            };
            let mut file = std::fs::File::create(&path).unwrap();
            let mut remaining = (index + 1) * bytes / parts - index * bytes / parts;
            buffer[0] = index as u8;
            while remaining > 0 {
                let n = remaining.min(buffer.len());
                file.write_all(&buffer[..n]).unwrap();
                hash.update(&buffer[..n]);
                remaining -= n;
            }
            file.sync_all().unwrap();
            sources.push(path);
        }
        let hash = format!("{:x}", hash.finalize());
        job.integrity.expected = Some(hash.clone());
        store.enqueue(&job).unwrap();
        Self {
            store,
            job,
            sources,
            hash,
            _temp: temp,
        }
    }
}

#[cfg(windows)]
fn process_io() -> (u64, u64) {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetProcessIoCounters, IO_COUNTERS,
    };
    let mut counters = IO_COUNTERS::default();
    // The current-process pseudo-handle is valid; counters is writable and correctly sized.
    assert_ne!(
        unsafe { GetProcessIoCounters(GetCurrentProcess(), &mut counters) },
        0
    );
    (counters.ReadTransferCount, counters.WriteTransferCount)
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit release-mode large-file finishing benchmark"]
async fn finalization_throughput() {
    for parts in [1, 4] {
        for sample in 0..3 {
            let mut fixture = Fixture::new(parts, 512 * 1024 * 1024);
            let before = process_io();
            let started = std::time::Instant::now();
            let finished = finalize_sources(
                &fixture.store,
                &mut fixture.job,
                &CancellationToken::new(),
                fixture.sources.clone(),
            )
            .await
            .unwrap()
            .unwrap();
            let elapsed = started.elapsed();
            let after = process_io();
            assert_eq!(fixture.job.integrity.actual.as_ref(), Some(&fixture.hash));
            drop(finished.guard);
            let (actual, guard) =
                crate::integrity::hash_file(&finished.path, &CancellationToken::new()).unwrap();
            assert_eq!(guard.metadata().unwrap().len(), fixture.job.downloaded);
            assert_eq!(actual, fixture.hash);
            drop(guard);
            println!(
                "FINALIZE_BENCH {}",
                serde_json::json!({
                    "parts": parts, "sample": sample, "bytes": fixture.job.downloaded,
                    "milliseconds": elapsed.as_secs_f64() * 1000.0,
                    "process_read_bytes": after.0 - before.0,
                    "process_write_bytes": after.1 - before.1,
                    "sha256_verified": true
                })
            );
        }
    }
}
