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

const FRAME_LIMIT: usize = 6 * 1024 * 1024;
/// The first frame can hold a whole extension, and escaping can double its size.
const INIT_FRAME_LIMIT: usize = 6 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Resolver,
    Processor,
}
pub(crate) fn read_frame(reader: &mut impl Read) -> Result<serde_json::Value> {
    read_limited(reader, FRAME_LIMIT)
}
pub(crate) fn read_init_frame(reader: &mut impl Read) -> Result<serde_json::Value> {
    read_limited(reader, INIT_FRAME_LIMIT)
}
fn read_limited(reader: &mut impl Read, limit: usize) -> Result<serde_json::Value> {
    let mut size = [0; 4];
    reader.read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    ensure!(
        size > 0 && size <= limit,
        "Provider message exceeds its size limit"
    );
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub(crate) fn write_frame(writer: &mut impl Write, value: &serde_json::Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    let limit = if value["op"] == "init" {
        INIT_FRAME_LIMIT
    } else {
        FRAME_LIMIT
    };
    ensure!(
        bytes.len() <= limit,
        "Provider message exceeds its size limit"
    );
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
    /// An extension's processor sees file bytes and must not reach the web. Its
    /// jobs that can reach the web read download errors, so its text stops here.
    hide_errors: bool,
    scope: uuid::Uuid,
    owner: uuid::Uuid,
    handoff: Option<super::handoff::Keys>,
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
        extension: Option<&crate::extensions::Context>,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        let cli = std::env::current_exe()?
            .parent()
            .context("Missing application directory")?
            .join(crate::paths::executable_name("de-cli"));
        Self::start_at(&cli, paths, script, role, extension, cancel)
    }
    pub(crate) fn start_at(
        cli: &std::path::Path,
        paths: &AppPaths,
        script: &Script,
        role: Role,
        extension: Option<&crate::extensions::Context>,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        Self::start_inner(cli, paths, script, role, extension, None, cancel)
    }
    pub(crate) fn start_processor(
        paths: &AppPaths,
        script: &Script,
        extension: Option<&crate::extensions::Context>,
        handoff: super::handoff::Keys,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        let cli = std::env::current_exe()?
            .parent()
            .context("Missing application directory")?
            .join(crate::paths::executable_name("de-cli"));
        Self::start_inner(
            &cli,
            paths,
            script,
            Role::Processor,
            extension,
            Some(handoff),
            cancel,
        )
    }
    fn start_inner(
        cli: &std::path::Path,
        paths: &AppPaths,
        script: &Script,
        role: Role,
        extension: Option<&crate::extensions::Context>,
        handoff: Option<super::handoff::Keys>,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        script.validate()?;
        let owner = uuid::Uuid::parse_str(&script.id)?;
        let scope = handoff
            .as_ref()
            .map_or_else(uuid::Uuid::new_v4, |keys| keys.scope);
        if let Some(keys) = &handoff {
            ensure!(
                role == Role::Processor,
                "Resolver cannot receive session keys"
            );
            keys.validate(scope, owner)?;
        }
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
            hide_errors: role == Role::Processor && extension.is_some(),
            scope,
            owner,
            handoff: None,
        };
        worker.call(
            serde_json::json!({"op":"init","source":script.body,"role":role,"script_id":script.id,"extension":extension,"handoff_scope":scope,"handoff":handoff}),
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
                        ensure!(
                            !self.hide_errors,
                            "Provider: this extension's processor failed"
                        );
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
        match self.resolve_any(url, serde_json::to_value(previous)?, cancel)? {
            super::Resolution::File(plan) => Ok(plan),
            super::Resolution::Media(_) => {
                anyhow::bail!("Provider changed from file to media. Restart this download.")
            }
        }
    }
    pub(crate) fn resolve_any(
        &mut self,
        url: &str,
        previous: serde_json::Value,
        cancel: &CancellationToken,
    ) -> Result<super::Resolution> {
        ensure!(
            self.role == Role::Resolver,
            "Processor workers cannot resolve or refresh"
        );
        self.handoff = None;
        let mut value = self.call(
            serde_json::json!({"op":"resolve","url":url,"previous":previous}),
            cancel,
        )?;
        let plan = super::Resolution::parse(value["plan"].take())?;
        let keys: super::handoff::Keys = serde_json::from_value(value["handoff"].take())?;
        keys.validate(self.scope, self.owner)?;
        keys.validate_plan(&plan)?;
        self.handoff = Some(keys);
        Ok(plan)
    }
    pub(crate) fn take_handoff(&mut self) -> Result<super::handoff::Keys> {
        self.handoff
            .take()
            .context("Provider must resolve before handing off keys")
    }
    pub(crate) fn transform_media(
        &mut self,
        context: &super::media::Chunk,
        bytes: &[u8],
        offset: u64,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        ensure!(
            self.role == Role::Processor && bytes.len() <= CHUNK_BYTES,
            "Invalid media processor request"
        );
        let value = self.call(serde_json::json!({"op":"media_transform","context":context,"bytes":STANDARD.encode(bytes),"offset":offset}), cancel)?;
        let text = value.as_str().context("Invalid provider chunk")?;
        ensure!(
            text.len() <= CHUNK_BYTES.div_ceil(3) * 4,
            "Provider output exceeds 256 KiB"
        );
        let output = STANDARD.decode(text)?;
        ensure!(
            output.len() == bytes.len(),
            "Provider must return exactly one output byte per input byte"
        );
        Ok(output)
    }
    pub(crate) fn finish_media(
        &mut self,
        plan: &super::media::Plan,
        output_size: u64,
        cancel: &CancellationToken,
    ) -> Result<()> {
        ensure!(
            self.role == Role::Processor,
            "Resolver workers cannot finish files"
        );
        let value = self.call(
            serde_json::json!({"op":"media_finish","plan":plan,"size":output_size}),
            cancel,
        )?;
        ensure!(value == true, "Provider finish must return true");
        Ok(())
    }
    pub(crate) fn whole_segment(
        &mut self,
        context: &super::media::Chunk,
        input: &mut impl Read,
        prepare: bool,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        let mut result = Vec::new();
        self.segment_to(context, input, &mut result, prepare, false, cancel)?;
        Ok(result)
    }
    pub(crate) fn stream_segment(
        &mut self,
        context: &super::media::Chunk,
        input: &mut impl Read,
        output: &mut impl Write,
        cancel: &CancellationToken,
    ) -> Result<u64> {
        self.segment_to(context, input, output, false, true, cancel)
    }
    fn segment_to(
        &mut self,
        context: &super::media::Chunk,
        input: &mut impl Read,
        output: &mut impl Write,
        prepare: bool,
        stream: bool,
        cancel: &CancellationToken,
    ) -> Result<u64> {
        context.validate()?;
        ensure!(
            self.role == Role::Processor
                && (stream || context.size <= super::media::MAX_SEGMENT_INPUT as u64),
            "Invalid whole-segment request"
        );
        self.call(
            serde_json::json!({"op":"segment_begin","context":context,"prepare":prepare,"stream":stream}),
            cancel,
        )?;
        let mut buffer = vec![0; CHUNK_BYTES];
        let mut offset = 0;
        while offset < context.size {
            let count = (context.size - offset).min(CHUNK_BYTES as u64) as usize;
            input.read_exact(&mut buffer[..count])?;
            self.call(
                serde_json::json!({"op":"segment_feed","bytes":STANDARD.encode(&buffer[..count])}),
                cancel,
            )?;
            offset += count as u64;
        }
        let length = self
            .call(serde_json::json!({"op":"segment_run"}), cancel)?
            .as_u64()
            .context("Missing segment output size")?;
        ensure!(
            if prepare {
                length == 0
            } else {
                length > 0
                    && length
                        <= if stream {
                            super::media::MAX_STREAM_OUTPUT
                        } else {
                            super::media::MAX_SEGMENT_OUTPUT as u64
                        }
            },
            "Invalid segment output size"
        );
        let mut written = 0;
        while written < length {
            let value = self.call(
                serde_json::json!({"op":"segment_read","offset":written}),
                cancel,
            )?;
            let encoded = value.as_str().context("Invalid segment output")?;
            ensure!(
                encoded.len() <= CHUNK_BYTES.div_ceil(3) * 4,
                "Segment output chunk exceeds limit"
            );
            let bytes = STANDARD.decode(encoded)?;
            ensure!(
                !bytes.is_empty()
                    && bytes.len() <= CHUNK_BYTES
                    && written + bytes.len() as u64 <= length,
                "Invalid segment output chunk"
            );
            output.write_all(&bytes)?;
            written += bytes.len() as u64;
        }
        Ok(written)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_extension_processors_error_text_stays_in_the_worker() {
        for (hide_errors, expected) in [
            (true, "Provider: this extension's processor failed"),
            (false, "Provider: file bytes"),
        ] {
            let (requests, _inbox) = mpsc::sync_channel(1);
            let (replies, rx) = mpsc::sync_channel(1);
            let mut worker = Worker {
                process: None,
                requests: Some(requests),
                replies: rx,
                io: None,
                timeout: Duration::from_secs(5),
                role: Role::Processor,
                hide_errors,
                scope: uuid::Uuid::new_v4(),
                owner: uuid::Uuid::new_v4(),
                handoff: None,
            };
            replies
                .send(Ok(serde_json::json!({"error":"file bytes"})))
                .unwrap();
            let error = worker
                .call(serde_json::json!({}), &CancellationToken::new())
                .unwrap_err();
            assert_eq!(error.to_string(), expected);
        }
    }
}
