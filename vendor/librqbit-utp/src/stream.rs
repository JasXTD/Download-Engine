use std::{
    net::SocketAddr,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::Poll,
};

use tokio::io::{AsyncRead, AsyncWrite};

use crate::{stream_rx::UtpStreamReadHalf, stream_tx::UtpStreamWriteHalf};

pub struct UtpStream {
    reader: UtpStreamReadHalf,
    writer: UtpStreamWriteHalf,
    remote_addr: SocketAddr,
    cancel: tokio_util::sync::CancellationToken,
    reset: Box<dyn Fn() + Send + Sync>,
    aborted: AtomicBool,
}

impl UtpStream {
    pub(crate) fn new(
        reader: UtpStreamReadHalf,
        writer: UtpStreamWriteHalf,
        remote_addr: SocketAddr,
        cancel: tokio_util::sync::CancellationToken,
        reset: Box<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            reader,
            writer,
            remote_addr,
            cancel,
            reset,
            aborted: AtomicBool::new(false),
        }
    }

    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    /// Stop immediately, discarding pending writes. Used by Pause and Stop all.
    pub fn abort(&self) {
        if self.aborted.swap(true, Ordering::AcqRel) {
            return;
        }
        self.cancel.cancel();
        // Notify the peer before the caller releases its endpoint permission. Do not
        // flush queued data, spawn work, or wait for network availability during Drop.
        (self.reset)();
    }

    pub fn split(self) -> (UtpStreamReadHalf, UtpStreamWriteHalf) {
        (self.reader, self.writer)
    }

    #[cfg(test)]
    pub async fn read_all_available(&mut self) -> std::io::Result<Vec<u8>> {
        self.reader.read_all_available().await
    }
}

impl AsyncRead for UtpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().reader).poll_read(cx, buf)
    }
}

impl AsyncWrite for UtpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.get_mut().writer).poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().writer).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().writer).poll_shutdown(cx)
    }
}
