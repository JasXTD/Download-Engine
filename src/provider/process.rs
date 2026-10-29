//! One bounded request in flight. Dropping a worker kills its owned process before joining I/O.
use super::*;
use crate::{child_process::Running, paths::AppPaths};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const FRAME_LIMIT: usize = 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Resolver,
    Processor,
}
pub(crate) fn read_frame(reader: &mut impl Read) -> Result<serde_json::Value> {
    let mut size = [0; 4];
    reader.read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    ensure!(
        size > 0 && size <= FRAME_LIMIT,
        "Provider message exceeds 1 MiB"
    );
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub(crate) fn write_frame(writer: &mut impl Write, value: &serde_json::Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= FRAME_LIMIT, "Provider message exceeds 1 MiB");
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

pub(crate) struct Worker {
    process: Option<Running>,
    requests: Option<mpsc::SyncSender<serde_json::Value>>,
    replies: mpsc::Receiver<Result<serde_json::Value>>,
    io: Option<thread::JoinHandle<()>>,
    timeout: Duration,
    role: Role,
}
impl Drop for Worker {
    fn drop(&mut self) {
        drop(self.process.take());
        drop(self.requests.take());
        if let Some(io) = self.io.take() {
            let _ = io.join();
        }
    }
}
impl Worker {
    pub(crate) fn start(
        paths: &AppPaths,
        script: &Script,
        role: Role,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        let cli = std::env::current_exe()?
            .parent()
            .context("Missing application directory")?
            .join("de-cli.exe");
        Self::start_at(&cli, paths, script, role, cancel)
    }
    pub(crate) fn start_at(
        cli: &std::path::Path,
        paths: &AppPaths,
        script: &Script,
        role: Role,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        script.validate()?;
        let mut command = Command::new(cli);
        command
            .arg("--provider-worker")
            .env("IDM_EX_PROVIDER_PROFILE", serde_json::to_string(paths)?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut process = Running::spawn(&mut command, cancel, Some(256 * 1024 * 1024))?;
        let mut input = process
            .child
            .stdin
            .take()
            .context("Missing provider stdin")?;
        let mut output = process
            .child
            .stdout
            .take()
            .context("Missing provider stdout")?;
        let (tx, requests) = mpsc::sync_channel::<serde_json::Value>(1);
        let (replies, rx) = mpsc::sync_channel(1);
        let io = thread::spawn(move || {
            while let Ok(request) = requests.recv() {
                let result =
                    write_frame(&mut input, &request).and_then(|()| read_frame(&mut output));
                let failed = result.is_err();
                if replies.send(result).is_err() || failed {
                    break;
                }
            }
        });
        let mut worker = Self {
            process: Some(process),
            requests: Some(tx),
            replies: rx,
            io: Some(io),
            timeout: Duration::from_secs(script.timeout_seconds.min(MAX_CALLBACK_SECONDS).into()),
            role,
        };
        worker.call(
            serde_json::json!({"op":"init","source":script.body,"role":role,"script_id":script.id}),
            cancel,
        )?;
        Ok(worker)
    }
    fn call(
        &mut self,
        request: serde_json::Value,
        cancel: &CancellationToken,
    ) -> Result<serde_json::Value> {
        ensure!(!cancel.is_cancelled(), "Provider cancelled");
        self.requests
            .as_ref()
            .context("Provider stopped")?
            .send(request)?;
        let deadline = Instant::now() + self.timeout;
        loop {
            ensure!(!cancel.is_cancelled(), "Provider cancelled");
            ensure!(Instant::now() < deadline, "Provider callback timed out");
            match self.replies.recv_timeout(Duration::from_millis(25)) {
                Ok(result) => {
                    let reply = result.context("Provider worker stopped unexpectedly")?;
                    if let Some(error) = reply.get("error").and_then(|v| v.as_str()) {
                        anyhow::bail!("Provider: {error}");
                    }
                    return reply
                        .get("value")
                        .cloned()
                        .context("Invalid provider reply");
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => return Err(error).context("Provider worker stopped unexpectedly"),
            }
        }
    }
    pub(crate) fn resolve(
        &mut self,
        url: &str,
        previous: Option<&Content>,
        cancel: &CancellationToken,
    ) -> Result<Plan> {
        ensure!(
            self.role == Role::Resolver,
            "Processor workers cannot resolve or refresh"
        );
        let value = self.call(
            serde_json::json!({"op":"resolve","url":url,"previous":previous}),
            cancel,
        )?;
        let plan: Plan = serde_json::from_value(value)?;
        plan.validate()?;
        Ok(plan)
    }
    pub(crate) fn transform(
        &mut self,
        plan: &Content,
        bytes: &[u8],
        offset: u64,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        ensure!(
            self.role == Role::Processor,
            "Resolver workers cannot process file bytes"
        );
        ensure!(bytes.len() <= CHUNK_BYTES, "Provider chunk exceeds 256 KiB");
        let value = self.call(serde_json::json!({"op":"transform","plan":plan,"bytes":STANDARD.encode(bytes),"offset":offset}), cancel)?;
        let value = value.as_str().context("Invalid provider chunk")?;
        ensure!(
            value.len() <= CHUNK_BYTES.div_ceil(3) * 4,
            "Provider output exceeds 256 KiB"
        );
        let output = STANDARD.decode(value)?;
        ensure!(
            output.len() == bytes.len(),
            "Provider must return exactly one output byte per input byte"
        );
        Ok(output)
    }
    pub(crate) fn finish(&mut self, plan: &Content, cancel: &CancellationToken) -> Result<()> {
        ensure!(
            self.role == Role::Processor,
            "Resolver workers cannot finish files"
        );
        let value = self.call(serde_json::json!({"op":"finish","plan":plan}), cancel)?;
        ensure!(value == true, "Provider finish must return true");
        Ok(())
    }
}
