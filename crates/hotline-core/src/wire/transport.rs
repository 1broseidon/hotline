//! Abort only a transport whose seat exceeded the write deadline.
use std::io;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;

/// The seat reunites its halves before selecting abortive close. Keeping this
/// on the transport lets plain, TLS and sealed seats share that teardown.
/// Consuming it makes closure precede the caller releasing admission.
pub(crate) trait AbortiveClose {
    fn abortive_close(self) -> io::Result<()>;
}

impl AbortiveClose for TcpStream {
    fn abortive_close(self) -> io::Result<()> {
        let result = socket2::SockRef::from(&self).set_linger(Some(std::time::Duration::ZERO));
        drop(self);
        result
    }
}

impl<S: AbortiveClose> AbortiveClose for tokio_rustls::server::TlsStream<S> {
    fn abortive_close(self) -> io::Result<()> {
        self.into_inner().0.abortive_close()
    }
}

impl<S: AbortiveClose> AbortiveClose for tokio_rustls::client::TlsStream<S> {
    fn abortive_close(self) -> io::Result<()> {
        self.into_inner().0.abortive_close()
    }
}

impl<S: AsyncRead + AsyncWrite + AbortiveClose + Unpin> AbortiveClose for WebSocketStream<S> {
    fn abortive_close(self) -> io::Result<()> {
        self.into_inner().abortive_close()
    }
}

#[cfg(test)]
impl AbortiveClose for tokio::io::DuplexStream {
    fn abortive_close(self) -> io::Result<()> {
        Ok(())
    }
}
