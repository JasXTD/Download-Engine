//! Optional, local antivirus integration. A saved report is history, never an open permit.
mod external;
mod installation;
mod process;
mod service;

pub use external::helper_request as external_helper_request;
pub use installation::{check_setup, open_antivirus};
pub use process::{defender_path, malwarebytes_path};
pub use service::Service;

use crate::{
    integrity,
    model::{Job, Status},
    store::Store,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Defender,
    ClamAv,
    Custom,
    Malwarebytes,
    Kaspersky,
    Bitdefender,
}
impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Defender => "Microsoft Defender",
            Self::ClamAv => "ClamAV",
            Self::Custom => "Other antivirus (command line)",
            Self::Malwarebytes => "Malwarebytes (review in its app)",
            Self::Kaspersky => "Kaspersky (review in its app)",
            Self::Bitdefender => "Bitdefender (review in its app)",
        }
    }
    pub const ALL: [Self; 6] = [
        Self::Defender,
        Self::Malwarebytes,
        Self::Kaspersky,
        Self::Bitdefender,
        Self::ClamAv,
        Self::Custom,
    ];
    pub fn external_review(self) -> bool {
        matches!(
            self,
            Self::Malwarebytes | Self::Kaspersky | Self::Bitdefender
        )
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Malwarebytes => "Malwarebytes",
            Self::Kaspersky => "Kaspersky",
            Self::Bitdefender => "Bitdefender",
            _ => self.label(),
        }
    }
    pub fn helper_id(self) -> Option<&'static str> {
        match self {
            Self::Malwarebytes => Some("malwarebytes"),
            Self::Kaspersky => Some("kaspersky"),
            Self::Bitdefender => Some("bitdefender"),
            _ => None,
        }
    }
    pub fn from_helper_id(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.helper_id() == Some(value))
    }
    pub fn from_report(report: &Report) -> Option<Self> {
        // Report labels are persisted, nonlocalized identifiers from older builds too.
        Self::ALL
            .into_iter()
            .find(|provider| provider.label() == report.scanner)
    }
    pub fn guide_url(self) -> &'static str {
        match self {
            Self::Defender => {
                "https://learn.microsoft.com/en-us/defender-endpoint/command-line-arguments-microsoft-defender-antivirus"
            }
            Self::ClamAv => "https://docs.clamav.net/manual/Usage/Scanning.html",
            Self::Malwarebytes => {
                "https://help.malwarebytes.com/hc/en-us/articles/34572984219035-Scan-a-file-or-folder-with-Malwarebytes"
            }
            Self::Kaspersky => {
                "https://support.kaspersky.com/help/Kaspersky/Win21.5/en-US/199673.htm"
            }
            Self::Bitdefender => "https://www.bitdefender.com/consumer/support/answer/1477/",
            Self::Custom => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub provider: Provider,
    pub executable: String,
    pub arguments: Vec<String>,
    pub clean_exit: Option<i32>,
    pub detected_exit: Option<i32>,
    pub custom_confirmed: bool,
    pub timeout_seconds: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: Provider::Defender,
            executable: String::new(),
            arguments: vec!["{file}".into()],
            clean_exit: None,
            detected_exit: None,
            custom_confirmed: false,
            timeout_seconds: 300,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (10..=3600).contains(&self.timeout_seconds),
            "Scan timeout must be between 10 and 3600 seconds"
        );
        ensure!(
            self.executable.len() <= 4096
                && self.arguments.len() <= 64
                && self.arguments.iter().map(String::len).sum::<usize>() <= 16000,
            "Scanner configuration is too long"
        );
        if matches!(self.provider, Provider::ClamAv | Provider::Custom) {
            validate_executable(Path::new(&self.executable))?;
        }
        if self.provider == Provider::ClamAv {
            ensure!(
                Path::new(&self.executable)
                    .file_name()
                    .is_some_and(|s| s.eq_ignore_ascii_case("clamscan.exe")),
                "Select ClamAV's clamscan.exe"
            );
        }
        if self.provider == Provider::Custom {
            ensure!(
                self.custom_confirmed,
                "Confirm that this scanner waits for the file scan and reports its result using the configured exit codes"
            );
            ensure!(
                self.clean_exit.is_some()
                    && self.detected_exit.is_some()
                    && self.clean_exit != self.detected_exit,
                "Enter different no-threat and threat exit codes from the antivirus documentation"
            );
            ensure!(
                self.arguments
                    .iter()
                    .map(|a| a.matches("{file}").count())
                    .sum::<usize>()
                    == 1,
                "Arguments must contain {{file}} exactly once"
            );
            ensure!(
                self.arguments
                    .iter()
                    .all(|a| !a.contains(['\0', '\r', '\n'])),
                "Enter one argument per line, without null characters"
            );
        }
        Ok(())
    }
    fn key(&self) -> Result<String> {
        Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(self)?)))
    }
}

fn validate_executable(path: &Path) -> Result<()> {
    process::validate_path(path)?;
    let name = path
        .file_name()
        .context("Select an antivirus executable")?
        .to_string_lossy()
        .to_ascii_lowercase();
    ensure!(
        name.ends_with(".exe"),
        "Select the antivirus .exe directly; scripts and shell commands are unsupported"
    );
    ensure!(
        ![
            "cmd.exe",
            "powershell.exe",
            "pwsh.exe",
            "wscript.exe",
            "cscript.exe",
            "mshta.exe",
            "rundll32.exe",
            "regsvr32.exe",
            "explorer.exe"
        ]
        .contains(&name.as_str()),
        "Select the antivirus scanner, not a command interpreter or launcher"
    );
    ensure!(
        ![
            "malwarebytes.exe",
            "mbam.exe",
            "mbbr.exe",
            "mbts.exe",
            "mbtslauncher.exe"
        ]
        .contains(&name.as_str()),
        "This Malwarebytes program cannot report scan results through exit codes. Choose the Malwarebytes scanner option to request scans and review results in its app."
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    NoThreatsFound,
    Detected,
    Attention,
    Failed,
    Cancelled,
    TimedOut,
    ScanRequested,
}
impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoThreatsFound => "No threats found",
            Self::Detected => "Threat or scanner alert detected",
            Self::Attention => "Scan needs attention",
            Self::Failed => "Scan failed or unavailable",
            Self::Cancelled => "Scan cancelled",
            Self::TimedOut => "Scan timed out",
            Self::ScanRequested => "Scan requested — review in antivirus",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub job_id: String,
    pub path: PathBuf,
    pub scanner: String,
    pub settings_key: String,
    pub verdict: Verdict,
    pub message: String,
    pub sha256: Option<String>,
    pub finished_at: i64,
}
impl Report {
    pub fn matches_file(&self, job: &Job) -> bool {
        self.job_id == job.id
            && Some(&self.path) == job.saved_path.as_ref()
            && job
                .integrity
                .actual
                .as_ref()
                .is_none_or(|hash| self.sha256.as_ref() == Some(hash))
    }
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS antivirus_settings (id INTEGER PRIMARY KEY CHECK(id=1), data TEXT NOT NULL);
        INSERT OR IGNORE INTO antivirus_settings VALUES(1,'{}');
        CREATE TABLE IF NOT EXISTS antivirus_reports (job_id TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE, data TEXT NOT NULL);")?;
    Ok(())
}
impl Store {
    pub fn antivirus_settings(&self) -> Result<Settings> {
        let json: String = self.connect()?.query_row(
            "SELECT data FROM antivirus_settings WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        let settings: Settings = serde_json::from_str(&json)
            .context("Cannot read antivirus settings; open Settings to repair them")?;
        // Even disabled configurations must be readable, but an incomplete disabled
        // custom draft cannot run a scan until it passes full validation.
        if settings.enabled {
            settings.validate()?;
        }
        Ok(settings)
    }
    pub fn save_antivirus_settings(&self, settings: &Settings) -> Result<()> {
        if settings.enabled {
            settings.validate()?;
        }
        let count = self.connect()?.execute(
            "UPDATE antivirus_settings SET data=?1 WHERE id=1",
            [serde_json::to_string(settings)?],
        )?;
        ensure!(count == 1, "Antivirus settings are unavailable");
        Ok(())
    }
    pub fn antivirus_report(&self, id: &str) -> Result<Option<Report>> {
        let data: Option<String> = self
            .connect()?
            .query_row(
                "SELECT data FROM antivirus_reports WHERE job_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }
    fn save_antivirus_report(&self, job: &Job, settings: &Settings, report: &Report) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let data: String = tx
            .query_row("SELECT data FROM jobs WHERE id=?1", [&job.id], |r| r.get(0))
            .context("This download has been removed")?;
        let current = crate::store::decode_job(&job.id, &data)?;
        check_current(job, &current)?;
        let current: String =
            tx.query_row("SELECT data FROM antivirus_settings WHERE id=1", [], |r| {
                r.get(0)
            })?;
        ensure!(
            serde_json::from_str::<Settings>(&current)? == *settings,
            "Antivirus settings changed during the scan"
        );
        tx.execute("INSERT INTO antivirus_reports(job_id,data) VALUES(?1,?2) ON CONFLICT(job_id) DO UPDATE SET data=excluded.data", params![job.id, serde_json::to_string(report)?])?;
        tx.commit()?;
        Ok(())
    }
}

fn check_current(snapshot: &Job, current: &Job) -> Result<()> {
    ensure!(
        current.status == Status::Complete
            && current.saved_path == snapshot.saved_path
            && current.request_id == snapshot.request_id,
        "The download changed; scan its current file again"
    );
    ensure!(
        current.integrity.matches() != Some(false),
        "The file checksum does not match. Opening is blocked."
    );
    Ok(())
}

/// This capability exists only after a successful scan in this app session.
/// It retains file/parent handles until the explicit Open action completes.
pub struct OpenPermit {
    job: Job,
    settings: Settings,
    _file: File,
    _parents: Vec<File>,
}
impl OpenPermit {
    pub fn job_id(&self) -> &str {
        &self.job.id
    }
    pub fn validate(&self, store: &Store) -> Result<&Path> {
        check_current(
            &self.job,
            &store
                .get(&self.job.id)?
                .context("This download has been removed")?,
        )?;
        ensure!(
            store.antivirus_settings()? == self.settings,
            "Antivirus settings changed; scan again before opening"
        );
        let path = self
            .job
            .saved_path
            .as_deref()
            .context("Missing saved file")?;
        ensure!(
            same_file::Handle::from_file(self._file.try_clone()?)?
                == same_file::Handle::from_path(path)?,
            "The file changed after scanning; scan again"
        );
        Ok(path)
    }
    pub fn open(self, store: &Store) -> Result<()> {
        crate::paths::open_download(self.validate(store)?)
    }
}

pub struct Outcome {
    pub report: Report,
    pub permit: Option<OpenPermit>,
}

/// A user-reviewed external scan cannot prove Malwarebytes' verdict, but it must
/// still bind Open to the exact bytes whose scan was requested.
pub fn open_after_external_review(store: &Store, snapshot: &Job, report: &Report) -> Result<()> {
    external_review_permit(store, snapshot, report, &CancellationToken::new())?.open(store)
}

fn validate_external_review(
    store: &Store,
    snapshot: &Job,
    report: &Report,
) -> Result<(Job, Settings, PathBuf)> {
    let current = store
        .get(&snapshot.id)?
        .context("This download has been removed")?;
    check_current(snapshot, &current)?;
    let settings = store.antivirus_settings()?;
    ensure!(
        settings.provider.external_review() && settings.key()? == report.settings_key,
        "Scanner settings changed; request a new scan before opening"
    );
    ensure!(
        report.matches_file(&current)
            && report.verdict == Verdict::ScanRequested
            && report.job_id == snapshot.id,
        "Request an external antivirus scan of this file before reviewing it"
    );
    Ok((current, settings, report.path.clone()))
}

fn external_review_permit(
    store: &Store,
    snapshot: &Job,
    report: &Report,
    cancel: &CancellationToken,
) -> Result<OpenPermit> {
    let (_, _, path) = validate_external_review(store, snapshot, report)?;
    ensure!(!cancel.is_cancelled(), "File verification cancelled");
    let parents = process::guard_parents(&path)?;
    let (hash, file) = integrity::hash_file(&path, cancel)?;
    ensure!(
        report
            .sha256
            .as_ref()
            .is_some_and(|expected| hash.eq_ignore_ascii_case(expected)),
        "The file changed after the antivirus scan request; request a new scan"
    );

    // Hashing can take long enough for queue state or scanner settings to change.
    // Recheck both while the non-write/non-delete file and parent handles are held.
    let (current, settings, current_path) = validate_external_review(store, snapshot, report)?;
    ensure!(!cancel.is_cancelled(), "File verification cancelled");
    ensure!(current_path == path, "The saved file location changed");
    ensure!(
        same_file::Handle::from_file(file.try_clone()?)? == same_file::Handle::from_path(&path)?,
        "The file changed after the antivirus scan request; request a new scan"
    );
    Ok(OpenPermit {
        job: current,
        settings,
        _file: file,
        _parents: parents,
    })
}

fn external_review_outcome(
    store: &Store,
    snapshot: &Job,
    report: Report,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let permit = external_review_permit(store, snapshot, &report, cancel)?;
    Ok(Outcome {
        report,
        permit: Some(permit),
    })
}

pub fn scan(
    store: &Store,
    job: &Job,
    settings: &Settings,
    cancel: &CancellationToken,
    for_open: bool,
) -> Result<Outcome> {
    scan_using(store, job, settings, cancel, for_open, process::scan)
}
fn scan_using(
    store: &Store,
    job: &Job,
    settings: &Settings,
    cancel: &CancellationToken,
    for_open: bool,
    run: impl FnOnce(&Settings, &Path, &CancellationToken) -> Result<(Verdict, String)>,
) -> Result<Outcome> {
    let path = job
        .saved_path
        .clone()
        .context("The saved file location is unavailable")?;
    check_current(
        job,
        &store
            .get(&job.id)?
            .context("This download has been removed")?,
    )?;
    let mut report = Report {
        job_id: job.id.clone(),
        path: path.clone(),
        scanner: settings.provider.label().into(),
        settings_key: settings.key()?,
        verdict: Verdict::Failed,
        message: String::new(),
        sha256: None,
        finished_at: 0,
    };
    let mut permit = None;
    let result = (|| -> Result<()> {
        settings.validate()?;
        let parents = process::guard_parents(&path)?;
        let (hash, file) = integrity::hash_file(&path, cancel)?;
        report.sha256 = Some(hash);
        if let Some(expected) = &job.integrity.expected {
            ensure!(
                report
                    .sha256
                    .as_ref()
                    .is_some_and(|h| h.eq_ignore_ascii_case(expected)),
                "The current file does not match its expected checksum"
            );
        }
        let (verdict, message) = run(settings, &path, cancel)?;
        ensure!(
            !settings.provider.external_review() || verdict != Verdict::NoThreatsFound,
            "An external antivirus request cannot confirm a clean scan"
        );
        report.verdict = verdict;
        report.message = message;
        // The Windows handles deny writes/deletion throughout the scan and dialog.
        ensure!(
            same_file::Handle::from_file(file.try_clone()?)?
                == same_file::Handle::from_path(&path)?,
            "The file changed during scanning"
        );
        if verdict == Verdict::NoThreatsFound && for_open {
            permit = Some(OpenPermit {
                job: job.clone(),
                settings: settings.clone(),
                _file: file,
                _parents: parents,
            });
        }
        Ok(())
    })();
    if let Err(error) = result {
        report.verdict = Verdict::Failed;
        report.message = format!("{error:#}");
        permit = None;
    }
    if cancel.is_cancelled() {
        report.verdict = Verdict::Cancelled;
        report.message = "Scanning was cancelled. Scan again before opening.".into();
        permit = None;
    }
    report.finished_at = chrono::Utc::now().timestamp();
    store.save_antivirus_report(job, settings, &report)?;
    Ok(Outcome { report, permit })
}

#[cfg(test)]
mod tests;
