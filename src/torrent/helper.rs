//! Owns `de-torrent.exe`. One copy serves every torrent, starts when the first one needs it,
//! and stops when the last one lets go.
use super::Settings;
use crate::{child_process::Running, store::Store};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const PROGRAM: &str = if cfg!(windows) {
    "de-torrent.exe"
} else {
    "de-torrent"
};
/// Saving a large torrent's progress pushes its files to disk, which can take a while.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
const STOP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REPLY: u64 = 256 * 1024 * 1024;
/// Backstop for all torrents together; parser and protocol limits apply before this.
const HELPER_MEMORY_LIMIT: usize = 1024 * 1024 * 1024;

type Reply = oneshot::Sender<Result<Value>>;

fn program() -> Result<PathBuf> {
    let application = std::env::current_exe()?;
    let folder = application.parent().context("Missing application folder")?;
    let beside = folder.join(PROGRAM);
    if beside.is_file() {
        return Ok(beside);
    }
    // Test programs run one folder below the real ones.
    if folder.ends_with("deps")
        && let Some(above) = folder.parent().map(|folder| folder.join(PROGRAM))
        && above.is_file()
    {
        return Ok(above);
    }
    bail!("The torrent helper is missing. Reinstall Download Engine.")
}

fn arguments(settings: &Settings, state_dir: &Path) -> Vec<std::ffi::OsString> {
    let mut list: Vec<std::ffi::OsString> = vec![
        "--state-dir".into(),
        state_dir.into(),
        // Uploading stops with the download, and nothing listens for strangers.
        "--no-seed".into(),
        "--trackers".into(),
    ];
    if settings.dht {
        list.push("--dht".into());
    }
    if let Some(port) = settings.incoming_port {
        list.push("--port".into());
        list.push(port.to_string().into());
    }
    if !settings.upload {
        list.push("--no-upload".into());
    }
    if let Some(limit) = settings.upload_limit {
        list.push("--upload-limit".into());
        list.push(limit.to_string().into());
    }
    list
}

struct Helper {
    running: Mutex<Option<Running>>,
    input: Mutex<ChildStdin>,
    waiting: Arc<Mutex<VecDeque<Reply>>>,
    alive: Arc<AtomicBool>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Hands each reply line to whoever has waited longest. The helper answers in order.
fn read_replies(output: ChildStdout, waiting: Arc<Mutex<VecDeque<Reply>>>, alive: Arc<AtomicBool>) {
    use std::io::Read;
    let mut output = BufReader::new(output.take(MAX_REPLY));
    let mut line = String::new();
    loop {
        line.clear();
        output.get_mut().set_limit(MAX_REPLY);
        match output.read_line(&mut line) {
            Ok(count) if count > 0 => {}
            _ => break,
        }
        let reply = decode(&line);
        if let Some(waiter) = lock(&waiting).pop_front() {
            let _ = waiter.send(reply);
        }
    }
    alive.store(false, Ordering::Release);
    for waiter in lock(&waiting).drain(..) {
        let _ = waiter.send(Err(stopped()));
    }
}

fn stopped() -> anyhow::Error {
    crate::retry::incomplete("The torrent helper stopped unexpectedly.")
}

fn decode(line: &str) -> Result<Value> {
    let reply: Value = serde_json::from_str(line).context("The torrent helper sent bad data")?;
    if reply["ok"] == Value::Bool(true) {
        return Ok(reply["result"].clone());
    }
    bail!(
        "{}",
        reply["error"]
            .as_str()
            .unwrap_or("The torrent helper failed")
    )
}

impl Helper {
    fn start(settings: &Settings, state_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(state_dir).context("Cannot create the torrent state folder")?;
        let mut command = Command::new(program()?);
        command
            .args(arguments(settings, state_dir))
            .current_dir(state_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut running = Running::spawn(
            &mut command,
            &CancellationToken::new(),
            Some(HELPER_MEMORY_LIMIT),
        )?;
        let input = running.child.stdin.take().context("Missing helper input")?;
        let output = running
            .child
            .stdout
            .take()
            .context("Missing helper output")?;
        let waiting = Arc::new(Mutex::new(VecDeque::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let (queue, flag) = (waiting.clone(), alive.clone());
        thread::Builder::new()
            .name("torrent-helper".into())
            .spawn(move || read_replies(output, queue, flag))
            .context("Cannot start the torrent helper")?;
        Ok(Self {
            running: Mutex::new(Some(running)),
            input: Mutex::new(input),
            waiting,
            alive,
        })
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn send(&self, command: &Value) -> Result<oneshot::Receiver<Result<Value>>> {
        let (reply, answer) = oneshot::channel();
        // One lock over both steps keeps replies in the order of the commands.
        let mut input = lock(&self.input);
        ensure!(self.is_alive(), stopped());
        lock(&self.waiting).push_back(reply);
        let line = format!("{command}\n");
        input
            .write_all(line.as_bytes())
            .and_then(|()| input.flush())
            .map_err(|_| stopped())?;
        Ok(answer)
    }

    /// Ends a helper that stopped answering. Its pipes close, which fails every waiting call.
    fn kill(&self) {
        drop(lock(&self.running).take());
    }

    /// Lets the helper save everything and leave, in the background.
    fn stop(self: Arc<Self>) {
        let _ = thread::Builder::new()
            .name("torrent-helper-stop".into())
            .spawn(move || {
                if let Ok(answer) = self.send(&json!({"cmd": "exit"})) {
                    let _ = wait(answer, STOP_TIMEOUT);
                }
                self.kill();
            });
    }
}

fn wait(mut answer: oneshot::Receiver<Result<Value>>, timeout: Duration) -> Result<Value> {
    let deadline = Instant::now() + timeout;
    loop {
        match answer.try_recv() {
            Ok(reply) => return reply,
            Err(oneshot::error::TryRecvError::Closed) => return Err(stopped()),
            Err(oneshot::error::TryRecvError::Empty) if Instant::now() >= deadline => {
                bail!("The torrent helper stopped answering")
            }
            Err(oneshot::error::TryRecvError::Empty) => {
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// How long a status window's interest in a torrent's details lasts without being renewed.
const WATCH_FOR: Duration = Duration::from_secs(3);
/// What a request came to: a message to show when it worked, or why it failed.
pub type RequestOutcome = std::result::Result<Option<&'static str>, String>;

#[derive(Default)]
struct Shared {
    helper: Option<Arc<Helper>>,
    users: usize,
    selections: HashMap<String, Vec<usize>>,
    test_peers: Vec<SocketAddr>,
    network: Option<(bool, Option<u16>)>,
    watched: HashMap<String, Instant>,
    requests: HashMap<String, Vec<(u64, super::Request)>>,
    outcomes: HashMap<String, (u64, RequestOutcome)>,
    tickets: u64,
}

/// The torrent helper as all of a store's clones see it. Costs nothing until it is used.
#[derive(Clone, Default)]
pub struct Host(Arc<Mutex<Shared>>);

impl Host {
    fn shared(&self) -> MutexGuard<'_, Shared> {
        lock(&self.0)
    }

    /// Keeps the helper running until the lease is dropped, starting it if needed.
    pub(super) fn lease(&self, store: &Store) -> Result<Lease> {
        let mut shared = self.shared();
        let helper = match shared.helper.clone().filter(|helper| helper.is_alive()) {
            Some(helper) => helper,
            None => {
                let settings = store.torrent_settings()?;
                ensure!(settings.enabled, super::OFF_MESSAGE);
                let state_dir = store.paths.data.join("torrents");
                let helper = Arc::new(Helper::start(&settings, &state_dir)?);
                shared.network = Some((settings.dht, settings.incoming_port));
                shared.helper = Some(helper.clone());
                helper
            }
        };
        shared.users += 1;
        Ok(Lease {
            host: self.clone(),
            helper,
        })
    }

    /// Whether the helper is running. It should only be while a torrent needs it.
    pub fn is_running(&self) -> bool {
        self.shared().helper.is_some()
    }

    pub(crate) fn network_changed(&self, settings: &Settings) -> bool {
        let shared = self.shared();
        shared.helper.is_some() && shared.network != Some((settings.dht, settings.incoming_port))
    }

    /// Scheduler workers are settled before this. An outstanding metadata
    /// preview receives an error and can be retried with the new network policy.
    pub(crate) fn restart(&self) {
        let mut shared = self.shared();
        if let Some(helper) = shared.helper.take() {
            helper.kill();
        }
        shared.network = None;
    }

    /// Passes changed settings to a running helper. One that starts later reads them itself.
    pub(crate) fn configure(&self, settings: &Settings) {
        if let Some(helper) = &self.shared().helper {
            let _ = helper.send(&json!({
                "cmd": "configure",
                "upload_limit": settings.upload_limit,
                "no_upload": !settings.upload,
            }));
        }
    }

    /// Hands new chosen files to the worker running this torrent.
    pub(crate) fn select(&self, id: &str, chosen: Vec<usize>) {
        self.shared().selections.insert(id.into(), chosen);
    }

    pub(super) fn take_selection(&self, id: &str) -> Option<Vec<usize>> {
        self.shared().selections.remove(id)
    }

    /// A status window shows this torrent's details. Asking again keeps them coming; they
    /// stop a few seconds after the last time.
    pub fn watch(&self, id: &str) {
        self.shared().watched.insert(id.into(), Instant::now());
    }

    pub(super) fn is_watched(&self, id: &str) -> bool {
        let mut shared = self.shared();
        shared
            .watched
            .retain(|_, since| since.elapsed() < WATCH_FOR);
        shared.watched.contains_key(id)
    }

    /// A number for a request, so that its outcome can be found again.
    pub fn new_ticket(&self) -> u64 {
        let mut shared = self.shared();
        shared.tickets += 1;
        shared.tickets
    }

    /// Leaves a request for the worker running this torrent.
    pub(crate) fn queue_request(&self, id: &str, ticket: u64, request: super::Request) {
        self.shared()
            .requests
            .entry(id.into())
            .or_default()
            .push((ticket, request));
    }

    pub(super) fn take_requests(&self, id: &str) -> Vec<(u64, super::Request)> {
        self.shared().requests.remove(id).unwrap_or_default()
    }

    /// Records what a request came to. Only the latest outcome per torrent is kept.
    pub(crate) fn settle(&self, id: &str, ticket: u64, outcome: RequestOutcome) {
        self.shared().outcomes.insert(id.into(), (ticket, outcome));
    }

    /// What this request came to, once it has.
    pub fn outcome(&self, id: &str, ticket: u64) -> Option<RequestOutcome> {
        self.shared()
            .outcomes
            .get(id)
            .filter(|(settled, _)| *settled == ticket)
            .map(|(_, outcome)| outcome.clone())
    }

    /// Lets tests name a peer on this computer. There's no other way to add one.
    #[doc(hidden)]
    pub fn set_test_peers(&self, peers: Vec<SocketAddr>) {
        self.shared().test_peers = peers;
    }

    /// Lets tests see what happens when the helper dies.
    #[doc(hidden)]
    pub fn kill_for_tests(&self) {
        if let Some(helper) = &self.shared().helper {
            helper.kill();
        }
    }

    pub(super) fn test_peers(&self) -> Vec<SocketAddr> {
        self.shared().test_peers.clone()
    }
}

pub(super) struct Lease {
    host: Host,
    helper: Arc<Helper>,
}

impl Lease {
    pub(super) async fn call(&self, command: Value) -> Result<Value> {
        let answer = self.helper.send(&command)?;
        match tokio::time::timeout(CALL_TIMEOUT, answer).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => Err(stopped()),
            Err(_) => {
                self.helper.kill();
                Err(stopped())
            }
        }
    }

    /// For threads outside the download runtime.
    pub(super) fn call_blocking(&self, command: Value) -> Result<Value> {
        wait(self.helper.send(&command)?, CALL_TIMEOUT).inspect_err(|error| {
            if error.to_string().contains("stopped answering") {
                self.helper.kill();
            }
        })
    }

    /// Lets a torrent receive up to `until` bytes in all, and says how many it has received.
    pub(super) async fn allow(&self, name: &str, until: Option<u64>) -> Result<u64> {
        let reply = self
            .call(json!({"cmd": "allow", "name": name, "until": until}))
            .await?;
        reply["received"].as_u64().context("Invalid torrent status")
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut shared = self.host.shared();
        shared.users = shared.users.saturating_sub(1);
        if shared.users == 0
            && let Some(helper) = shared.helper.take()
        {
            helper.stop();
        }
    }
}
