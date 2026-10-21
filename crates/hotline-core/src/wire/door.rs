//! The listening door: binding, accepting and the authenticated handshake.

use super::seat::Seat;
use super::socket::seated;
use super::{Agents, Parts, RoomHandle, relay_door, same_secret};
use crate::log::Log;
use std::io;
use std::net::{Ipv4Addr, TcpListener as StdTcpListener};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::Error;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

/// A bound, not yet running door. Bound synchronously, so the port is known
/// before the shell opens a window that has to reach it.
pub struct Door {
    listener: StdTcpListener,
    port: u16,
    desk_token: String,
    behind: Behind,
}

/// What a door seats its socket on: the room's whole wire, or, on a server
/// that runs only a relay, the four commands that administer it (ADR 0006).
#[derive(Clone)]
enum Behind {
    Room(Agents),
    Relay(Arc<crate::remote::Remote>),
}

impl Door {
    pub fn bind(
        log: Log,
        desk_token: String,
        room: Arc<dyn RoomHandle>,
        parts: Parts,
    ) -> io::Result<Self> {
        Self::bind_to(desk_token, Behind::Room(Agents { log, room, parts }))
    }

    /// The door of a server that runs only a relay: no room behind it.
    pub fn bind_relay(desk_token: String, remote: Arc<crate::remote::Remote>) -> io::Result<Self> {
        Self::bind_to(desk_token, Behind::Relay(remote))
    }

    fn bind_to(desk_token: String, behind: Behind) -> io::Result<Self> {
        let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        Ok(Self {
            listener,
            port,
            desk_token,
            behind,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Serves sockets until the listener itself is gone. A transient accept
    /// error — a client that hung up before the handshake, a brief shortage
    /// of file descriptors, an interrupted call — is waited out, because
    /// returning here kills the wire for the life of the process.
    pub async fn run(self) -> io::Result<()> {
        let listener = TcpListener::from_std(self.listener)?;
        let desk_token = Arc::new(self.desk_token);
        loop {
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                Err(error) => match accept_again(&error) {
                    Some(AcceptAgain::AfterPause) => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                    Some(AcceptAgain::Now) => continue,
                    None => return Err(error),
                },
            };
            let desk_token = desk_token.clone();
            let behind = self.behind.clone();
            tokio::spawn(async move {
                if let Err(error) = serve(stream, &desk_token, behind).await {
                    eprintln!("[wire] a socket ended: {error}");
                }
            });
        }
    }
}

/// Whether an accept error is something to wait out, and whether to pause
/// first so a process that has run out of file descriptors is not a spin.
/// `None` means the listener itself is gone.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum AcceptAgain {
    Now,
    AfterPause,
}

pub(super) fn accept_again(error: &io::Error) -> Option<AcceptAgain> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Networking::WinSock::{WSAECONNRESET, WSAEMFILE, WSAENOBUFS};
        // Winsock reports a reset peer separately from an exhausted socket
        // table or buffer pool. Only resource pressure needs a delayed retry.
        match error.raw_os_error() {
            Some(WSAECONNRESET) => return Some(AcceptAgain::Now),
            Some(WSAEMFILE | WSAENOBUFS) => return Some(AcceptAgain::AfterPause),
            _ => {}
        }
    }
    #[cfg(unix)]
    {
        // accept(2): a network error already pending on the new socket is
        // reported by accept and is the peer's problem, not the listener's.
        match error.raw_os_error() {
            Some(
                libc::ECONNABORTED
                | libc::EINTR
                | libc::ECONNRESET
                | libc::EPROTO
                | libc::ENETDOWN
                | libc::ENETUNREACH
                | libc::EHOSTDOWN
                | libc::EHOSTUNREACH
                | libc::ENOPROTOOPT
                | libc::EOPNOTSUPP,
            ) => return Some(AcceptAgain::Now),
            Some(libc::EMFILE | libc::ENFILE) => return Some(AcceptAgain::AfterPause),
            _ => {}
        }
    }
    match error.kind() {
        io::ErrorKind::ConnectionAborted | io::ErrorKind::Interrupted => Some(AcceptAgain::Now),
        _ => None,
    }
}

// The handshake callback's error is tungstenite's `ErrorResponse`, a whole
// HTTP response by value; the trait fixes the type, so the lint has nothing to
// offer here.
#[expect(clippy::result_large_err, reason = "tungstenite fixes this error type")]
async fn serve(stream: TcpStream, desk_token: &str, behind: Behind) -> Result<(), Error> {
    // Once a seat is dropped, no queued TCP bytes should keep it alive.
    socket2::SockRef::from(&stream).set_linger(Some(Duration::ZERO))?;
    let socket = accept_hdr_async(stream, |request: &Request, response: Response| {
        let uri = request.uri();
        if uri.path() != "/ws" {
            return Err(refuse(
                StatusCode::NOT_FOUND,
                "This door serves the room's wire only.",
            ));
        }
        let presented = uri
            .query()
            .and_then(|query| {
                query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("token="))
            })
            .unwrap_or("");
        if !same_secret(presented, desk_token) {
            return Err(refuse(StatusCode::UNAUTHORIZED, "unauthorized"));
        }
        Ok(response)
    })
    .await?;
    match behind {
        Behind::Room(desk) => seated(socket, Seat::Desk, desk).await,
        Behind::Relay(remote) => relay_door::serve(socket, remote).await,
    }
}

fn refuse(status: StatusCode, body: &str) -> ErrorResponse {
    let mut response = ErrorResponse::new(Some(body.to_string()));
    *response.status_mut() = status;
    response
}
