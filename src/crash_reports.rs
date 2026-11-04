//! Local failure reports, deliberately without panic payloads or process memory.
//! Rust panic hooks are not native exception/signal handlers. A desktop session
//! marker also detects forced termination/power loss, without claiming its cause.
use crate::{diagnostics::Component, paths::AppPaths};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;

const MAX_BYTES: u64 = 8192;
const MAX_ENTRIES: usize = 4096;
pub const RETAINED_REPORTS: usize = 20;
static RECORDER: OnceLock<Recorder> = OnceLock::new();
static FAILED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    RustPanic,
    UnexpectedExit,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceLocation {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: u8,
    pub id: Uuid,
    pub session_started_utc: String,
    pub recorded_utc: String,
    pub version: String,
    pub os: String,
    pub architecture: String,
    pub component: Component,
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<SourceLocation>,
}

#[derive(Clone, Debug)]
pub struct SavedReport {
    pub report: Report,
    pub unreviewed: bool,
}

#[derive(Default)]
pub struct Snapshot {
    pub reports: Vec<SavedReport>,
    pub skipped: usize,
}

struct Recorder {
    root: PathBuf,
    base: Report,
    captured: AtomicBool,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl Report {
    fn new(component: Component, id: Uuid) -> Self {
        let time = now();
        Self {
            schema: 1,
            id,
            session_started_utc: time.clone(),
            recorded_utc: time,
            version: env!("CARGO_PKG_VERSION").into(),
            os: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            component,
            kind: Kind::UnexpectedExit,
            location: None,
        }
    }

    fn valid(&self) -> bool {
        let time = |s: &str| s.len() <= 30 && chrono::DateTime::parse_from_rfc3339(s).is_ok();
        self.schema == 1
            && !self.id.is_nil()
            && time(&self.session_started_utc)
            && time(&self.recorded_utc)
            && !self.version.is_empty()
            && self.version.len() <= 32
            && self
                .version
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'.')
            && ["windows", "macos", "linux"].contains(&self.os.as_str())
            && ["x86", "x86_64", "aarch64", "arm"].contains(&self.architecture.as_str())
            && (self.kind == Kind::RustPanic || self.location.is_none())
            && self
                .location
                .as_ref()
                .is_none_or(|p| valid_source(&p.file) && p.line > 0 && p.column > 0)
    }
}

fn valid_source(file: &str) -> bool {
    file.len() <= 240
        && file.ends_with(".rs")
        && ["src/", "tests/", "torrent/", "vendor/", "dependency/"]
            .iter()
            .any(|prefix| file.starts_with(prefix))
        && file
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
        && file
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn source_location(location: &std::panic::Location<'_>) -> Option<SourceLocation> {
    let file = location.file().replace('\\', "/");
    let root = format!("{}/", env!("CARGO_MANIFEST_DIR").replace('\\', "/"));
    let relative = file.strip_prefix(&root).unwrap_or(&file);
    let file = if valid_source(relative) {
        relative.to_owned()
    } else {
        // Dependencies may have absolute build-machine paths. Keep only the
        // Rust source basename, never a developer's home or registry path.
        format!("dependency/{}", file.rsplit('/').next()?)
    };
    valid_source(&file).then_some(SourceLocation {
        file,
        line: location.line(),
        column: location.column(),
    })
}

pub fn directory(paths: &AppPaths) -> PathBuf {
    paths.data.join("logs").join("crashes")
}

fn report_path(root: &Path, id: Uuid, reviewed: bool) -> PathBuf {
    root.join(format!(
        "{}-{id}.json",
        if reviewed { "reviewed" } else { "report" }
    ))
}

fn read_report(path: &Path) -> Result<Report> {
    // The shared file opener refuses symlinks, reparse points and hard links.
    #[cfg(not(unix))]
    let file = crate::download_file::read_part_sync(path)?;
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.nlink() == 1,
            "Crash report is not an owned regular file"
        );
        file
    };
    ensure!(
        file.metadata()?.len() <= MAX_BYTES,
        "Crash report exceeds the size limit"
    );
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Crash report exceeds the size limit"
    );
    let report: Report = serde_json::from_slice(&bytes).context("Invalid crash report")?;
    ensure!(report.valid(), "Invalid crash report metadata");
    Ok(report)
}

fn write_new(path: &Path, report: &Report) -> Result<()> {
    ensure!(report.valid(), "Invalid crash report metadata");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let result = (|| -> Result<()> {
        serde_json::to_writer_pretty(&mut file, report)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    })();
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn saved_name(name: &str) -> Option<(Uuid, bool)> {
    let (id, reviewed) = if let Some(id) = name.strip_prefix("report-") {
        (id, false)
    } else {
        (name.strip_prefix("reviewed-")?, true)
    };
    let id = id.strip_suffix(".json")?;
    let parsed = Uuid::parse_str(id).ok()?;
    (parsed.to_string() == id).then_some((parsed, reviewed))
}

pub fn load(paths: &AppPaths) -> Result<Snapshot> {
    load_root(&directory(paths))
}

fn load_root(root: &Path) -> Result<Snapshot> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Snapshot::default());
        }
        Err(error) => return Err(error.into()),
    };
    let mut snapshot = Snapshot::default();
    for (index, entry) in entries.enumerate() {
        ensure!(
            index < MAX_ENTRIES,
            "The crash report folder contains too many entries"
        );
        let entry = entry?;
        let name = entry.file_name();
        let Some((id, reviewed)) = name.to_str().and_then(saved_name) else {
            continue;
        };
        match read_report(&entry.path()) {
            Ok(report) if report.id == id => snapshot.reports.push(SavedReport {
                report,
                unreviewed: !reviewed,
            }),
            _ => snapshot.skipped += 1,
        }
    }
    snapshot.reports.sort_by(|a, b| {
        b.report
            .recorded_utc
            .cmp(&a.report.recorded_utc)
            .then(a.report.id.cmp(&b.report.id))
    });
    Ok(snapshot)
}

fn retain_recent(root: &Path) -> Result<()> {
    // Only our validated report names are removed. Unknown files, incomplete
    // writes and redirected paths are never treated as owned report data.
    let reports = load_root(root)?;
    for saved in reports.reports.iter().skip(RETAINED_REPORTS) {
        let path = report_path(root, saved.report.id, !saved.unreviewed);
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(crate) fn init(paths: &AppPaths, component: Component, session: Uuid) -> Result<()> {
    let root = directory(paths);
    fs::create_dir_all(&root)?;
    retain_recent(&root)?;
    RECORDER
        .set(Recorder {
            root,
            base: Report::new(component, session),
            captured: AtomicBool::new(false),
        })
        .map_err(|_| anyhow::anyhow!("Crash reporting is already initialized"))
}

pub fn available() -> bool {
    RECORDER.get().is_some() && !FAILED.load(Ordering::Relaxed)
}

pub(crate) fn capture(location: Option<&std::panic::Location<'_>>) {
    let Some(recorder) = RECORDER.get() else {
        return;
    };
    // One immutable report per process. This avoids recursive panic handling
    // and races between panicking workers. No database/log lock or queue wait.
    if recorder.captured.swap(true, Ordering::AcqRel) {
        return;
    }
    let mut report = recorder.base.clone();
    report.kind = Kind::RustPanic;
    report.recorded_utc = now();
    report.location = location.and_then(source_location);
    if write_new(&report_path(&recorder.root, report.id, false), &report).is_err() {
        FAILED.store(true, Ordering::Relaxed);
        let _ = writeln!(
            std::io::stderr(),
            "Download Engine could not save its local crash report."
        );
    }
}

/// Construct only after taking the desktop profile lock; keep it alive until
/// the desktop has stopped. A second launch must never replace this marker.
pub struct DesktopSession {
    root: PathBuf,
    id: Uuid,
}
impl DesktopSession {
    pub fn begin(paths: &AppPaths) -> Result<Self> {
        let root = directory(paths);
        fs::create_dir_all(&root)?;
        let marker = root.join("desktop-session.json");
        if marker.try_exists()? {
            let mut old = read_report(&marker)?;
            ensure!(
                old.component == Component::Desktop && old.kind == Kind::UnexpectedExit,
                "Invalid desktop session marker"
            );
            // A panic already explains this session; don't invent a second
            // incident when it is found again during the following launch.
            if !report_path(&root, old.id, false).try_exists()?
                && !report_path(&root, old.id, true).try_exists()?
            {
                old.recorded_utc = now();
                write_new(&report_path(&root, old.id, false), &old)?;
            }
            fs::remove_file(&marker)?;
        }
        retain_recent(&root)?;
        let base = RECORDER
            .get()
            .filter(|r| r.root == root && r.base.component == Component::Desktop)
            .map(|r| r.base.clone())
            .unwrap_or_else(|| Report::new(Component::Desktop, Uuid::new_v4()));
        write_new(&marker, &base)?;
        Ok(Self { root, id: base.id })
    }
}
impl Drop for DesktopSession {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let marker = self.root.join("desktop-session.json");
        let cleaned = read_report(&marker).and_then(|report| {
            ensure!(report.id == self.id, "Desktop session ownership changed");
            fs::remove_file(marker)?;
            Ok(())
        });
        if cleaned.is_err() {
            let _ = writeln!(
                std::io::stderr(),
                "Download Engine could not mark this session as closed."
            );
        }
    }
}

pub fn acknowledge(paths: &AppPaths, ids: &[Uuid]) -> Result<()> {
    ensure!(ids.len() <= MAX_ENTRIES, "Too many crash reports");
    let root = directory(paths);
    for &id in ids {
        let source = report_path(&root, id, false);
        if !source.try_exists()? {
            continue;
        }
        ensure!(
            read_report(&source)?.id == id,
            "Crash report identity changed"
        );
        let destination = report_path(&root, id, true);
        ensure!(!destination.try_exists()?, "Reviewed report already exists");
        fs::rename(source, destination)?;
    }
    Ok(())
}

/// The preview, clipboard and exported file are exactly the same allowlisted
/// data. No log directory, queue DB, memory dump or arbitrary text is attached.
pub fn preview(report: &Report) -> Result<String> {
    ensure!(report.valid(), "Invalid crash report metadata");
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "application": "Download Engine",
        "privacy": "Local report only. No URLs, download filenames, personal paths, credentials, cookies, panic messages, environment variables or process memory. Source locations refer to application/dependency code.",
        "report": report,
    }))? + "\n")
}

pub fn export(report: &Report, destination: &Path) -> Result<PathBuf> {
    ensure!(
        destination.is_absolute() && destination.is_dir(),
        "Choose an existing folder for the crash report"
    );
    let text = preview(report)?;
    let path = destination.join(format!(
        "DownloadEngine-crash-{}-{}.json",
        report.id,
        Uuid::new_v4()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    let result = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(&path);
    }
    result?;
    Ok(path)
}

#[cfg(test)]
mod tests;
