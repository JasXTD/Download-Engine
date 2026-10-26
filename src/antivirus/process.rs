use super::{Provider, Settings, Verdict, validate_executable};
use crate::child_process::{Running, reader};
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub(super) use crate::download_file::{guard_parents, validate_path};

#[cfg(windows)]
pub(super) fn known_folder(id: &windows_core::GUID) -> Result<PathBuf> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    };
    // The shell allocates this terminated string; free it on both conversion outcomes.
    unsafe {
        let value = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)?;
        let path = value.to_string();
        CoTaskMemFree(Some(value.0.cast()));
        Ok(PathBuf::from(path?))
    }
}
pub fn defender_path() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::{FOLDERID_ProgramData, FOLDERID_ProgramFiles};
        let root = known_folder(&FOLDERID_ProgramData)?.join("Microsoft/Windows Defender/Platform");
        let mut versions = Vec::new();
        if root.is_dir() {
            for entry in std::fs::read_dir(root)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.is_empty()
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_digit() || b == b'.' || b == b'-')
                {
                    let version: Option<Vec<u32>> =
                        name.split(['.', '-']).map(|n| n.parse().ok()).collect();
                    let candidate = entry.path().join("MpCmdRun.exe");
                    if let Some(version) = version
                        && candidate.is_file()
                    {
                        versions.push((version, candidate));
                    }
                }
            }
        }
        versions.sort_by(|a, b| b.0.cmp(&a.0));
        let fallback = known_folder(&FOLDERID_ProgramFiles)?.join("Windows Defender/MpCmdRun.exe");
        if let Some((_, path)) = versions.into_iter().next() {
            return Ok(path);
        }
        ensure!(
            fallback.is_file(),
            "Microsoft Defender's scanner is not installed or available"
        );
        Ok(fallback)
    }
    #[cfg(not(windows))]
    anyhow::bail!("Microsoft Defender scanning is supported on Windows")
}
pub fn malwarebytes_path() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let path = known_folder(&windows::Win32::UI::Shell::FOLDERID_ProgramFiles)?
            .join("Malwarebytes/Anti-Malware/Malwarebytes.exe");
        ensure!(
            path.is_file(),
            "Malwarebytes was not found in Program Files. Open it from the Start menu."
        );
        Ok(path)
    }
    #[cfg(not(windows))]
    anyhow::bail!("This Malwarebytes shortcut is supported on Windows")
}
fn arguments(settings: &Settings, file: &Path) -> Result<Vec<String>> {
    let file = file
        .to_str()
        .context("The scanner requires a Unicode file path")?;
    Ok(match settings.provider {
        Provider::Defender => vec![
            "-Scan".into(),
            "-ScanType".into(),
            "3".into(),
            "-File".into(),
            file.into(),
            "-DisableRemediation".into(),
        ],
        Provider::ClamAv => vec![
            "--stdout".into(),
            "--alert-exceeds-max=yes".into(),
            "--alert-encrypted=yes".into(),
            "--max-scantime=0".into(),
            "--".into(),
            file.into(),
        ],
        Provider::Custom => settings
            .arguments
            .iter()
            .map(|arg| arg.replace("{file}", file))
            .collect(),
        Provider::Malwarebytes | Provider::Kaspersky | Provider::Bitdefender => {
            anyhow::bail!("This antivirus uses its Explorer scan action")
        }
    })
}

pub(super) fn scan(
    settings: &Settings,
    file: &Path,
    cancel: &CancellationToken,
) -> Result<(Verdict, String)> {
    if settings.provider.external_review() {
        return super::external::request(
            settings.provider,
            file,
            cancel,
            Duration::from_secs(settings.timeout_seconds),
        );
    }
    let exe = if settings.provider == Provider::Defender {
        defender_path()?
    } else {
        PathBuf::from(&settings.executable)
    };
    validate_executable(&exe)?;
    ensure!(
        !same_file::is_same_file(&exe, file)?,
        "The downloaded file cannot be used as its own scanner"
    );
    let _folders = guard_parents(&exe)?;
    let _program = crate::download_file::read_completed(&exe)
        .context("The antivirus executable is unavailable")?;
    let mut command = Command::new(&exe);
    command
        .args(arguments(settings, file)?)
        .current_dir(exe.parent().context("Missing scanner folder")?)
        .env("LC_ALL", "C");
    let output = run(
        &mut command,
        cancel,
        Duration::from_secs(settings.timeout_seconds),
    )?;
    Ok(classify(settings, output))
}

struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    truncated: bool,
    stopped: Option<Verdict>,
}
fn classify(settings: &Settings, output: Output) -> (Verdict, String) {
    if let Some(stopped) = output.stopped {
        return (stopped, "Scan again before opening this file.".into());
    }
    let verdict = match (settings.provider, output.code) {
        (_, _) if output.truncated => Verdict::Attention,
        (Provider::Defender, Some(0)) => Verdict::NoThreatsFound,
        // Microsoft's exit 2 can mean a detection OR a scan error. Never guess.
        (Provider::Defender, Some(2)) => Verdict::Attention,
        (Provider::ClamAv, Some(0))
            if output.stdout.lines().any(|line| {
                line.trim()
                    .strip_prefix("Scanned files:")
                    .is_some_and(|n| n.trim() == "1")
            }) =>
        {
            Verdict::NoThreatsFound
        }
        (Provider::ClamAv, Some(0)) => Verdict::Attention,
        (Provider::ClamAv, Some(1)) => Verdict::Detected,
        (Provider::Custom, Some(code)) if Some(code) == settings.clean_exit => {
            Verdict::NoThreatsFound
        }
        (Provider::Custom, Some(code)) if Some(code) == settings.detected_exit => Verdict::Detected,
        _ => Verdict::Failed,
    };
    let summary = match verdict {
        Verdict::NoThreatsFound => {
            "The scanner reported no threats in this scan. This does not guarantee that a file is safe."
        }
        Verdict::Detected => {
            "The scanner reported a threat or an alert. Opening is blocked. Review the file in your antivirus app."
        }
        Verdict::Attention if settings.provider == Provider::Defender => {
            "Defender reported a detection, required action, or a scan error. Opening is blocked. Review Windows Security and scan again."
        }
        Verdict::Attention => {
            "The scanner did not confirm a complete single-file scan. Opening is blocked."
        }
        _ => {
            "The scanner could not finish successfully. Check that it is active, licensed if required, and up to date. Opening is blocked."
        }
    };
    let mut message = format!(
        "{summary}\nExit code: {}",
        output
            .code
            .map(|n| n.to_string())
            .unwrap_or_else(|| "unavailable".into())
    );
    if verdict != Verdict::NoThreatsFound {
        let details: String = format!("{}\n{}", output.stdout, output.stderr)
            .chars()
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
            .take(3000)
            .collect();
        if !details.trim().is_empty() {
            message.push('\n');
            message.push_str(details.trim());
        }
    }
    (verdict, message)
}

fn run(command: &mut Command, cancel: &CancellationToken, timeout: Duration) -> Result<Output> {
    ensure!(!cancel.is_cancelled(), "Scan cancelled");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut running = Running::spawn(command, cancel, None)?;
    let stdout = reader(
        running
            .child
            .stdout
            .take()
            .context("Missing scanner output")?,
    );
    let stderr = reader(
        running
            .child
            .stderr
            .take()
            .context("Missing scanner error output")?,
    );
    let started = Instant::now();
    let (code, stopped) = loop {
        if cancel.is_cancelled() {
            break (None, Some(Verdict::Cancelled));
        }
        if started.elapsed() >= timeout {
            break (None, Some(Verdict::TimedOut));
        }
        if let Some(status) = running.child.try_wait()? {
            break (status.code(), None);
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    drop(running);
    let (stdout, out_truncated) = stdout
        .recv_timeout(Duration::from_secs(2))
        .context("Scanner output did not close")??;
    let (stderr, err_truncated) = stderr
        .recv_timeout(Duration::from_secs(2))
        .context("Scanner error output did not close")??;
    Ok(Output {
        code,
        stdout,
        stderr,
        truncated: out_truncated || err_truncated,
        stopped,
    })
}

pub(super) fn request_external(
    provider: Provider,
    command: &mut Command,
    cancel: &CancellationToken,
    timeout: Duration,
) -> Result<(Verdict, String)> {
    let output = run(command, cancel, timeout)?;
    external_request_outcome(provider, output)
}
fn external_request_outcome(provider: Provider, output: Output) -> Result<(Verdict, String)> {
    if let Some(stopped) = output.stopped {
        return Ok((
            stopped,
            format!(
                "The scan request stopped. {} may already be scanning; review or cancel it in its app.",
                provider.name()
            ),
        ));
    }
    ensure!(
        output.code == Some(0)
            && !output.truncated
            && output.stdout.trim()
                == format!(
                    "IDM_EX_AV_REQUESTED_V1:{}",
                    provider
                        .helper_id()
                        .context("Unsupported external scanner")?
                ),
        "Could not confirm the {} scan request. Check its app before retrying. {}",
        provider.name(),
        output
            .stderr
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(2000)
            .collect::<String>()
    );
    Ok((
        Verdict::ScanRequested,
        format!(
            "A scan was requested through {}'s Explorer action. Check that it completed and review its result in that app. Download Engine has not verified a clean scan. Cancelling or closing Download Engine does not stop the external scan.",
            provider.name()
        ),
    ))
}

#[cfg(test)]
mod tests;
