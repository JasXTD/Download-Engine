//! TCP/uTP policy and lifetime. Every protocol uses the same endpoint restrictions.
use crate::{
    mse::{self, Encryption},
    peer::BoxedStream,
};
use anyhow::{Context as _, Result, ensure};
use librqbit_core::{hash_id::Id20, network::is_public_endpoint};
use librqbit_dualstack_sockets::{
    BindOpts, ConnectOpts, PollSendToVectored, UdpSocket, tcp_connect,
};
use librqbit_utp::{DefaultUtpEnvironment, SocketOpts, Transport, UtpSocket, UtpStream};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    num::NonZeroUsize,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::{OwnedSemaphorePermit, Semaphore},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const UTP_STREAMS: usize = 64;
type Socket = UtpSocket<PublicUdp, DefaultUtpEnvironment>;

type NamedAddresses = Arc<Mutex<HashMap<SocketAddr, usize>>>;
fn canonical(mut addr: SocketAddr) -> SocketAddr {
    if let IpAddr::V6(ip) = addr.ip()
        && let Some(ip) = ip.to_ipv4_mapped()
    {
        addr.set_ip(IpAddr::V4(ip));
    }
    addr
}
struct NamedGrant {
    addresses: NamedAddresses,
    addr: SocketAddr,
}
impl NamedGrant {
    fn new(addresses: NamedAddresses, addr: SocketAddr) -> Self {
        let addr = canonical(addr);
        *addresses.lock().entry(addr).or_default() += 1;
        Self { addresses, addr }
    }
}
impl Drop for NamedGrant {
    fn drop(&mut self) {
        let mut addresses = self.addresses.lock();
        if let Some(count) = addresses.get_mut(&self.addr) {
            *count -= 1;
            if *count == 0 {
                addresses.remove(&self.addr);
            }
        }
    }
}
struct PublicUdp {
    inner: UdpSocket,
    allow_local: bool,
    named: NamedAddresses,
}
impl PublicUdp {
    fn allowed(&self, addr: SocketAddr) -> bool {
        self.allow_local
            || is_public_endpoint(addr)
            || self.named.lock().contains_key(&canonical(addr))
    }
    fn check(&self, addr: SocketAddr) -> io::Result<()> {
        if self.allowed(addr) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Torrent peer endpoint is blocked",
            ))
        }
    }
}
impl Transport for PublicUdp {
    fn try_send_to(&self, buf: &[u8], target: SocketAddr) -> io::Result<usize> {
        self.check(target)?;
        self.inner.try_send_to(buf, target)
    }

    async fn recv_from<'a>(&'a self, buf: &'a mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let mut discarded = 0;
        loop {
            let received = self.inner.recv_from(buf).await?;
            if self.allowed(received.1) {
                return Ok(received);
            }
            discarded += 1;
            if discarded % 32 == 0 {
                tokio::task::yield_now().await;
            }
        }
    }
    async fn send_to<'a>(&'a self, buf: &'a [u8], target: SocketAddr) -> io::Result<usize> {
        self.check(target)?;
        self.inner.send_to(buf, target).await
    }
    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        buf: &[u8],
        target: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        if let Err(error) = self.check(target) {
            return Poll::Ready(Err(error));
        }
        self.inner.poll_send_to(cx, buf, target)
    }
    fn bind_addr(&self) -> SocketAddr {
        self.inner.bind_addr()
    }
}
impl PollSendToVectored for PublicUdp {
    fn poll_send_to_vectored(
        &self,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
        target: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        if let Err(error) = self.check(target) {
            return Poll::Ready(Err(error));
        }
        self.inner.poll_send_to_vectored(cx, bufs, target)
    }
}

struct AbortOnDrop {
    stream: UtpStream,
    _permit: OwnedSemaphorePermit,
    _named: Option<NamedGrant>,
}
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.stream.abort();
    }
}
impl AsyncRead for AbortOnDrop {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}
impl AsyncWrite for AbortOnDrop {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

pub struct Peers {
    pub binding: Option<Arc<librqbit_dualstack_sockets::BindDevice>>,
    pub encryption: Encryption,
    allow_local: bool,
    utp: Mutex<Option<Arc<Socket>>>,
    slots: Arc<Semaphore>,
    cancel: CancellationToken,
    named: NamedAddresses,
    stopping: tokio::sync::Mutex<()>,
}
impl Default for Peers {
    fn default() -> Self {
        Self {
            binding: None,
            encryption: Encryption::Disabled,
            allow_local: false,
            utp: Mutex::new(None),
            slots: Arc::new(Semaphore::new(UTP_STREAMS)),
            cancel: CancellationToken::new(),
            named: Arc::default(),
            stopping: tokio::sync::Mutex::new(()),
        }
    }
}
impl Peers {
    pub async fn new(
        encryption: Encryption,
        utp: bool,
        listen_port: Option<u16>,
        allow_local: bool,
        cancel: CancellationToken,
        binding: Option<Arc<librqbit_dualstack_sockets::BindDevice>>,
    ) -> Result<Arc<Self>> {
        let named = Arc::default();
        let socket = if utp {
            #[cfg(all(test, windows))]
            crate::testkit::forbid_network();
            let port = listen_port.unwrap_or(0);
            let udp = UdpSocket::bind_udp(
                SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port),
                BindOpts {
                    device: binding.as_deref(),
                    ..Default::default()
                },
            )
            .or_else(|_| {
                UdpSocket::bind_udp(
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port),
                    BindOpts {
                        device: binding.as_deref(),
                        request_dualstack: false,
                        ..Default::default()
                    },
                )
            })
            .context("Could not open the torrent UDP port")?;
            Some(UtpSocket::new_with_opts(
                PublicUdp {
                    inner: udp,
                    allow_local,
                    named: Arc::clone(&named),
                },
                DefaultUtpEnvironment::default(),
                SocketOpts {
                    cancellation_token: cancel.child_token(),
                    max_live_vsocks: NonZeroUsize::new(UTP_STREAMS),
                    vsock_rx_bufsize_bytes: NonZeroUsize::new(128 * 1024),
                    vsock_tx_bufsize_bytes_initial: NonZeroUsize::new(32 * 1024),
                    vsock_tx_bufsize_bytes_max: NonZeroUsize::new(256 * 1024),
                    accept_incoming: Some(listen_port.is_some()),
                    dont_wait_for_lastack: true,
                    ..Default::default()
                },
            )?)
        } else {
            None
        };
        Ok(Arc::new(Self {
            binding,
            encryption,
            allow_local,
            utp: Mutex::new(socket),
            cancel,
            named,
            ..Self::default()
        }))
    }

    async fn raw_connect(
        &self,
        addr: SocketAddr,
        udp: bool,
        explicitly_named: bool,
    ) -> Result<BoxedStream> {
        #[cfg(all(test, windows))]
        crate::testkit::forbid_network();
        if udp {
            let socket = self.utp.lock().clone().context("uTP is disabled")?;
            let permit = self
                .slots
                .clone()
                .try_acquire_owned()
                .context("uTP connection limit reached")?;
            // Only an explicitly supplied peer gets an exception, for this socket's lifetime.
            // Discovered addresses never enter this bounded, reference-counted allowlist.
            let named = explicitly_named.then(|| NamedGrant::new(self.named.clone(), addr));
            let stream = timeout(CONNECT_TIMEOUT, socket.connect(addr))
                .await
                .context("uTP connect timed out")??;
            Ok(Box::new(AbortOnDrop {
                stream,
                _permit: permit,
                _named: named,
            }))
        } else {
            let stream = timeout(
                CONNECT_TIMEOUT,
                tcp_connect(
                    addr,
                    ConnectOpts {
                        bind_device: self.binding.as_deref(),
                        ..Default::default()
                    },
                ),
            )
            .await
            .context("TCP connect timed out")??;
            stream.set_nodelay(true)?;
            Ok(Box::new(stream))
        }
    }

    async fn attempt(
        &self,
        addr: SocketAddr,
        hash: Id20,
        udp: bool,
        explicitly_named: bool,
    ) -> Result<(BoxedStream, crate::details::ConnectionInfo)> {
        let stream = self.raw_connect(addr, udp, explicitly_named).await?;
        let (stream, encrypted) = match self.encryption {
            Encryption::Disabled => (stream, false),
            Encryption::Required => (mse::outgoing(stream, hash).await?, true),
            Encryption::Prefer => match mse::outgoing(stream, hash).await {
                Ok(stream) => (stream, true),
                Err(_) => (self.raw_connect(addr, udp, explicitly_named).await?, false), // Fresh connection, only in Prefer mode.
            },
        };
        Ok((
            stream,
            crate::details::ConnectionInfo {
                transport: if udp {
                    crate::details::PeerTransport::Utp
                } else {
                    crate::details::PeerTransport::Tcp
                },
                encrypted: Some(encrypted),
            },
        ))
    }
    pub(crate) async fn connect(
        &self,
        addr: SocketAddr,
        hash: Id20,
        explicitly_named: bool,
    ) -> Result<(BoxedStream, crate::details::ConnectionInfo)> {
        if let Some(device) = &self.binding {
            device.check()?;
        }
        ensure!(
            self.allow_local || explicitly_named || is_public_endpoint(addr),
            "Torrent peer endpoint is blocked"
        );
        tokio::select! {
            _ = self.cancel.cancelled() => anyhow::bail!("Torrent session has stopped"),
            result = async {
                match self.attempt(addr,hash,false,explicitly_named).await {
                    Ok(stream) => Ok(stream),
                    Err(error) if self.utp.lock().is_none() => Err(error),
                    Err(_) => self.attempt(addr,hash,true,explicitly_named).await,
                }
            } => result,
        }
    }
    pub(crate) async fn accept_utp(&self) -> Result<(BoxedStream, SocketAddr)> {
        let socket = self.utp.lock().clone().context("uTP is disabled")?;
        // Backpressure must not terminate the session's listener when all slots are busy.
        let permit = self.slots.clone().acquire_owned().await?;
        let stream = socket.accept().await?;
        let addr = stream.remote_addr();
        Ok((
            Box::new(AbortOnDrop {
                stream,
                _permit: permit,
                _named: None,
            }),
            addr,
        ))
    }
    pub(crate) async fn shutdown(&self) {
        self.cancel.cancel();
        let _stopping = self.stopping.lock().await;
        // Retain the socket until joining finishes. A cancelled shutdown can be retried;
        // concurrent callers cannot return before the first one has released the port.
        let socket = self.utp.lock().clone();
        if let Some(socket) = socket {
            socket.shutdown().await;
            self.utp.lock().take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_exceptions_are_exact_canonical_and_released() {
        let addresses: NamedAddresses = Arc::default();
        let first = NamedGrant::new(addresses.clone(), "127.0.0.1:3000".parse().unwrap());
        let second = NamedGrant::new(
            addresses.clone(),
            "[::ffff:127.0.0.1]:3000".parse().unwrap(),
        );
        assert_eq!(addresses.lock().len(), 1);
        assert!(
            !addresses
                .lock()
                .contains_key(&"127.0.0.1:3001".parse().unwrap())
        );
        drop(first);
        assert_eq!(addresses.lock().values().copied().sum::<usize>(), 1);
        drop(second);
        assert!(addresses.lock().is_empty());
    }
    #[tokio::test]
    async fn discovered_private_peers_fail_before_a_socket_opens() {
        let peers = Peers::default();
        for addr in [
            "127.0.0.1:3000",
            "10.1.2.3:3000",
            "[::1]:3000",
            "1.2.3.4:22",
        ] {
            let error = peers
                .connect(addr.parse().unwrap(), Id20::new([1; 20]), false)
                .await
                .err()
                .unwrap();
            assert!(error.to_string().contains("blocked"));
        }
    }
}
