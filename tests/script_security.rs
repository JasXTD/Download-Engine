//! Real Lua/CLI calls against isolated profiles; queues stay paused.
use download_engine::{
    control::Action,
    paths::AppPaths,
    script_automation::{Language, Run, Script, Trigger},
    store::Store,
};
use std::process::{Command, Output};

struct Fixture {
    temp: tempfile::TempDir,
    store: Store,
    queue: String,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        })
        .unwrap();
        store.set_queue_running("main", false).unwrap();
        let queue = store.create_queue("Held security fixtures").unwrap().id;
        Self { temp, store, queue }
    }
    fn run(&self, sample: bool, change_files: bool) -> Run {
        let script = self
            .store
            .save_script(&Script {
                id: uuid::Uuid::new_v4().to_string(),
                revision: 0,
                name: "Security fixture".into(),
                language: Language::Lua,
                body: "-- invoked explicitly by the test".into(),
                enabled: false,
                trigger: Trigger::QueueCompleted {
                    queue: self.queue.clone(),
                },
                timeout_seconds: 30,
                change_files,
            })
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        if sample {
            self.store.run_script_sample(&script.id, &id, &[]).unwrap()
        } else {
            self.store.run_script(&script.id, &id).unwrap()
        }
    }
    fn lua(&self, run: &Run, source: &str) -> Output {
        let file = self.temp.path().join(format!("{}.lua", run.id));
        let event = self.temp.path().join(format!("{}.json", run.id));
        std::fs::write(&file, source).unwrap();
        std::fs::write(&event, serde_json::to_vec(&run.context).unwrap()).unwrap();
        // Release verification can exercise the packaged CLI with the same
        // isolated fixtures and the scheduler library built from this source.
        let binary = std::env::var_os("XDOWNLOADER_SECURITY_CLI")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_de-cli").into());
        let mut command = Command::new(binary);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command
            .arg("--lua-worker")
            .env("IDM_EX_SCRIPT_RUN", &run.id)
            .env(
                "IDM_EX_SCRIPT_PROFILE",
                serde_json::to_string(&self.store.paths).unwrap(),
            )
            .env("XDOWNLOADER_SCRIPT_FILE", file)
            .env("XDOWNLOADER_EVENT_FILE", event)
            .output()
            .unwrap()
    }
}
fn quoted(value: impl AsRef<str>) -> String {
    serde_json::to_string(value.as_ref()).unwrap()
}

#[test]
fn script_security_add_cannot_target_an_unconfigured_folder() {
    for change_files in [false, true] {
        let f = Fixture::new();
        let run = f.run(false, change_files);
        let _scheduler = download_engine::scheduler::Scheduler::start(f.store.clone()).unwrap();
        let outside = f.temp.path().join("outside-download-roots");
        let output = f.lua(&run, &format!(
            "xd.add('https://example.invalid/fixture.bin', {{ queue = {}, folder = {}, name = 'harmless-fixture.bin' }})",
            quoted(&f.queue), quoted(outside.to_str().unwrap())
        ));
        assert!(
            !output.status.success(),
            "A script queued a file outside its configured roots"
        );
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("download folder"), "{error}");
        assert!(f.store.list().unwrap().is_empty());
        assert!(!outside.exists());
    }
}

#[test]
fn script_security_add_allows_configured_folders_and_keeps_provenance() {
    let f = Fixture::new();
    let category = f.temp.path().join("configured-category");
    let mut folders = f.store.download_folders().unwrap();
    folders.overrides.insert(
        download_engine::categories::Category::Documents,
        category.clone(),
    );
    f.store.save_download_folders(&folders).unwrap();
    let run = f.run(false, false);
    let _scheduler = download_engine::scheduler::Scheduler::start(f.store.clone()).unwrap();
    let nested = f.store.paths.downloads.join("nested").join("new-folder");
    for (index, folder) in [Some(nested.clone()), Some(category.join("reports")), None]
        .into_iter()
        .enumerate()
    {
        let option = folder
            .as_ref()
            .map(|p| format!(", folder = {}", quoted(p.to_str().unwrap())))
            .unwrap_or_default();
        let output = f.lua(
            &run,
            &format!(
                "xd.add('https://example.invalid/fixture-{index}.pdf', {{ queue = {}{option} }})",
                quoted(&f.queue)
            ),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let jobs = f.store.list().unwrap();
    assert_eq!(jobs.len(), 3);
    assert!(
        jobs.iter()
            .all(|j| j.status == download_engine::model::Status::Queued)
    );
    assert!(jobs.iter().any(|j| j.directory == nested));
    assert!(jobs.iter().any(|j| j.directory == category));
    assert!(!nested.exists() && !category.exists());
    let db = rusqlite::Connection::open(f.store.paths.data.join("queue.sqlite3")).unwrap();
    let mut statement = db.prepare("SELECT action FROM control_requests").unwrap();
    let requests: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(requests.len(), 3);
    for request in requests {
        let request: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(request["script_run"], run.id);
    }
}

#[test]
fn script_security_add_rejects_traversal_prefix_siblings_and_stream_paths() {
    let f = Fixture::new();
    let run = f.run(false, true);
    let _scheduler = download_engine::scheduler::Scheduler::start(f.store.clone()).unwrap();
    for folder in [
        f.store.paths.downloads.join("..").join("outside"),
        f.temp.path().join("downloads-sibling"),
        f.store.paths.downloads.join("folder:stream"),
        f.store.paths.downloads.join("ambiguous."),
    ] {
        let output = f.lua(&run, &format!(
            "xd.command('add', 'https://example.invalid/a.bin', 'https://example.invalid/b.bin', '--queue', {}, '--folder', {})",
            quoted(&f.queue), quoted(folder.to_str().unwrap())
        ));
        assert!(!output.status.success(), "Accepted {}", folder.display());
        assert!(
            f.store.list().unwrap().is_empty(),
            "Rejected batch was partly queued"
        );
    }
}

#[test]
fn script_security_sample_add_does_not_queue_or_create_folders() {
    let f = Fixture::new();
    let run = f.run(true, false);
    let folder = f.store.paths.downloads.join("sample-folder");
    let output = f.lua(
        &run,
        &format!(
            "xd.add('https://example.invalid/sample.bin', {{ queue = {}, folder = {} }})",
            quoted(&f.queue),
            quoted(folder.to_str().unwrap())
        ),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("nothing changed"));
    assert!(f.store.list().unwrap().is_empty() && !folder.exists());
}

#[test]
fn script_security_sample_cannot_cancel_a_real_pending_command() {
    let f = Fixture::new();
    let run = f.run(true, false);
    let pending = uuid::Uuid::new_v4().to_string();
    f.store.submit_control(&pending, &Action::PauseAll).unwrap();
    // Editing the exposed event cannot change the durable run's sample status.
    let output = f.lua(
        &run,
        &format!(
            "event.sample = false; xd.command('cancel', {})",
            quoted(&pending)
        ),
    );
    assert_eq!(
        f.store.control_receipt(&pending).unwrap().state,
        "pending",
        "A sample script changed a real command"
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("sample"));
    // Normal automation retains its documented pending-command cancellation.
    let real = f.run(false, false);
    let output = f.lua(
        &real,
        &format!("xd.command('cancel', {})", quoted(&pending)),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        f.store.control_receipt(&pending).unwrap().state,
        "cancelled"
    );
}

#[cfg(windows)]
#[test]
fn script_security_add_rejects_a_junction_inside_the_download_folder() {
    use std::os::windows::process::CommandExt;
    let f = Fixture::new();
    let outside = f.temp.path().join("junction-target");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(&f.store.paths.downloads).unwrap();
    let sentinel = outside.join("sentinel");
    std::fs::write(&sentinel, b"unchanged").unwrap();
    let link = f.store.paths.downloads.join("linked");
    let explorer = download_engine::paths::file_browser().unwrap();
    let result = Command::new(explorer.parent().unwrap().join("System32").join("cmd.exe"))
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let run = f.run(false, true);
    let scheduler = download_engine::scheduler::Scheduler::start(f.store.clone()).unwrap();
    let output = f.lua(
        &run,
        &format!(
            "xd.add('https://example.invalid/fixture.bin', {{ queue = {}, folder = {} }})",
            quoted(&f.queue),
            quoted(link.join("new-folder").to_str().unwrap())
        ),
    );
    drop(scheduler);
    std::fs::remove_dir(&link).unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    // Windows may report the reparse handle as a non-directory before the
    // explicit linked-folder check. Both reject the destination before writes.
    assert!(error.contains("folder"), "{error}");
    assert!(f.store.list().unwrap().is_empty());
    assert!(!outside.join("new-folder").exists());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"unchanged");
}
