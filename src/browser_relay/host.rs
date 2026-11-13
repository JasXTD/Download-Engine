//! Native messaging owns the browser connection; only authenticated local IPC
//! can ask it to fetch from the reviewed origins. Chunks are pulled, not pushed.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, mpsc},
    task::JoinSet,
};

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Reply {
    Head {
        id: Uuid,
        status: u16,
        headers: Vec<(String, String)>,
    },
    Chunk {
        id: Uuid,
        data: String,
    },
    End {
        id: Uuid,
    },
    Failed {
        id: Uuid,
        #[serde(default)]
        reason: Failure,
    },
}
impl Reply {
    fn id(&self) -> Uuid {
        match self {
            Self::Head { id, .. }
            | Self::Chunk { id, .. }
            | Self::End { id }
            | Self::Failed { id, .. } => *id,
        }
    }
}
type Pending = Arc<Mutex<HashMap<Uuid, mpsc::Sender<Reply>>>>;
struct RequestGuard {
    id: Uuid,
    pending: Pending,
    commands: mpsc::Sender<serde_json::Value>,
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&self.id);
        }
        let _ = self
            .commands
            .try_send(serde_json::json!({"kind":"cancel","id":self.id}));
    }
}

pub fn serve(page: String, origins: Vec<String>, output: &mut impl io::Write) -> Result<()> {
    validate_scope(&page, &origins)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let session = Session {
            port: listener.local_addr()?.port(),
            token: Uuid::new_v4(),
            expires: chrono::Utc::now().timestamp() + LIFETIME_SECONDS,
            page,
            origins,
        };
        let mut ready = crate::native::Reply::success("Browser video connection ready");
        ready.browser_relay = Some(session.clone());
        crate::native::write_frame(output, &ready)?;
        let (input_tx, input) = mpsc::channel(1);
        // Browser disconnect closes stdin. This thread never touches the DB or
        // desktop state, and the process exits when the native port closes.
        std::thread::Builder::new()
            .name("browser-video-input".into())
            .spawn(move || {
                let mut input = io::stdin().lock();
                while let Ok(Some(frame)) = crate::native::read_frame(&mut input) {
                    let Ok(reply) = serde_json::from_slice::<Reply>(&frame) else {
                        break;
                    };
                    if input_tx.blocking_send(reply).is_err() {
                        break;
                    }
                }
            })?;
        run(listener, session, input, output).await
    })
}

async fn run(
    listener: TcpListener,
    session: Session,
    mut input: mpsc::Receiver<Reply>,
    output: &mut impl io::Write,
) -> Result<()> {
    let pending: Pending = Arc::default();
    let (commands, mut outgoing) = mpsc::channel(32);
    let slots = Arc::new(Semaphore::new(8));
    let mut tasks = JoinSet::new();
    let lifetime = tokio::time::sleep(std::time::Duration::from_secs(LIFETIME_SECONDS as u64));
    tokio::pin!(lifetime);
    loop {
        tokio::select! {
            _=&mut lifetime=>break,
            reply=input.recv()=>{
                let Some(reply)=reply else {break};
                let sender=pending.lock().map_err(|_|anyhow::anyhow!("Browser video connection stopped"))?.get(&reply.id()).cloned();
                if let Some(sender)=sender {
                    // More than one reply per pull is a broken protocol, not an
                    // invitation to buffer unbounded data from the browser.
                    if sender.try_send(reply).is_err(){break}
                }
            },
            command=outgoing.recv()=>{
                if let Some(command)=command {crate::native::write_frame(output,&command)?;}
            },
            accepted=listener.accept()=>{
                let (socket,peer)=accepted?;
                if !peer.ip().is_loopback(){continue}
                let Ok(permit)=slots.clone().try_acquire_owned() else {continue};
                let session=session.clone();let pending=pending.clone();let commands=commands.clone();
                tasks.spawn(async move {
                    let _permit=permit;
                    // Errors only close this stream. Never log its URL, token,
                    // content or a page-provided error string.
                    let _=transfer(socket,&session,pending,commands).await;
                });
            },
            _=tasks.join_next(),if !tasks.is_empty()=>{},
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}

async fn next_reply(socket: &TcpStream, replies: &mut mpsc::Receiver<Reply>) -> Result<Reply> {
    let deadline = tokio::time::sleep(HEAD_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            // The desktop sends one request only. EOF or more input means abort;
            // cancellation therefore interrupts even a browser header/body wait.
            result=socket.readable()=>{
                result?;
                let mut byte=[0];
                match socket.try_read(&mut byte) {
                    Err(e) if e.kind()==io::ErrorKind::WouldBlock=>continue,
                    _=>anyhow::bail!("Browser video desktop connection closed"),
                }
            },
            reply=replies.recv()=>return reply.context("Browser disconnected"),
            _=&mut deadline=>anyhow::bail!("Browser video timed out"),
        }
    }
}

async fn transfer(
    mut socket: TcpStream,
    session: &Session,
    pending: Pending,
    commands: mpsc::Sender<serde_json::Value>,
) -> Result<()> {
    let request = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let length = socket.read_u32_le().await? as usize;
        ensure!(
            length > 0 && length <= HEAD_BYTES,
            "Invalid browser video IPC frame"
        );
        let mut data = zeroize::Zeroizing::new(vec![0; length]);
        socket.read_exact(&mut data).await?;
        Ok::<Request, anyhow::Error>(serde_json::from_slice(&data)?)
    })
    .await
    .context("Browser video authentication timed out")??;
    ensure!(
        request.token == session.token,
        "Invalid browser video capability"
    );
    let url = crate::model::validate_url(&request.url)?;
    session.allows(&url)?;
    if let Some(range) = &request.range {
        ensure!(
            range.length > 0
                && range.length <= crate::hls::MAX_PART_BYTES
                && range
                    .start
                    .checked_add(range.length)
                    .is_some_and(|n| n <= i64::MAX as u64),
            "Invalid browser video byte range"
        );
    }
    let id = Uuid::new_v4();
    let (tx, mut replies) = mpsc::channel(1);
    pending
        .lock()
        .map_err(|_| anyhow::anyhow!("Browser video stopped"))?
        .insert(id, tx);
    let _guard = RequestGuard {
        id,
        pending,
        commands: commands.clone(),
    };
    commands
        .send(serde_json::json!({"kind":"fetch","id":id,"url":url.as_str(),"range":request.range}))
        .await?;
    let head = match next_reply(&socket, &mut replies).await? {
        Reply::Head {
            status, headers, ..
        } => Head {
            status,
            headers,
            failure: None,
        },
        Reply::Failed { reason, .. } => Head {
            status: 502,
            headers: vec![],
            failure: Some(reason),
        },
        _ => anyhow::bail!("Invalid browser video response"),
    };
    ensure!(head.headers.len() <= 5, "Too many browser video headers");
    let bytes = serde_json::to_vec(&head)?;
    ensure!(bytes.len() <= HEAD_BYTES, "Browser video headers too large");
    socket.write_u32_le(bytes.len() as u32).await?;
    socket.write_all(&bytes).await?;
    if head.failure.is_some() {
        return Ok(());
    }
    ensure!(
        matches!(head.status, 200 | 206),
        "Browser video request rejected"
    );
    let maximum = request
        .range
        .as_ref()
        .map_or(crate::hls::MAX_PART_BYTES, |r| r.length);
    let mut total = 0u64;
    loop {
        commands
            .send(serde_json::json!({"kind":"read","id":id}))
            .await?;
        match next_reply(&socket, &mut replies).await? {
            Reply::Chunk { data, .. } => {
                ensure!(
                    !data.is_empty() && data.len() <= CHUNK_BYTES.div_ceil(3) * 4,
                    "Invalid browser video chunk"
                );
                let bytes = STANDARD.decode(data)?;
                ensure!(
                    !bytes.is_empty() && bytes.len() <= CHUNK_BYTES,
                    "Invalid browser video chunk"
                );
                total = total
                    .checked_add(bytes.len() as u64)
                    .context("Browser video size overflow")?;
                ensure!(total <= maximum, "Browser video exceeds size limit");
                // Backpressure allows only one browser chunk in flight. Slow
                // user bandwidth caps do not get mistaken for a network stall.
                socket.write_u32_le(bytes.len() as u32).await?;
                socket.write_all(&bytes).await?;
            }
            Reply::End { .. } => {
                socket.write_u32_le(0).await?;
                break;
            }
            Reply::Failed { reason, .. } => {
                socket.write_u32_le(FAILURE_FRAME).await?;
                socket.write_u8(reason as u8).await?;
                break;
            }
            _ => anyhow::bail!("Browser video stream failed"),
        }
    }
    Ok(())
}
