//! The wire sees ordinary text frames; only this adapter sees Noise records.
use super::sealed;
use futures_util::{Sink, Stream};
use std::{
    collections::VecDeque,
    io,
    pin::Pin,
    task::{Context, Poll, ready},
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Error, Message},
};

pub(super) const FRAME_MAX: usize = 32 * 1024 * 1024;
fn invalid(reason: impl Into<String>) -> Error {
    Error::Io(io::Error::new(io::ErrorKind::InvalidData, reason.into()))
}

pub(super) struct Channel<S> {
    socket: WebSocketStream<S>,
    state: snow::TransportState,
    decoder: sealed::Decoder,
    pending: VecDeque<Vec<u8>>,
}
impl<S> Channel<S> {
    /// The relay claim ends at message two; its transport state never wraps
    /// the visitor's end-to-end records.
    pub(super) fn into_socket(self) -> WebSocketStream<S> {
        self.socket
    }
    pub(super) fn new(socket: WebSocketStream<S>, state: snow::TransportState) -> Self {
        Self {
            socket,
            state,
            decoder: sealed::Decoder::new(FRAME_MAX),
            pending: VecDeque::new(),
        }
    }
}
impl<S: AsyncRead + AsyncWrite + Unpin> Channel<S> {
    fn drain(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        while !self.pending.is_empty() {
            ready!(Pin::new(&mut self.socket).poll_ready(cx))?;
            let bytes = self.pending.pop_front().unwrap();
            Pin::new(&mut self.socket).start_send(Message::Binary(bytes.into()))?;
        }
        Poll::Ready(Ok(()))
    }
}
impl<S: AsyncRead + AsyncWrite + Unpin> Stream for Channel<S> {
    type Item = Result<Message, Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        // Yield after a bounded number of chunks, including control traffic.
        for _ in 0..32 {
            let Some(message) = ready!(Pin::new(&mut this.socket).poll_next(cx)) else {
                return Poll::Ready(None);
            };
            match message {
                Ok(Message::Binary(bytes)) => match this.decoder.decode(&mut this.state, &bytes) {
                    Ok(Some(text)) => return Poll::Ready(Some(Ok(Message::Text(text.into())))),
                    Ok(None) => {}
                    Err(error) => return Poll::Ready(Some(Err(invalid(error)))),
                },
                Ok(Message::Ping(_) | Message::Pong(_)) => {}
                Ok(Message::Close(close)) => return Poll::Ready(Some(Ok(Message::Close(close)))),
                Ok(_) => {
                    return Poll::Ready(Some(Err(invalid(
                        "Sealed channels require binary Noise messages.",
                    ))));
                }
                Err(error) => return Poll::Ready(Some(Err(error))),
            }
        }
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}
impl<S: AsyncRead + AsyncWrite + Unpin> Sink<Message> for Channel<S> {
    type Error = Error;
    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.get_mut().drain(cx)
    }
    fn start_send(self: Pin<&mut Self>, message: Message) -> Result<(), Error> {
        let this = self.get_mut();
        match message {
            Message::Text(text) if text.len() <= FRAME_MAX => {
                if !this.pending.is_empty() {
                    return Err(invalid("Sealed writer is not ready."));
                }
                this.pending = sealed::encode(&mut this.state, &text)
                    .map_err(invalid)?
                    .into();
                Ok(())
            }
            Message::Close(close) => Pin::new(&mut this.socket).start_send(Message::Close(close)),
            _ => Err(invalid("The sealed wire sends bounded UTF-8 frames only.")),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        let this = self.get_mut();
        ready!(this.drain(cx))?;
        Pin::new(&mut this.socket).poll_flush(cx)
    }
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        let this = self.get_mut();
        ready!(this.drain(cx))?;
        Pin::new(&mut this.socket).poll_close(cx)
    }
}
