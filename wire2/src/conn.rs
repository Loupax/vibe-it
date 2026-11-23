//! A stream socket from either listener kind (TCP, or `unix:` for a forwarder inside a network
//! namespace). Exposes readiness + raw fd for splice(2) and AsyncRead/AsyncWrite for parsed paths.

use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, Interest, ReadBuf};
use tokio::net::{TcpStream, UnixStream};

pub enum Conn {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl Conn {
    pub async fn ready(&self, interest: Interest) -> io::Result<()> {
        match self {
            Conn::Tcp(s) => s.ready(interest).await.map(|_| ()),
            Conn::Unix(s) => s.ready(interest).await.map(|_| ()),
        }
    }

    /// Run a non-blocking syscall on the fd; a WouldBlock result clears tokio's readiness flag.
    pub fn try_io<R>(
        &self,
        interest: Interest,
        f: impl FnOnce() -> io::Result<R>,
    ) -> io::Result<R> {
        match self {
            Conn::Tcp(s) => s.try_io(interest, f),
            Conn::Unix(s) => s.try_io(interest, f),
        }
    }

    /// shutdown(SHUT_WR): tells the peer no more bytes come, the other direction stays open.
    pub fn shutdown_write(&self) {
        // SAFETY: shutdown(2) on an fd this Conn owns; errors (ENOTCONN after a reset) are irrelevant here.
        unsafe { libc::shutdown(self.as_raw_fd(), libc::SHUT_WR) };
    }
}

impl From<TcpStream> for Conn {
    fn from(s: TcpStream) -> Self {
        Conn::Tcp(s)
    }
}

impl AsRawFd for Conn {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Conn::Tcp(s) => s.as_raw_fd(),
            Conn::Unix(s) => s.as_raw_fd(),
        }
    }
}

impl AsyncRead for Conn {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Tcp(s) => Pin::new(s).poll_read(cx, buf),
            Conn::Unix(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Conn {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Conn::Tcp(s) => Pin::new(s).poll_write(cx, buf),
            Conn::Unix(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Tcp(s) => Pin::new(s).poll_flush(cx),
            Conn::Unix(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Tcp(s) => Pin::new(s).poll_shutdown(cx),
            Conn::Unix(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}
