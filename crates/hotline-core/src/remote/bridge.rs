//! Each bridge is a loopback capability, never the remote device's private key.
use super::client::{Client, PairedDesk, State};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        handshake::server::{ErrorResponse, Request, Response},
        protocol::WebSocketConfig,
    },
};
use tokio_util::sync::CancellationToken;

pub struct Bridge {
    pub origin: String,
    pub token: String,
    pub state: tokio::sync::watch::Receiver<State>,
    cancel: CancellationToken,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl Bridge {
    pub async fn start(desk: &PairedDesk) -> Result<Self, String> {
        Self::with_client(desk, Client::default()).await
    }
    pub async fn with_client(desk: &PairedDesk, client: Client) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "Could not open the local desk bridge.")?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let origin = format!("http://{address}");
        let token = super::secret();
        let cancel = CancellationToken::new();
        let stopped = cancel.clone();
        let (state, receive) = tokio::sync::watch::channel(State::Connecting);
        let desk = desk.clone();
        let key = token.clone();
        tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    _ = stopped.cancelled() => break,
                    _ = tasks.join_next(), if !tasks.is_empty() => {},
                    accepted = listener.accept() => {
                        let Ok((socket, peer)) = accepted else { break; };
                        if !peer.ip().is_loopback() || tasks.len() >= 32 { continue; }
                        let client = client.clone(); let desk = desk.clone(); let key = key.clone(); let state = state.clone();
                        tasks.spawn(async move { serve(socket, client, desk, key, state).await; });
                    }
                }
            }
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        });
        Ok(Self {
            origin,
            token,
            state: receive,
            cancel,
        })
    }
}

fn authorize(request: &Request, token: &str) -> Result<Option<String>, ()> {
    let url = url::Url::parse(&format!("http://127.0.0.1{}", request.uri())).map_err(|_| ())?;
    let tokens: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| key == "token")
        .collect();
    if tokens.len() != 1 || !crate::wire::same_secret(&tokens[0].1, token) {
        return Err(());
    }
    if url.path() == "/ws" {
        return Ok(None);
    }
    let id = url
        .path()
        .strip_prefix("/computer/")
        .and_then(|path| path.strip_suffix("/ws"))
        .ok_or(())?;
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(());
    }
    Ok(Some(id.into()))
}

#[allow(clippy::result_large_err)] // tungstenite fixes the callback error type.
async fn serve(
    stream: TcpStream,
    client: Client,
    desk: PairedDesk,
    token: String,
    state: tokio::sync::watch::Sender<State>,
) {
    let target = Arc::new(Mutex::new(None));
    let selected = target.clone();
    let callback =
        move |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
            match authorize(request, &token) {
                Ok(persona) => {
                    *selected.lock().unwrap() = persona;
                    Ok(response)
                }
                Err(()) => Err(http::Response::builder()
                    .status(403)
                    .body(Some("Forbidden".into()))
                    .unwrap()),
            }
        };
    let config = WebSocketConfig::default()
        .max_message_size(Some(32 * 1024 * 1024))
        .max_frame_size(Some(32 * 1024 * 1024));
    let Ok(Ok(mut local)) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::accept_hdr_async_with_config(stream, callback, Some(config)),
    )
    .await
    else {
        return;
    };
    let persona = target.lock().unwrap().clone();
    if let Some(persona) = persona {
        viewer(local, client, desk, &persona).await;
        return;
    }
    let mut remote = client.connect(&desk);
    loop {
        tokio::select! {
            changed = remote.state.changed() => {
                if changed.is_err() { break; }
                state.send_replace(*remote.state.borrow_and_update());
            }
            frame = local.next() => match frame {
                Some(Ok(Message::Text(text))) => if remote.outgoing.send(text.to_string()).await.is_err() { break; },
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {},
                _ => break,
            },
            frame = remote.incoming.recv() => {
                let Some(frame) = frame else { break; };
                if local.send(Message::text(frame)).await.is_err() { break; }
            }
        }
    }
}

async fn viewer(
    mut local: WebSocketStream<TcpStream>,
    client: Client,
    desk: PairedDesk,
    persona: &str,
) {
    let Ok(mut remote) = client.open(&desk, Some(persona)).await else {
        return;
    };
    loop {
        tokio::select! {
            frame = local.next() => match frame {
                Some(Ok(Message::Text(text))) if text.len() <= 65536 => {
                    if remote.send(Message::text(json!({"type":"text","data":text.as_str()}).to_string())).await.is_err() { break; }
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {},
                _ => break,
            },
            frame = remote.next() => {
                let Some(Ok(Message::Text(text))) = frame else { break; };
                let Ok(value) = serde_json::from_str::<Value>(&text) else { break; };
                let Some(data) = value["data"].as_str() else { break; };
                let frame = match value["type"].as_str() {
                    Some("text") => Message::text(data),
                    Some("binary") => { let Ok(bytes) = STANDARD.decode(data) else { break; }; Message::Binary(bytes.into()) }
                    _ => break,
                };
                if local.send(frame).await.is_err() { break; }
            }
        }
    }
}
