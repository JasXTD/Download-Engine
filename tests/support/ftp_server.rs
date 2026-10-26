//! Isolated loopback FTP/FTPS fixture. Certificates/keys are public test data;
//! custom trust exists only in test connectors, never in the Windows trust store.
#![allow(dead_code)]
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;

pub const CERT: &[u8] = include_bytes!("../fixtures/ftp/server.cer");
pub const OTHER_CERT: &[u8] = include_bytes!("../fixtures/ftp/alternate.cer");
const PFX: &[u8] = include_bytes!("../fixtures/ftp/server.pfx");
const OTHER_PFX: &[u8] = include_bytes!("../fixtures/ftp/alternate.pfx");
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Plain,
    Explicit,
    Implicit,
}
#[derive(Clone)]
pub struct Config {
    pub body: Vec<u8>,
    pub modified: String,
    pub size: bool,
    pub timestamp: bool,
    pub epsv: bool,
    pub rest: bool,
    pub cut: Option<usize>,
    pub delay: Duration,
    pub retr_status: Option<u16>,
    pub end_status: u16,
    pub reject_auth: bool,
    pub reject_prot: bool,
    pub other_data_certificate: bool,
    pub low_port: bool,
    pub stall_greeting: bool,
    pub stall_data: bool,
    pub huge_reply: bool,
    pub require_login: bool,
    pub change_after: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            body: (0..256 * 1024).map(|n| (n % 251) as u8).collect(),
            modified: "20261011120000.123".into(),
            size: true,
            timestamp: true,
            epsv: true,
            rest: true,
            cut: None,
            delay: Duration::ZERO,
            retr_status: None,
            end_status: 226,
            reject_auth: false,
            reject_prot: false,
            other_data_certificate: false,
            low_port: false,
            stall_greeting: false,
            stall_data: false,
            huge_reply: false,
            require_login: false,
            change_after: false,
        }
    }
}
pub struct State {
    pub config: Mutex<Config>,
    pub commands: Mutex<Vec<String>>,
    pub sent: AtomicUsize,
    pub connections: AtomicUsize,
    pub protected_data: AtomicUsize,
}
pub struct Server {
    pub address: SocketAddr,
    pub state: Arc<State>,
    stop: CancellationToken,
}
impl Server {
    pub async fn new(mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(State {
            config: Mutex::new(Config::default()),
            commands: Mutex::new(Vec::new()),
            sent: AtomicUsize::new(0),
            connections: AtomicUsize::new(0),
            protected_data: AtomicUsize::new(0),
        });
        let stop = CancellationToken::new();
        let token = stop.clone();
        let data = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _=token.cancelled()=>break,
                    accepted=listener.accept()=> {
                        let Ok((socket,_))=accepted else {break};
                        data.connections.fetch_add(1,Ordering::SeqCst);
                        let data=data.clone();let token=token.clone();
                        tokio::spawn(async move {tokio::select!{_=token.cancelled()=>{},_=serve(socket,mode,data)=>{}}});
                    }
                }
            }
        });
        Self {
            address,
            state,
            stop,
        }
    }
    pub fn url(&self, secure: bool) -> String {
        format!(
            "{}://{}:{}/fixture.bin",
            if secure { "ftps" } else { "ftp" },
            if secure { "localhost" } else { "127.0.0.1" },
            self.address.port()
        )
    }
    pub fn update(&self, change: impl FnOnce(&mut Config)) {
        change(&mut self.state.config.lock().unwrap());
    }
    pub fn commands(&self) -> Vec<String> {
        self.state.commands.lock().unwrap().clone()
    }
    pub async fn received(&self, bytes: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while self.state.sent.load(Ordering::SeqCst) < bytes {
                tokio::time::sleep(Duration::from_millis(5)).await
            }
        })
        .await
        .unwrap();
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
type Stream = Box<dyn Io>;
async fn tls(socket: Stream, other: bool) -> anyhow::Result<Stream> {
    let identity =
        native_tls::Identity::from_pkcs12(if other { OTHER_PFX } else { PFX }, "fixture")?;
    let acceptor = tokio_native_tls::TlsAcceptor::from(
        native_tls::TlsAcceptor::builder(identity)
            .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
            .build()?,
    );
    Ok(Box::new(acceptor.accept(socket).await?))
}
async fn write(socket: &mut Stream, text: &str) -> anyhow::Result<()> {
    socket.write_all(text.as_bytes()).await?;
    socket.flush().await?;
    Ok(())
}
async fn command(socket: &mut Stream) -> anyhow::Result<String> {
    let mut line = Vec::new();
    loop {
        let byte = socket.read_u8().await?;
        line.push(byte);
        anyhow::ensure!(line.len() < 8192, "long fixture command");
        if byte == b'\n' {
            break;
        }
    }
    Ok(String::from_utf8(line)?
        .trim_end_matches(['\r', '\n'])
        .into())
}
async fn serve(socket: TcpStream, mode: Mode, state: Arc<State>) -> anyhow::Result<()> {
    let mut socket: Stream = Box::new(socket);
    let mut encrypted = false;
    if mode == Mode::Implicit {
        socket = tls(socket, false).await?;
        encrypted = true;
    }
    let initial = state.config.lock().unwrap().clone();
    if initial.stall_greeting {
        std::future::pending::<()>().await;
    }
    if initial.huge_reply {
        write(&mut socket, &format!("220 {}\r\n", "x".repeat(5000))).await?;
        return Ok(());
    }
    write(&mut socket, "220-Test fixture\r\n220 Ready\r\n").await?;
    let mut data = None;
    let mut offset = 0;
    let mut user = String::new();
    let mut protected = false;
    loop {
        let line = command(&mut socket).await?;
        state.commands.lock().unwrap().push(line.clone());
        let (verb, arg) = line.split_once(' ').unwrap_or((&line, ""));
        let config = state.config.lock().unwrap().clone();
        match verb {
            "AUTH" if mode == Mode::Explicit && !config.reject_auth => {
                write(&mut socket, "234 Start TLS\r\n").await?;
                socket = tls(socket, false).await?;
                encrypted = true;
            }
            "AUTH" => write(&mut socket, "500 TLS unavailable\r\n").await?,
            "PBSZ" if encrypted => write(&mut socket, "200 PBSZ accepted\r\n").await?,
            "PROT" if encrypted && arg == "P" && !config.reject_prot => {
                protected = true;
                write(&mut socket, "200 Private data\r\n").await?;
            }
            "PROT" => write(&mut socket, "534 Protection unavailable\r\n").await?,
            "USER" => {
                user = arg.into();
                write(&mut socket, "331 Password required\r\n").await?;
            }
            "PASS" => {
                if (config.require_login
                    && (!encrypted || user != "fixture-user" || arg != "fixture-password"))
                    || (!config.require_login && user != "anonymous")
                {
                    write(&mut socket, "530 fixture-password rejected\r\n").await?;
                } else {
                    write(&mut socket, "230 Welcome\r\n").await?;
                }
            }
            "TYPE" | "OPTS" => write(&mut socket, "200 Accepted\r\n").await?,
            "CWD" => write(&mut socket, "250 Directory changed\r\n").await?,
            "SIZE" if config.size => {
                write(&mut socket, &format!("213 {}\r\n", config.body.len())).await?
            }
            "MDTM" if config.timestamp => {
                write(&mut socket, &format!("213 {}\r\n", config.modified)).await?
            }
            "EPSV" if !config.epsv => write(&mut socket, "502 Use PASV\r\n").await?,
            "EPSV" | "PASV" => {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let port = if config.low_port {
                    22
                } else {
                    listener.local_addr()?.port()
                };
                data = Some(listener);
                if verb == "EPSV" {
                    write(
                        &mut socket,
                        &format!("229 Entering Extended Passive Mode (|||{port}|)\r\n"),
                    )
                    .await?;
                } else {
                    write(
                        &mut socket,
                        &format!(
                            "227 Entering Passive Mode (10,99,88,77,{},{})\r\n",
                            port / 256,
                            port % 256
                        ),
                    )
                    .await?;
                }
            }
            "REST" if config.rest => {
                offset = arg.parse()?;
                write(&mut socket, "350 Restart accepted\r\n").await?;
            }
            "RETR" => {
                if let Some(code) = config.retr_status {
                    write(
                        &mut socket,
                        &format!("{code} fixture-password unavailable\r\n"),
                    )
                    .await?;
                    continue;
                }
                write(&mut socket, "150 Sending file\r\n").await?;
                let (channel, _) = data.take().unwrap().accept().await?;
                let mut channel: Stream = Box::new(channel);
                if protected {
                    channel = tls(channel, config.other_data_certificate).await?;
                    state.protected_data.fetch_add(1, Ordering::SeqCst);
                }
                if config.stall_data {
                    std::future::pending::<()>().await;
                }
                let end = config
                    .cut
                    .map_or(config.body.len(), |n| (offset + n).min(config.body.len()));
                for chunk in config
                    .body
                    .get(offset..end)
                    .unwrap_or_default()
                    .chunks(4096)
                {
                    channel.write_all(chunk).await?;
                    channel.flush().await?;
                    state.sent.fetch_add(chunk.len(), Ordering::SeqCst);
                    if !config.delay.is_zero() {
                        tokio::time::sleep(config.delay).await;
                    }
                }
                channel.shutdown().await?;
                drop(channel);
                if config.change_after {
                    state.config.lock().unwrap().modified = "20261012120000.123".into();
                }
                write(
                    &mut socket,
                    &format!("{} Transfer complete\r\n", config.end_status),
                )
                .await?;
            }
            _ => write(&mut socket, "502 Unsupported\r\n").await?,
        }
    }
}
