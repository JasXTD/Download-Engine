//! Local, bounded diagnostics. Only typed metadata is persisted: never request
//! bodies, URLs, paths, headers, credentials, or arbitrary exception text.
use crate::paths::AppPaths;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
};
use uuid::Uuid;

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const BACKUPS: usize = 3;
const MAX_RECORD_BYTES: usize = 4096;
static LOGGER: OnceLock<Client> = OnceLock::new();
static WRITE_FAILED: AtomicBool = AtomicBool::new(false);
static DROPPED: AtomicU64 = AtomicU64::new(0);

struct Client {
    writer: Arc<Logger>,
    sender: mpsc::SyncSender<Pending>,
}
enum Pending {
    Record(Box<Record>),
    Flush(mpsc::Sender<()>),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    Desktop,
    NativeHost,
}
impl Component {
    fn name(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::NativeHost => "native-host",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Started,
    Ready,
    Stopped,
    StartupFailed,
    Panic,
    DownloadStarted,
    DownloadCompleted,
    DownloadPaused,
    DownloadRetryScheduled,
    DownloadQuotaHeld,
    DownloadFailed,
    DownloadStateSaveFailed,
    WorkerFailed,
    SchedulerFailed,
    UiError,
    McpStarted,
    McpStopped,
    McpFailed,
    VideoLookupStarted,
    VideoLookupSucceeded,
    VideoLookupFailed,
    BrowserRegistered,
    BrowserUnregistered,
    BrowserSetupFailed,
    NativeRequestReceived,
    NativeRequestSucceeded,
    NativeRequestFailed,
    NativeOperationFailed,
    NativeHostFailed,
    NativeMessageInvalid,
    DesktopLaunchFailed,
    DiagnosticsExported,
    EntriesDropped,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NativeAction {
    BrowserRecovery,
    Ping,
    BatchOptions,
    PreviewVideo,
    ReviewVideo,
    AddBatch,
    Add,
    AddBrowserLogin,
    CapturePrepare,
    CaptureCommit,
    CaptureStatus,
    CaptureAbort,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Application,
    Io,
    NotFound,
    PermissionDenied,
    DiskFull,
    Database,
    Network,
    Timeout,
    Authentication,
    Quota,
    Cancelled,
    Unsupported,
    Validation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub kind: FailureKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}
impl Failure {
    /// Classify existing UI strings without ever copying them into the log.
    pub fn message(message: &str) -> Self {
        let text: String = message
            .chars()
            .take(4096)
            .flat_map(char::to_lowercase)
            .collect();
        let kind = if text.contains("timed out") || text.contains("timeout") {
            FailureKind::Timeout
        } else if text.contains("sqlite") || text.contains("database") {
            FailureKind::Database
        } else if text.contains("credential")
            || text.contains("authentication")
            || text.contains("unauthorized")
        {
            FailureKind::Authentication
        } else if text.contains("quota") {
            FailureKind::Quota
        } else if text.contains("cancelled") {
            FailureKind::Cancelled
        } else if text.contains("unsupported") {
            FailureKind::Unsupported
        } else if text.contains("invalid") {
            FailureKind::Validation
        } else if text.contains("connection") || text.contains("server") || text.contains("http") {
            FailureKind::Network
        } else {
            FailureKind::Application
        };
        Self {
            kind,
            code: None,
            http_status: text.split_once("http ").and_then(|(_, value)| {
                let status = value.get(..3)?.parse::<u16>().ok()?;
                ((100..=599).contains(&status)
                    && !value.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
                .then_some(status)
            }),
        }
    }
    pub fn error(error: &anyhow::Error) -> Self {
        // Prefer structured codes. Display chains may contain secrets supplied
        // by a server, a URL parser, the OS, or a panic payload.
        for source in error.chain() {
            if let Some(error) = source.downcast_ref::<rusqlite::Error>() {
                return Self {
                    kind: FailureKind::Database,
                    code: error.sqlite_error().map(|e| e.extended_code),
                    http_status: None,
                };
            }
            if let Some(error) = source.downcast_ref::<reqwest::Error>() {
                return Self {
                    kind: if error.is_timeout() {
                        FailureKind::Timeout
                    } else {
                        FailureKind::Network
                    },
                    code: None,
                    http_status: error.status().map(|s| s.as_u16()),
                };
            }
            if let Some(error) = source.downcast_ref::<io::Error>() {
                return Self {
                    kind: match error.kind() {
                        io::ErrorKind::NotFound => FailureKind::NotFound,
                        io::ErrorKind::PermissionDenied => FailureKind::PermissionDenied,
                        io::ErrorKind::StorageFull => FailureKind::DiskFull,
                        io::ErrorKind::TimedOut => FailureKind::Timeout,
                        _ => FailureKind::Io,
                    },
                    code: error.raw_os_error(),
                    http_status: None,
                };
            }
        }
        Self::message(&error.to_string())
    }
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Details {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<NativeAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
}

/// Typed event metadata. Use `read_recent` to load validated records from disk.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub time: String,
    pub version: String,
    pub component: Component,
    pub pid: u32,
    pub session: Uuid,
    pub level: Level,
    pub event: Event,
    pub details: Details,
}

struct Logger {
    root: PathBuf,
    component: Component,
    session: Uuid,
    limit: u64,
}
impl Logger {
    fn new(paths: &AppPaths, component: Component) -> io::Result<Self> {
        let root = directory(paths);
        fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            component,
            session: Uuid::new_v4(),
            limit: MAX_FILE_BYTES,
        })
    }
    fn write(
        &self,
        level: Level,
        event: Event,
        details: Details,
        nonblocking: bool,
    ) -> io::Result<()> {
        self.write_record(self.entry(level, event, details), nonblocking)
    }
    fn entry(&self, level: Level, event: Event, details: Details) -> Record {
        Record {
            time: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            version: env!("CARGO_PKG_VERSION").into(),
            component: self.component,
            pid: std::process::id(),
            session: self.session,
            level,
            event,
            details,
        }
    }
    fn write_record(&self, record: Record, nonblocking: bool) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(&record)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(io::Error::other("Diagnostic record too large"));
        }
        // Open anew under an OS lock: several browser host processes can run
        // together. No writer retains a handle to a renamed log on Windows.
        let _guard = lock(&self.root, self.component, nonblocking)?;
        let path = log_path(&self.root, self.component, 0);
        let length = match fs::metadata(&path) {
            Ok(meta) => meta.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error),
        };
        if length > 0 {
            let mut existing = File::open(&path)?;
            existing.seek(SeekFrom::End(-1))?;
            let mut last = [0];
            existing.read_exact(&mut last)?;
            if last[0] != b'\n' {
                // Isolate a crash-interrupted tail so the next event remains a
                // valid record. Export reports and skips the partial entry.
                bytes.insert(0, b'\n');
            }
        }
        if length.saturating_add(bytes.len() as u64) > self.limit {
            remove_if_present(&log_path(&self.root, self.component, BACKUPS))?;
            for index in (0..BACKUPS).rev() {
                let old = log_path(&self.root, self.component, index);
                match fs::rename(old, log_path(&self.root, self.component, index + 1)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(&bytes)?;
        file.flush()
    }
}

pub fn directory(paths: &AppPaths) -> PathBuf {
    paths.data.join("logs")
}

fn log_path(root: &Path, component: Component, index: usize) -> PathBuf {
    root.join(if index == 0 {
        format!("{}.log", component.name())
    } else {
        format!("{}.{}.log", component.name(), index)
    })
}
fn lock(root: &Path, component: Component, nonblocking: bool) -> io::Result<File> {
    lock_mode(
        root,
        component,
        if nonblocking {
            LockMode::Try
        } else {
            LockMode::Wait
        },
    )
}
enum LockMode {
    Try,
    Wait,
    Bounded,
}
fn lock_mode(root: &Path, component: Component, mode: LockMode) -> io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(format!("{}.lock", component.name())))?;
    if matches!(mode, LockMode::Wait) {
        // Only the background writer waits. OS-queued locks avoid starvation
        // between browser processes; UI and transfer threads only enqueue.
        fs2::FileExt::lock_exclusive(&file)?;
        return Ok(file);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => break,
            Err(error)
                if matches!(mode, LockMode::Bounded)
                    && error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
                    && std::time::Instant::now() < deadline =>
            {
                // Bounded contention backoff; a stuck external lock must not
                // freeze a download, a native reply, or application shutdown.
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(file)
}
fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

pub fn init(paths: &AppPaths, component: Component) -> io::Result<()> {
    let result = (|| {
        let logger = Arc::new(Logger::new(paths, component)?);
        let (sender, receiver) = mpsc::sync_channel(1024);
        let writer = logger.clone();
        std::thread::Builder::new()
            .name("diagnostic-writer".into())
            .spawn(move || {
                while let Ok(pending) = receiver.recv() {
                    match pending {
                        Pending::Record(record) => {
                            let reported = writer.report_dropped();
                            if writer.write_record(*record, false).is_err() {
                                DROPPED.fetch_add(1, Ordering::Relaxed);
                                unavailable();
                            } else if reported {
                                WRITE_FAILED.store(false, Ordering::Relaxed);
                            }
                        }
                        Pending::Flush(done) => {
                            if !writer.report_dropped() {
                                unavailable();
                            }
                            let _ = done.send(());
                        }
                    }
                }
            })?;
        LOGGER
            .set(Client {
                writer: logger,
                sender,
            })
            .map_err(|_| io::Error::other("Diagnostics already initialized"))?;
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // A panic could originate while writing a log. Never wait on our
            // own file lock here, and never persist the arbitrary panic payload.
            if let Some(logger) = LOGGER.get() {
                let _ = logger
                    .writer
                    .write(Level::Error, Event::Panic, Details::default(), true);
            }
            previous(info);
        }));
        Ok(())
    })();
    WRITE_FAILED.store(result.is_err(), Ordering::Relaxed);
    if result.is_ok() {
        record(Level::Info, Event::Started, Details::default());
    }
    result
}
impl Logger {
    fn report_dropped(&self) -> bool {
        let dropped = DROPPED.swap(0, Ordering::Relaxed);
        if dropped > 0
            && self
                .write(
                    Level::Warn,
                    Event::EntriesDropped,
                    Details {
                        dropped: Some(dropped),
                        ..Default::default()
                    },
                    false,
                )
                .is_err()
        {
            DROPPED.fetch_add(dropped, Ordering::Relaxed);
            return false;
        }
        true
    }
}
pub fn available() -> bool {
    LOGGER.get().is_some() && !WRITE_FAILED.load(Ordering::Relaxed)
}
pub fn record(level: Level, event: Event, details: Details) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    let entry = logger.writer.entry(level, event, details);
    if logger
        .sender
        .try_send(Pending::Record(Box::new(entry)))
        .is_err()
    {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        unavailable();
    }
}
fn unavailable() {
    if !WRITE_FAILED.swap(true, Ordering::Relaxed) {
        // stderr is safe for native messaging; stdout is reserved for frames.
        let _ = writeln!(
            io::stderr(),
            "Download Engine diagnostics could not be written. Check the Logs page."
        );
    }
}

/// Drain accepted records at normal shutdown/export. A stuck external lock
/// cannot prevent the app from exiting indefinitely.
pub fn flush() -> io::Result<()> {
    let Some(logger) = LOGGER.get() else {
        return Ok(());
    };
    let (sender, receiver) = mpsc::channel();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match logger.sender.try_send(Pending::Flush(sender.clone())) {
            Ok(()) => break,
            Err(mpsc::TrySendError::Full(_)) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(2))
            }
            Err(_) => return Err(io::Error::other("Diagnostic writer is busy or unavailable")),
        }
    }
    receiver
        .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
        .map_err(|_| io::Error::other("Diagnostic writer did not finish"))?;
    if WRITE_FAILED.load(Ordering::Relaxed) {
        return Err(io::Error::other("Some diagnostics could not be written"));
    }
    Ok(())
}
pub fn error(event: Event, error: &anyhow::Error) {
    record(
        Level::Error,
        event,
        Details {
            failure: Some(Failure::error(error)),
            ..Default::default()
        },
    );
}
pub fn message(event: Event, message: &str) {
    record(
        Level::Error,
        event,
        Details {
            failure: Some(Failure::message(message)),
            ..Default::default()
        },
    );
}

pub const RECENT_LIMIT: usize = 2000;

#[derive(Default, Debug)]
pub struct Snapshot {
    /// Newest first, capped at RECENT_LIMIT. Export still includes all retained logs.
    pub records: Vec<Record>,
    pub skipped: usize,
    pub older: usize,
}

/// Bounded local snapshot for the viewer. Disk I/O and lock acquisition belong
/// on a background thread. Missing logs are an empty state, not a write request.
pub fn read_recent(paths: &AppPaths) -> anyhow::Result<Snapshot> {
    let (mut records, skipped) = read_records(paths)?;
    // Parse offsets as well as UTC. Reverse stable chronological order so events
    // within the same millisecond retain their append order (newest first).
    records.sort_by_cached_key(|record| chrono::DateTime::parse_from_rfc3339(&record.time).ok());
    records.reverse();
    let older = records.len().saturating_sub(RECENT_LIMIT);
    records.truncate(RECENT_LIMIT);
    Ok(Snapshot {
        records,
        skipped,
        older,
    })
}

/// Viewer and export use the same allowlist, validation and size bounds.
fn read_records(paths: &AppPaths) -> anyhow::Result<(Vec<Record>, usize)> {
    let root = directory(paths);
    match fs::metadata(&root) {
        Ok(metadata) => anyhow::ensure!(metadata.is_dir(), "The log location is not a folder"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(error) => return Err(error.into()),
    }
    let mut records = Vec::new();
    let mut skipped = 0usize;
    for component in [Component::Desktop, Component::NativeHost] {
        let mut snapshots = Vec::new();
        let guard = lock_mode(&root, component, LockMode::Bounded)?;
        for index in (0..=BACKUPS).rev() {
            let file = match File::open(log_path(&root, component, index)) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            anyhow::ensure!(
                file.metadata()?.len() <= MAX_FILE_BYTES,
                "A diagnostic log exceeds the size limit"
            );
            let mut snapshot = Vec::new();
            file.take(MAX_FILE_BYTES + 1).read_to_end(&mut snapshot)?;
            anyhow::ensure!(
                snapshot.len() as u64 <= MAX_FILE_BYTES,
                "A diagnostic log exceeds the size limit"
            );
            snapshots.push(snapshot);
        }
        // Keep writers unblocked while parsing/validating the snapshot.
        drop(guard);
        for snapshot in snapshots {
            let mut reader = BufReader::new(snapshot.as_slice());
            loop {
                let mut line = Vec::new();
                let count = reader
                    .by_ref()
                    .take((MAX_RECORD_BYTES + 1) as u64)
                    .read_until(b'\n', &mut line)?;
                if count == 0 {
                    break;
                }
                anyhow::ensure!(
                    count <= MAX_RECORD_BYTES,
                    "A diagnostic entry exceeds the size limit"
                );
                match serde_json::from_slice::<Record>(&line) {
                    Ok(record) if valid_record(&record) => records.push(record),
                    _ => skipped += 1, // e.g. the final line interrupted by a crash
                }
            }
        }
    }
    Ok((records, skipped))
}

/// A deliberately small, allowlisted export. No directory recursion, queue DB,
/// browser storage, crash dump, or settings files are included.
pub fn export(paths: &AppPaths, destination: &Path) -> anyhow::Result<PathBuf> {
    flush()?;
    anyhow::ensure!(
        destination.is_absolute() && destination.is_dir(),
        "Choose an existing folder for diagnostics"
    );
    anyhow::ensure!(
        directory(paths).is_dir(),
        "No diagnostic logs are available yet"
    );
    let (mut records, skipped) = read_records(paths)?;
    anyhow::ensure!(
        !records.is_empty(),
        "No valid diagnostic entries are available to export"
    );
    records.sort_by(|a, b| a.time.cmp(&b.time));
    let report = serde_json::json!({
        "application": "Download Engine", "version": env!("CARGO_PKG_VERSION"),
        "created_utc": chrono::Utc::now().to_rfc3339(), "os": std::env::consts::OS,
        "architecture": std::env::consts::ARCH, "skipped_entries": skipped,
        "privacy": "Typed event metadata only. No URLs, filenames, paths, cookies, credentials, request bodies or raw exception text. Browser extension logs are separate.",
        "records": records,
    });
    let name = format!(
        "DownloadEngine-diagnostics-{}-{}.json",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        Uuid::new_v4()
    );
    let path = destination.join(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let result = (|| -> anyhow::Result<()> {
        serde_json::to_writer_pretty(&mut file, &report)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    })();
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(&path);
    }
    result?;
    record(Level::Info, Event::DiagnosticsExported, Details::default());
    Ok(path)
}

fn valid_record(record: &Record) -> bool {
    // Strings loaded from disk are untrusted too; never export extra fields or
    // secrets injected into nominal timestamp/version fields.
    record.time.len() <= 30
        && chrono::DateTime::parse_from_rfc3339(&record.time).is_ok()
        && record.version.len() <= 32
        && !record.version.is_empty()
        && record
            .version
            .bytes()
            .all(|c| c.is_ascii_digit() || c == b'.')
        && record
            .details
            .failure
            .as_ref()
            .and_then(|f| f.http_status)
            .is_none_or(|status| (100..=599).contains(&status))
}

#[cfg(test)]
mod tests;
