//! Parse editor buffers without running them. One cancellable worker owns the
//! latest request, so typing never queues an unbounded series of PowerShells.
use super::{Language, MAX_SCRIPT_BYTES};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    io::Write,
    process::{Command, Stdio},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Diagnostic {
    pub line: usize,
    /// One-based Unicode character column. Lua's parser supplies only a line.
    pub column: Option<usize>,
    pub message: String,
}

pub struct Reply {
    pub revision: u64,
    pub result: Result<Vec<Diagnostic>>,
}
struct Request {
    revision: u64,
    language: Language,
    source: String,
    cancel: CancellationToken,
}
#[derive(Default)]
struct Pending {
    request: Option<Request>,
    closed: bool,
}

pub struct Checker {
    pending: Arc<(Mutex<Pending>, Condvar)>,
    replies: mpsc::Receiver<Reply>,
    cancel: CancellationToken,
}
impl Checker {
    pub fn new() -> Result<Self> {
        let pending = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let worker = pending.clone();
        let (tx, replies) = mpsc::channel();
        thread::Builder::new()
            .name("script-syntax".into())
            .spawn(move || {
                loop {
                    let (lock, wake) = &*worker;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    while state.request.is_none() && !state.closed {
                        state = wake.wait(state).unwrap_or_else(|e| e.into_inner());
                    }
                    if state.closed {
                        break;
                    }
                    let request = state.request.take().expect("pending syntax request");
                    drop(state);
                    if request.cancel.is_cancelled() {
                        continue;
                    }
                    let result = check(request.language, &request.source, &request.cancel);
                    if !request.cancel.is_cancelled()
                        && tx
                            .send(Reply {
                                revision: request.revision,
                                result,
                            })
                            .is_err()
                    {
                        break;
                    }
                }
            })
            .context("Cannot start syntax checker")?;
        Ok(Self {
            pending,
            replies,
            cancel: CancellationToken::new(),
        })
    }

    /// Cancel immediately on an edit, before the UI's debounce period ends.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn submit(&mut self, revision: u64, language: Language, source: String) {
        self.cancel.cancel();
        self.cancel = CancellationToken::new();
        let (lock, wake) = &*self.pending;
        lock.lock().unwrap_or_else(|e| e.into_inner()).request = Some(Request {
            revision,
            language,
            source,
            cancel: self.cancel.clone(),
        });
        wake.notify_one();
    }

    pub fn try_recv(&self) -> Option<Reply> {
        self.replies.try_recv().ok()
    }
}
impl Drop for Checker {
    fn drop(&mut self) {
        self.cancel.cancel();
        let (lock, wake) = &*self.pending;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        state.request = None;
        wake.notify_one();
        // The worker observes cancellation and drops its owned process. Do not
        // join on the UI thread while Windows is closing the helper's handles.
    }
}

pub fn check(
    language: Language,
    source: &str,
    cancel: &CancellationToken,
) -> Result<Vec<Diagnostic>> {
    ensure!(source.len() <= MAX_SCRIPT_BYTES, "Script exceeds 64 KiB");
    ensure!(!cancel.is_cancelled(), "Syntax check cancelled");
    match language {
        Language::Lua => {
            let lua = mlua::Lua::new_with(mlua::StdLib::NONE, Default::default())?;
            lua.set_memory_limit(16 * 1024 * 1024)?;
            match lua
                .load(source)
                .set_name("@editor")
                .set_mode(mlua::chunk::ChunkMode::Text)
                .into_function()
            {
                Ok(_) => Ok(vec![]),
                Err(mlua::Error::SyntaxError { message, .. }) => {
                    let line = message
                        .strip_prefix("editor:")
                        .and_then(|s| s.split(':').next())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1);
                    Ok(vec![Diagnostic {
                        line,
                        column: None,
                        message,
                    }])
                }
                Err(error) => Err(error.into()),
            }
        }
        Language::PowerShell => powershell(source, cancel),
    }
}

// Only this fixed program executes. Editor text arrives as UTF-8 data on stdin,
// and ParseInput creates an AST; it does not invoke commands or load profiles.
const PARSER: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$source = [Console]::In.ReadToEnd()
$tokens = $null; $errors = $null
$null = [Management.Automation.Language.Parser]::ParseInput($source, [ref]$tokens, [ref]$errors)
$items = @($errors | Select-Object -First 8 | ForEach-Object {
    @{ line = $_.Extent.StartLineNumber; column = $_.Extent.StartColumnNumber;
       message = $_.Message.Substring(0, [Math]::Min(512, $_.Message.Length)) }
})
[Console]::Out.Write((ConvertTo-Json -InputObject @{ diagnostics = $items } -Compress -Depth 4))
"#;

fn powershell(source: &str, cancel: &CancellationToken) -> Result<Vec<Diagnostic>> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(
        PARSER
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut command = Command::new(super::runner::shell()?);
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &encoded,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut process =
        crate::child_process::Running::spawn(&mut command, cancel, Some(256 * 1024 * 1024))?;
    let mut input = process
        .child
        .stdin
        .take()
        .context("Syntax checker stdin unavailable")?;
    let source_bytes = source.as_bytes().to_vec();
    let (written, writing) = mpsc::channel();
    thread::Builder::new()
        .name("syntax-input".into())
        .spawn(move || {
            let result = input.write_all(&source_bytes);
            drop(input);
            let _ = written.send(result);
        })?;
    let output = crate::child_process::reader(
        process
            .child
            .stdout
            .take()
            .context("Syntax checker stdout unavailable")?,
    );
    let errors = crate::child_process::reader(
        process
            .child
            .stderr
            .take()
            .context("Syntax checker stderr unavailable")?,
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        ensure!(!cancel.is_cancelled(), "Syntax check cancelled");
        ensure!(
            Instant::now() < deadline,
            "PowerShell syntax check timed out"
        );
        if let Some(status) = process.child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(20));
    };
    // No descendant may retain a pipe after the parser exits.
    drop(process);
    let (out, truncated) = output.recv_timeout(Duration::from_secs(1))??;
    let (err, _) = errors.recv_timeout(Duration::from_secs(1))??;
    ensure!(
        status.success(),
        "PowerShell syntax checker failed: {}",
        err.trim()
    );
    writing.recv_timeout(Duration::from_secs(1))??;
    ensure!(!truncated, "Syntax checker output exceeded its limit");
    #[derive(Deserialize)]
    struct Parsed {
        diagnostics: Vec<Diagnostic>,
    }
    let mut parsed: Parsed = serde_json::from_str(out.trim_start_matches('\u{feff}'))
        .context("Invalid syntax checker response")?;
    for diagnostic in &mut parsed.diagnostics {
        // PowerShell's columns count UTF-16 units; egui counts Unicode characters.
        if let Some(column) = diagnostic.column {
            let line = source
                .split('\n')
                .nth(diagnostic.line.saturating_sub(1))
                .unwrap_or("");
            let mut units = 0;
            let chars = line
                .chars()
                .take_while(|c| {
                    if units >= column.saturating_sub(1) {
                        return false;
                    }
                    units += c.len_utf16();
                    true
                })
                .count();
            diagnostic.column = Some(chars + 1);
        }
    }
    Ok(parsed.diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lua_parses_without_executing_and_reports_lines() {
        let cancel = CancellationToken::new();
        assert!(
            check(
                Language::Lua,
                "error('must not execute')\nwhile true do end",
                &cancel
            )
            .unwrap()
            .is_empty()
        );
        let errors = check(
            Language::Lua,
            "-- valid Unicode: 中文 🦀\nlocal value = )",
            &cancel,
        )
        .unwrap();
        assert_eq!(errors[0].line, 2);
        assert!(!errors[0].message.is_empty());
        assert!(check(Language::Lua, "local =", &cancel).unwrap().len() == 1);
        assert!(check(Language::Lua, &"a".repeat(MAX_SCRIPT_BYTES + 1), &cancel).is_err());
        cancel.cancel();
        assert!(check(Language::Lua, "", &cancel).is_err());
    }
    #[test]
    #[cfg(windows)]
    fn powershell_parses_without_side_effects_and_uses_unicode_columns() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("must-not-exist");
        let source = format!(
            "Set-Content -LiteralPath '{}' -Value touched\nwhile ($true) {{}}",
            marker.display()
        );
        let cancel = CancellationToken::new();
        assert!(
            check(Language::PowerShell, &source, &cancel)
                .unwrap()
                .is_empty()
        );
        assert!(!marker.exists());
        let source = "# 中文\n'🦀'; )";
        let errors = check(Language::PowerShell, source, &cancel).unwrap();
        assert_eq!(errors[0].line, 2);
        assert_eq!(errors[0].column, Some(6));
    }
    #[test]
    fn worker_keeps_the_newest_request() {
        let mut worker = Checker::new().unwrap();
        for revision in 1..=40 {
            worker.submit(revision, Language::Lua, "local =".into());
        }
        worker.submit(41, Language::Lua, "print('valid')".into());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(reply) = worker.try_recv()
                && reply.revision == 41
            {
                assert!(reply.result.unwrap().is_empty());
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[cfg(windows)]
    fn superseding_powershell_cancels_its_owned_parser() {
        let mut worker = Checker::new().unwrap();
        worker.submit(1, Language::PowerShell, "$value = 1\n".repeat(4000));
        thread::sleep(Duration::from_millis(60));
        let started = Instant::now();
        worker.submit(2, Language::Lua, "print('latest')".into());
        loop {
            if let Some(reply) = worker.try_recv()
                && reply.revision == 2
            {
                assert!(reply.result.unwrap().is_empty());
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "cancelled parser held up the next request"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
