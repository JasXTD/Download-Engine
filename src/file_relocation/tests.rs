use super::*;
use std::sync::{
    Arc, Condvar, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

struct Hook {
    force_copy: bool,
    crash: u8,
    reached: AtomicBool,
    arrived: mpsc::Sender<()>,
    released: Mutex<bool>,
    changed: Condvar,
}
fn hooks() -> &'static Mutex<std::collections::HashMap<String, Arc<Hook>>> {
    static HOOKS: OnceLock<Mutex<std::collections::HashMap<String, Arc<Hook>>>> = OnceLock::new();
    HOOKS.get_or_init(Default::default)
}
thread_local! { static CURRENT: std::cell::RefCell<Option<Arc<Hook>>> = const { std::cell::RefCell::new(None) }; }
pub(super) struct Scope;
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|h| h.replace(None));
    }
}
pub(super) fn enter(id: &str) -> Scope {
    CURRENT.with(|h| h.replace(hooks().lock().unwrap().get(id).cloned()));
    Scope
}
pub(super) fn force_copy() -> bool {
    CURRENT.with(|h| h.borrow().as_ref().is_some_and(|h| h.force_copy))
}
pub(super) fn after_copy_chunk(cancel: &CancellationToken) {
    let hook = CURRENT.with(|h| h.borrow().clone());
    let Some(hook) = hook else {
        return;
    };
    if hook.crash == 5 {
        panic!("Simulated interruption during copy");
    }
    if hook.crash != 0 || hook.reached.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = hook.arrived.send(());
    let mut released = hook.released.lock().unwrap();
    while !*released && !cancel.is_cancelled() {
        released = hook
            .changed
            .wait_timeout(released, Duration::from_millis(10))
            .unwrap()
            .0;
    }
}
pub(super) fn after_publish(_id: &str) {
    if CURRENT.with(|h| h.borrow().as_ref().is_some_and(|h| h.crash == 2)) {
        panic!("Simulated crash after filesystem publication, before database commit");
    }
}
pub(super) fn before_publish() {
    if CURRENT.with(|h| h.borrow().as_ref().is_some_and(|h| h.crash == 1)) {
        panic!("Interrupted before publication");
    }
}
pub(super) fn after_commit() {
    if CURRENT.with(|h| h.borrow().as_ref().is_some_and(|h| h.crash == 3)) {
        panic!("Interrupted after commit");
    }
}
pub(super) fn after_copy(file: &mut File) -> Result<()> {
    if CURRENT.with(|h| h.borrow().as_ref().is_some_and(|h| h.crash == 4)) {
        file.seek(SeekFrom::Start(0))?;
        file.write_all(b"corrupt")?;
    }
    Ok(())
}
pub(crate) struct Gate {
    id: String,
    hook: Arc<Hook>,
    arrived: mpsc::Receiver<()>,
}
impl Gate {
    pub(crate) fn copying(id: &str) -> Self {
        Self::new(id, true, 0)
    }
    fn new(id: &str, force_copy: bool, crash: u8) -> Self {
        let (tx, arrived) = mpsc::channel();
        let hook = Arc::new(Hook {
            force_copy,
            crash,
            reached: AtomicBool::new(false),
            arrived: tx,
            released: Mutex::new(crash != 0),
            changed: Condvar::new(),
        });
        hooks().lock().unwrap().insert(id.into(), hook.clone());
        Self {
            id: id.into(),
            hook,
            arrived,
        }
    }
    pub(crate) fn wait(&self) {
        self.arrived
            .recv_timeout(Duration::from_secs(10))
            .expect("Copy did not reach the controlled chunk boundary");
    }
    pub(crate) fn release(&self) {
        *self.hook.released.lock().unwrap() = true;
        self.hook.changed.notify_all();
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
        hooks().lock().unwrap().remove(&self.id);
    }
}

fn fixture() -> (tempfile::TempDir, Store, Job) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(crate::paths::AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let job = store
        .new_download(
            "https://example.invalid/fixture.bin",
            Some("original.bin"),
            None,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    let mut job = store.enqueue(&job).unwrap();
    let path = job.directory.join(&job.name);
    std::fs::create_dir_all(&job.directory).unwrap();
    std::fs::write(&path, vec![0x4a; 700_000]).unwrap();
    job.status = Status::Complete;
    job.saved_path = Some(path);
    job.downloaded = 700_000;
    job.total = Some(700_000);
    store.save(&job).unwrap();
    (temp, store, job)
}
fn run(store: &Store, job: &Job, copy: bool) -> Result<()> {
    relocate_inner(
        store,
        job,
        job.directory.join("moved.bin"),
        "moved.bin".into(),
        job.directory.clone(),
        &CancellationToken::new(),
        copy,
    )
}

#[test]
fn rename_and_copy_preserve_bytes_streams_and_neighbouring_download_state() {
    for copied in [false, true] {
        let (_temp, store, job) = fixture();
        let mut neighbour = store
            .new_download(
                "https://example.invalid/paused.bin",
                None,
                None,
                uuid::Uuid::new_v4().to_string(),
            )
            .unwrap();
        neighbour.status = Status::Paused;
        store.enqueue(&neighbour).unwrap();
        let before = serde_json::to_value(store.get(&neighbour.id).unwrap()).unwrap();
        let cache = store.paths.data.join("parts").join(&neighbour.id);
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("keep.part"), b"existing partial download").unwrap();
        for name in ["Zone.Identifier", "user-note"] {
            std::fs::write(
                format!("{}:{name}", job.saved_path.as_ref().unwrap().display()),
                b"retained stream",
            )
            .unwrap();
        }
        run(&store, &job, copied).unwrap();
        let moved = store.get(&job.id).unwrap().unwrap();
        let path = moved.saved_path.as_ref().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), vec![0x4a; 700_000]);
        for name in ["Zone.Identifier", "user-note"] {
            assert_eq!(
                std::fs::read(format!("{}:{name}", path.display())).unwrap(),
                b"retained stream"
            );
        }
        assert!(!job.saved_path.as_ref().unwrap().exists());
        assert!(pending(&store).unwrap().is_empty());
        assert_eq!(
            serde_json::to_value(store.get(&neighbour.id).unwrap()).unwrap(),
            before
        );
        assert_eq!(
            std::fs::read(cache.join("keep.part")).unwrap(),
            b"existing partial download"
        );
    }
}

#[test]
fn interrupted_publication_recovers_after_reopen_for_rename_and_copy() {
    for copied in [false, true] {
        let (_temp, store, job) = fixture();
        let gate = Gate::new(&job.id, copied, 2);
        let panic =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&store, &job, copied)));
        assert!(panic.is_err());
        drop(gate);
        let paths = store.paths.clone();
        drop(store);
        let store = Store::open(paths).unwrap();
        recover(&store, &job.id, &CancellationToken::new()).unwrap();
        let moved = store.get(&job.id).unwrap().unwrap();
        assert_eq!(moved.saved_path, Some(job.directory.join("moved.bin")));
        assert_eq!(
            std::fs::read(moved.saved_path.unwrap()).unwrap(),
            vec![0x4a; 700_000]
        );
        assert!(!job.saved_path.as_ref().unwrap().exists());
        assert!(pending(&store).unwrap().is_empty());
    }
}

#[test]
fn failed_metadata_commit_retains_recovery_and_never_rolls_back_over_another_file() {
    let (_temp, store, job) = fixture();
    let conn = store.connect().unwrap();
    conn.execute_batch("CREATE TRIGGER stop_move BEFORE UPDATE ON jobs BEGIN SELECT RAISE(ABORT,'simulated save failure'); END;").unwrap();
    assert!(run(&store, &job, false).is_err());
    assert!(job.directory.join("moved.bin").is_file());
    std::fs::write(job.saved_path.as_ref().unwrap(), b"new unrelated file").unwrap();
    conn.execute_batch("DROP TRIGGER stop_move").unwrap();
    recover(&store, &job.id, &CancellationToken::new()).unwrap();
    assert_eq!(
        std::fs::read(job.saved_path.as_ref().unwrap()).unwrap(),
        b"new unrelated file"
    );
    assert_eq!(
        store.get(&job.id).unwrap().unwrap().saved_path,
        Some(job.directory.join("moved.bin"))
    );
}

#[test]
fn cancellation_cleans_only_owned_staging_and_keeps_original() {
    let (_temp, store, job) = fixture();
    let gate = Gate::copying(&job.id);
    let cancel = CancellationToken::new();
    let worker_store = store.clone();
    let worker_job = job.clone();
    let token = cancel.clone();
    let worker = std::thread::spawn(move || {
        relocate(
            &worker_store,
            &worker_job,
            worker_job.directory.join("moved.bin"),
            "moved.bin".into(),
            worker_job.directory.clone(),
            &token,
        )
    });
    gate.wait();
    cancel.cancel();
    assert!(worker.join().unwrap().is_err());
    assert_eq!(
        std::fs::read(job.saved_path.unwrap()).unwrap(),
        vec![0x4a; 700_000]
    );
    assert!(!job.directory.join("moved.bin").exists());
    assert!(pending(&store).unwrap().is_empty());
    assert_eq!(std::fs::read_dir(&job.directory).unwrap().count(), 1);
}

#[test]
fn interrupted_copy_recovers_at_each_durable_boundary() {
    for phase in [1, 3, 5] {
        let (_temp, store, job) = fixture();
        let gate = Gate::new(&job.id, true, phase);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&store, &job, true)))
                .is_err()
        );
        drop(gate);
        let paths = store.paths.clone();
        drop(store);
        let store = Store::open(paths).unwrap();
        recover(&store, &job.id, &CancellationToken::new()).unwrap();
        let saved = store.get(&job.id).unwrap().unwrap();
        let expected = if phase == 3 {
            job.directory.join("moved.bin")
        } else {
            job.saved_path.clone().unwrap()
        };
        assert_eq!(saved.saved_path.as_ref(), Some(&expected));
        assert_eq!(std::fs::read(expected).unwrap(), vec![0x4a; 700_000]);
        assert!(pending(&store).unwrap().is_empty());
        assert_eq!(std::fs::read_dir(&job.directory).unwrap().count(), 1);
    }
}

#[test]
fn checksum_failure_and_destination_collision_preserve_original() {
    let (_temp, store, job) = fixture();
    let gate = Gate::new(&job.id, true, 4);
    let error = run(&store, &job, true).unwrap_err();
    assert!(error.to_string().contains("checksum"), "{error:#}");
    drop(gate);
    assert_eq!(
        std::fs::read(job.saved_path.as_ref().unwrap()).unwrap(),
        vec![0x4a; 700_000]
    );
    assert!(pending(&store).unwrap().is_empty());
    let target = job.directory.join("moved.bin");
    std::fs::write(&target, b"unrelated existing file").unwrap();
    for copied in [false, true] {
        assert!(
            run(&store, &job, copied)
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"unrelated existing file");
    }
    assert_eq!(std::fs::read_dir(&job.directory).unwrap().count(), 2);
}

#[test]
fn recovery_preserves_a_replaced_target() {
    let (_temp, store, job) = fixture();
    let gate = Gate::new(&job.id, true, 2);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&store, &job, true))).is_err()
    );
    drop(gate);
    let target = job.directory.join("moved.bin");
    // A third party replaces the destination before restart. Its bytes must
    // never be deleted or adopted merely because the pathname matches.
    std::fs::remove_file(&target).unwrap();
    std::fs::write(&target, b"replacement").unwrap();
    recover(&store, &job.id, &CancellationToken::new()).unwrap();
    assert_eq!(
        store.get(&job.id).unwrap().unwrap().saved_path,
        job.saved_path
    );
    assert_eq!(std::fs::read(target).unwrap(), b"replacement");
    assert_eq!(
        std::fs::read(job.saved_path.unwrap()).unwrap(),
        vec![0x4a; 700_000]
    );
}

#[test]
fn opening_an_older_profile_and_relocation_preserve_existing_metadata() {
    let (_temp, store, job) = fixture();
    let conn = store.connect().unwrap();
    let data: String = conn
        .query_row("SELECT data FROM jobs WHERE id=?1", [&job.id], |r| r.get(0))
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&data).unwrap();
    value["retained_extension"] = serde_json::json!({"keep": [1, 2, 3]});
    conn.execute(
        "UPDATE jobs SET data=?1 WHERE id=?2",
        params![value.to_string(), job.id],
    )
    .unwrap();
    conn.execute_batch("DROP TABLE file_relocations").unwrap();
    crate::store::migrations::unversioned_fixture(&conn);
    drop(conn);
    let paths = store.paths.clone();
    drop(store);
    let reopened = Store::open(paths).unwrap();
    let read = || -> serde_json::Value {
        let data: String = reopened
            .connect()
            .unwrap()
            .query_row("SELECT data FROM jobs WHERE id=?1", [&job.id], |r| r.get(0))
            .unwrap();
        serde_json::from_str(&data).unwrap()
    };
    assert_eq!(
        read(),
        value,
        "Additive initialization rewrote an existing download"
    );
    assert_eq!(
        std::fs::read(job.saved_path.as_ref().unwrap()).unwrap(),
        vec![0x4a; 700_000]
    );
    run(&reopened, &job, false).unwrap();
    let moved = read();
    value["name"] = moved["name"].clone();
    value["saved_path"] = moved["saved_path"].clone();
    value["directory"] = moved["directory"].clone();
    assert_eq!(
        moved, value,
        "Relocation changed unrelated download metadata"
    );
}

#[test]
fn case_only_rename_keeps_the_existing_file_and_updates_its_name() {
    let (_temp, store, job) = fixture();
    let target = job.directory.join("ORIGINAL.bin");
    relocate(
        &store,
        &job,
        target.clone(),
        "ORIGINAL.bin".into(),
        job.directory.clone(),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        store.get(&job.id).unwrap().unwrap().saved_path,
        Some(target.clone())
    );
    assert_eq!(std::fs::read(&target).unwrap(), vec![0x4a; 700_000]);
    assert_eq!(
        std::fs::read_dir(&job.directory)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name(),
        "ORIGINAL.bin"
    );
}
