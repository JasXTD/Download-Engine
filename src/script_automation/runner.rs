use super::*;
use crate::{child_process::Running, store::Store};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{HashMap, VecDeque},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const BOOTSTRAP: &str = include_str!("bootstrap.ps1");
type Chunk = (&'static str, String);
struct Outcome {
    state: &'static str,
    error: Option<String>,
    code: Option<i32>,
}
struct Active {
    process: Option<Running>,
    output: mpsc::Receiver<Chunk>,
    readers: Vec<thread::JoinHandle<()>>,
    pending: VecDeque<Chunk>,
    deadline: Instant,
    folder: Option<PathBuf>,
    outcome: Option<Outcome>,
    /// A site reader leaves its result in the run folder.
    reader: bool,
}

pub(super) fn shell() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut buffer = vec![0u16; 32768];
        let length = unsafe {
            windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
                buffer.as_mut_ptr(),
                buffer.len() as u32,
            )
        };
        ensure!(
            length > 0 && (length as usize) < buffer.len(),
            "Cannot locate the Windows system directory"
        );
        let path = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length as usize]))
            .join("WindowsPowerShell/v1.0/powershell.exe");
        ensure!(path.is_file(), "Windows PowerShell 5.1 is unavailable");
        Ok(path)
    }
    #[cfg(not(windows))]
    anyhow::bail!("Script automation requires Windows PowerShell")
}

fn read_output(
    mut pipe: impl Read + Send + 'static,
    level: &'static str,
    tx: mpsc::Sender<Chunk>,
    budget: Arc<AtomicUsize>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        let mut pending = Vec::new();
        loop {
            let count = match pipe.read(&mut buffer) {
                Ok(n) => n,
                Err(e) => {
                    let _ = tx.send(("error", format!("Cannot read script output: {e}")));
                    break;
                }
            };
            if count == 0 {
                if !pending.is_empty() {
                    let _ = tx.send((level, String::from_utf8_lossy(&pending).into_owned()));
                }
                break;
            }
            // Keep draining a noisy process after the retained output limit, without
            // growing the channel or letting a full stdout pipe freeze the script.
            let previous = budget
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                    Some(n.saturating_add(count))
                })
                .unwrap();
            if previous > MAX_LOG_BYTES + 8192 {
                continue;
            }
            pending.extend_from_slice(&buffer[..count]);
            let end = match std::str::from_utf8(&pending) {
                Err(e) if e.error_len().is_none() => e.valid_up_to(),
                _ => pending.len(),
            };
            if end > 0 {
                let text = String::from_utf8_lossy(&pending[..end]).into_owned();
                if tx.send((level, text)).is_err() {
                    break;
                }
                pending.drain(..end);
            }
        }
    })
}

fn cleanup(folder: &Path) -> Result<()> {
    if !folder.exists() {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(folder)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "Refusing to clean a linked script runtime directory"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "Refusing to clean a redirected script runtime directory"
        );
    }
    for name in [
        "script.ps1",
        "script.lua",
        "bootstrap.ps1",
        "event.json",
        "result.json",
    ] {
        match std::fs::remove_file(folder.join(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    std::fs::remove_dir(folder).context("Cannot remove the script runtime directory")
}

impl Active {
    fn start(store: &Store, claim: &Claim) -> Result<Self> {
        let runtime = store.paths.data.join("script-runtime");
        std::fs::create_dir_all(&runtime)?;
        crate::model::validate_job_id(&claim.run.id)?;
        let folder = runtime.join(&claim.run.id);
        std::fs::create_dir(&folder)?;
        let result = (|| -> Result<Self> {
            std::fs::write(
                folder.join("event.json"),
                serde_json::to_vec(&claim.run.context)?,
            )?;
            let cli = std::env::current_exe()?
                .parent()
                .context("Application directory unavailable")?
                .join("de-cli.exe");
            // Extension jobs get their settings and library code over a pipe, so a
            // saved secret never sits in the run folder. A job must not start
            // without its limits, so a failed read stops the run.
            let reach = if claim.run.trigger == "site_reader" {
                crate::extensions::Reach::Web
            } else {
                crate::extensions::Reach::Automation
            };
            let extension = store.extension_context(&store.script(&claim.run.script_id)?, reach)?;
            let mut command = match claim.run.language {
                Language::Lua => {
                    std::fs::write(folder.join("script.lua"), &claim.body)?;
                    let mut command = Command::new(&cli);
                    command
                        .arg("--lua-worker")
                        .env("XDOWNLOADER_SCRIPT_FILE", folder.join("script.lua"))
                        .env("XDOWNLOADER_RESULT_FILE", folder.join("result.json"));
                    command
                }
                Language::PowerShell => {
                    // BOM makes non-ASCII source unambiguous to Windows PowerShell 5.1.
                    std::fs::write(folder.join("script.ps1"), format!("\u{feff}{}", claim.body))?;
                    std::fs::write(folder.join("bootstrap.ps1"), format!("\u{feff}{BOOTSTRAP}"))?;
                    let mut command = Command::new(shell()?);
                    command
                        .args([
                            "-NoLogo",
                            "-NoProfile",
                            "-NonInteractive",
                            "-ExecutionPolicy",
                            "Bypass",
                            "-File",
                        ])
                        .arg(folder.join("bootstrap.ps1"))
                        .env("XDOWNLOADER_SCRIPT_FILE", folder.join("script.ps1"));
                    command
                }
            };
            command
                .current_dir(&store.paths.downloads)
                .env("XDOWNLOADER_CLI", cli)
                .env("XDOWNLOADER_EVENT_FILE", folder.join("event.json"))
                .env(
                    "IDM_EX_SCRIPT_PROFILE",
                    serde_json::to_string(&store.paths)?,
                )
                .env("IDM_EX_SCRIPT_RUN", &claim.run.id)
                .stdin(if extension.is_some() {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if extension.is_some() {
                command.env("XDOWNLOADER_EXTENSION", "1");
            }
            // Include native parsing/conversion allocations in the worker budget.
            // Lua calls the CLI in-process and does not need child processes.
            let memory_limit = (claim.run.language == Language::Lua).then_some(256 * 1024 * 1024);
            let mut process =
                Running::spawn(&mut command, &CancellationToken::new(), memory_limit)?;
            let (tx, output) = mpsc::channel();
            let budget = Arc::new(AtomicUsize::new(0));
            let stdout = process
                .child
                .stdout
                .take()
                .context("Missing script stdout")?;
            let stderr = process
                .child
                .stderr
                .take()
                .context("Missing script stderr")?;
            let mut readers = vec![
                read_output(stdout, "info", tx.clone(), budget.clone()),
                read_output(stderr, "error", tx, budget),
            ];
            if let Some(extension) = &extension {
                use std::io::Write;
                let mut pipe = process.child.stdin.take().context("Missing script stdin")?;
                let bytes = zeroize::Zeroizing::new(serde_json::to_vec(extension)?);
                // A full pipe must not stall the scheduler. Closing it ends the input.
                readers.push(thread::spawn(move || {
                    let _ = pipe.write_all(&bytes);
                }));
            }
            Ok(Self {
                process: Some(process),
                output,
                readers,
                pending: VecDeque::new(),
                deadline: Instant::now() + Duration::from_secs(u64::from(claim.timeout)),
                folder: Some(folder.clone()),
                outcome: None,
                reader: claim.run.trigger == "site_reader",
            })
        })();
        if result.is_err() {
            cleanup(&folder)?;
        }
        result
    }
    fn failed(error: anyhow::Error) -> Self {
        let (_, output) = mpsc::channel();
        let message = format!("Cannot start script: {error:#}");
        Self {
            process: None,
            output,
            readers: Vec::new(),
            pending: VecDeque::from([("error", message.clone())]),
            deadline: Instant::now(),
            folder: None,
            outcome: Some(Outcome {
                state: "failed",
                error: Some(message),
                code: None,
            }),
            reader: false,
        }
    }
    fn stop(&mut self, outcome: Outcome) {
        self.outcome = Some(outcome);
        // Kill the whole Job before joining pipe readers; children may hold pipes open.
        drop(self.process.take());
        for reader in self.readers.drain(..) {
            if reader.join().is_err() {
                self.pending
                    .push_back(("error", "An output reader stopped unexpectedly".into()));
            }
        }
    }
    fn poll(&mut self, store: &Store, id: &str) -> Result<bool> {
        if self.outcome.is_none() {
            if store.script_stop_requested(id)? {
                self.stop(Outcome {
                    state: "stopped",
                    error: None,
                    code: None,
                });
            } else if let Some(process) = &mut self.process {
                match process.child.try_wait() {
                    Ok(Some(status)) => self.stop(Outcome {
                        state: if status.success() {
                            "succeeded"
                        } else {
                            "failed"
                        },
                        error: if status.success() {
                            None
                        } else {
                            Some(format!(
                                "Script exited with {}",
                                status
                                    .code()
                                    .map_or("no exit code".into(), |n| n.to_string())
                            ))
                        },
                        code: status.code(),
                    }),
                    Ok(None) if Instant::now() >= self.deadline => self.stop(Outcome {
                        state: "timed_out",
                        error: Some("Script exceeded its time limit".into()),
                        code: None,
                    }),
                    Ok(None) => {}
                    Err(e) => self.stop(Outcome {
                        state: "failed",
                        error: Some(format!("Cannot monitor script: {e}")),
                        code: None,
                    }),
                }
            }
        }
        self.pending.extend(self.output.try_iter());
        // Keep a successful reader's result before its folder is cleaned.
        if self.reader
            && let (Some(outcome), Some(folder)) = (&mut self.outcome, &self.folder)
            && outcome.state == "succeeded"
            && let Err(error) = keep_result(store, id, folder)
        {
            let message = format!("{error:#}");
            self.pending.push_back(("error", format!("{message}\n")));
            *outcome = Outcome {
                state: "failed",
                error: Some(message),
                code: outcome.code,
            };
        }
        if self.outcome.is_some()
            && let Some(folder) = self.folder.take()
            && let Err(error) = cleanup(&folder)
        {
            self.pending.push_back((
                "warn",
                format!(
                    "Could not clean temporary script files: {error:#}. They remain at {}.\n",
                    folder.display()
                ),
            ));
        }
        while let Some((level, text)) = self.pending.front() {
            store.append_script_log(id, level, text)?;
            self.pending.pop_front();
        }
        if let Some(outcome) = &self.outcome {
            store.finish_script(id, outcome.state, outcome.error.clone(), outcome.code)?;
            return Ok(true);
        }
        Ok(false)
    }
}
pub(super) fn keep_result(store: &Store, id: &str, folder: &Path) -> Result<()> {
    let mut text = String::new();
    std::fs::File::open(folder.join("result.json"))
        .context("The site reader finished without a result")?
        .take(super::reading::MAX_READING_BYTES as u64 + 1)
        .read_to_string(&mut text)?;
    ensure!(
        text.len() <= super::reading::MAX_READING_BYTES,
        "The site reader's result exceeds 1 MiB"
    );
    store.save_script_result(id, &serde_json::from_str(&text)?)
}

impl Drop for Active {
    fn drop(&mut self) {
        drop(self.process.take());
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        // An abnormal shutdown is recovered durably on the next scheduler start.
    }
}

pub(crate) struct Controller {
    active: HashMap<String, Active>,
    startup: bool,
}
impl Default for Controller {
    fn default() -> Self {
        Self {
            active: HashMap::new(),
            startup: true,
        }
    }
}
impl Controller {
    pub(crate) fn tick(&mut self, store: &Store, now: i64) -> Result<()> {
        if self.startup {
            let root = store.paths.data.join("script-runtime");
            if root.exists() {
                let retained = store.script_runs(None)?;
                for item in std::fs::read_dir(root)? {
                    let item = item?;
                    if item
                        .file_name()
                        .to_str()
                        .is_some_and(|n| crate::model::validate_job_id(n).is_ok())
                        && let Err(error) = cleanup(&item.path())
                    {
                        let message = format!(
                            "Could not clean temporary script files: {error:#}. Left {} for inspection.\n",
                            item.path().display()
                        );
                        if let Some(run) = retained
                            .iter()
                            .find(|r| item.file_name() == std::ffi::OsStr::new(&r.id))
                        {
                            store.append_script_log(&run.id, "warn", &message)?;
                        } else {
                            crate::diagnostics::message(
                                crate::diagnostics::Event::SchedulerFailed,
                                &message,
                            );
                        }
                    }
                }
            }
        }
        let mut done = Vec::new();
        let mut error = None;
        for (id, active) in &mut self.active {
            match active.poll(store, id) {
                Ok(true) => done.push(id.clone()),
                Ok(false) => {}
                Err(e) => error = Some(e),
            }
        }
        for id in done {
            self.active.remove(&id);
        }
        if let Some(error) = error {
            return Err(error);
        }
        store.schedule_scripts(now, self.startup)?;
        self.startup = false;
        // One slot more than MAX_ACTIVE is kept for site readers, which a person
        // is waiting on, so busy automation scripts never hold them up.
        while self.active.len() <= MAX_ACTIVE {
            let Some(claim) = store.claim_script(self.active.len() == MAX_ACTIVE)? else {
                break;
            };
            match Active::start(store, &claim) {
                Ok(active) => {
                    self.active.insert(claim.run.id.clone(), active);
                    if claim.run.context["events_expired"].as_bool() == Some(true) {
                        store.append_script_log(&claim.run.id,"warn","Some download events expired while this script was busy. Only retained events are included.\n")?;
                    }
                }
                Err(error) => {
                    // Retain the terminal outcome until SQLite accepts it, just
                    // like a completed process. A transient write error must
                    // not strand a claimed run with no process to monitor.
                    self.active
                        .insert(claim.run.id.clone(), Active::failed(error));
                }
            }
        }
        Ok(())
    }
    pub(crate) fn shutdown(&mut self, store: &Store) -> Result<()> {
        // Stop every process even if an individual database/log write fails.
        let mut error = None;
        for (id, active) in &mut self.active {
            active.stop(Outcome {
                state: "interrupted",
                error: Some("App closed during this run".into()),
                code: None,
            });
            if let Err(e) = active.poll(store, id) {
                error = Some(e);
            }
        }
        self.active.clear();
        for run in store.script_runs(None)?.into_iter().filter(Run::active) {
            if let Err(e) = store.finish_script(
                &run.id,
                "interrupted",
                Some("App closed before this run finished".into()),
                None,
            ) {
                error = Some(e);
            }
        }
        if let Some(error) = error {
            return Err(error);
        }
        Ok(())
    }
}
