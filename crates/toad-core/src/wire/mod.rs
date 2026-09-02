//! The wire: one WebSocket per client, carrying commands and subscriptions.
//!
//! Frames are JSON text, and there are three a client may send:
//!
//! - `{"id": n, "cmd": "<name>", "params": {…}}`, answered exactly once with
//!   `{"id": n, "ok": true, "result": …}` or `{"id": n, "ok": false,
//!   "error": "<sentence>"}`.
//! - `{"id": n, "sub": <target>}`, answered `{"id": n, "ok": true}` and then
//!   `{"sub": n, "snapshot": [...]}` once, `{"sub": n, "event": {…}}` per
//!   event as it lands, `{"sub": n, "removed": "<id>"}` when a view's row
//!   goes away, and, on a tape, `{"sub": n, "ephemeral": {…}}` for streaming
//!   deltas that are never written down.
//! - `{"id": m, "unsub": n}`, which ends subscription `n`.
//!
//! The ordering rule is the whole reason a subscription is not simply "load,
//! then listen": the broadcast is subscribed to BEFORE the fold is loaded for
//! the snapshot, so no event can land in the gap between them. An event that
//! lands in both is harmless, because every client folds a stream by event id
//! and the second copy of a line supersedes the first with itself.
//!
//! One writer task per socket, fed by an mpsc: answers, snapshots and events
//! are whole frames in the order they were queued, and never interleave.
//! Commands are answered on the read loop, in the order they arrived, because
//! the append a command makes IS its answer and a client that creates a
//! teammate and then subscribes must see it.

use crate::contract::{Command, RosterEntry, SessionInfo, StreamDelta, Target, ViewName};
use crate::log::{Log, StreamId};
use crate::store::previews;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, TcpListener as StdTcpListener};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::{Error, Message};
use tokio_tungstenite::{WebSocketStream, accept_hdr_async};

mod commands;
mod roster;

#[cfg(test)]
mod tests;

/// The live half of the room, which the wire calls but does not own: the
/// sessions and the vault.
///
/// The log is the wire's own — a teammate and a setting are events it appends
/// itself — but a session is a running agent and a credential is a secret,
/// and neither is a thing to reimplement behind a door. This is the seam:
/// exactly the methods the commands and the roster view call, nothing more.
///
/// Every method answers promptly. `prompt` starts a turn and returns; what
/// the turn produces reaches the client as tape events and ephemeral deltas,
/// not as the answer to the command. `start` and `set_model` wait on the
/// driver coming up, which is why they alone are async: the read loop awaits
/// them, so a socket's commands are still answered one at a time, in order.
#[async_trait]
pub trait RoomHandle: Send + Sync + 'static {
    async fn start(&self, persona_id: &str) -> Result<SessionInfo, String>;
    fn stop(&self, persona_id: &str) -> Result<(), String>;
    fn prompt(
        &self,
        persona_id: &str,
        text: &str,
        reply_to: Option<String>,
        attachments: Option<Vec<crate::contract::Attachment>>,
    ) -> Result<(), String>;
    fn cancel(&self, persona_id: &str) -> Result<(), String>;
    async fn set_model(&self, persona_id: &str, model_id: &str) -> Result<SessionInfo, String>;

    /// What this teammate's session is doing, idle when it has none.
    fn info(&self, persona_id: &str) -> SessionInfo;

    /// Every session's info as it changes, for the roster view.
    fn subscribe_info(&self) -> broadcast::Receiver<SessionInfo>;

    /// Text as an agent writes it, for a tape subscription to forward. Never
    /// written to a tape: the durable line lands when the message is whole.
    fn subscribe_deltas(&self) -> broadcast::Receiver<StreamDelta>;

    fn credential_create(
        &self,
        provider_id: &str,
        label: &str,
        secret: &str,
    ) -> Result<crate::contract::Credential, String>;
    fn credential_revoke(&self, id: &str) -> Result<(), String>;
    fn credential_delete(&self, id: &str) -> Result<(), String>;
    /// What the room knows of every credential, the secrets left in the vault.
    fn credentials(&self) -> Vec<crate::contract::Credential>;

    /// Every model the desk's keys can reach, for the model picker.
    fn models(&self) -> Vec<crate::contract::ConfigChoice>;
}

/// What a socket may do, decided by the token it presented.
///
/// A set, not a routing table: the desk seat is the window, and it may do
/// everything the room can do. The phone seat arrives in Phase 3 as a second
/// variant with two smaller answers, and no command has to know about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seat {
    Desk,
}

impl Seat {
    pub fn permits(&self, _command: &Command) -> bool {
        match self {
            Seat::Desk => true,
        }
    }

    pub fn permits_sub(&self, _target: &Target) -> bool {
        match self {
            Seat::Desk => true,
        }
    }
}

/// A bound, not yet running door. Bound synchronously, so the port is known
/// before the shell opens a window that has to reach it.
pub struct Door {
    listener: StdTcpListener,
    port: u16,
    desk_token: String,
    log: Log,
    room: Arc<dyn RoomHandle>,
}

impl Door {
    pub fn bind(log: Log, desk_token: String, room: Arc<dyn RoomHandle>) -> io::Result<Self> {
        let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        Ok(Self {
            listener,
            port,
            desk_token,
            log,
            room,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Serves sockets until the listener fails. Runs on a tokio runtime.
    pub async fn run(self) -> io::Result<()> {
        let listener = TcpListener::from_std(self.listener)?;
        let desk_token = Arc::new(self.desk_token);
        loop {
            let (stream, _) = listener.accept().await?;
            let desk_token = desk_token.clone();
            let log = self.log.clone();
            let room = self.room.clone();
            tokio::spawn(async move {
                if let Err(error) = serve(stream, &desk_token, log, room).await {
                    eprintln!("[wire] a socket ended: {error}");
                }
            });
        }
    }
}

async fn serve(
    stream: TcpStream,
    desk_token: &str,
    log: Log,
    room: Arc<dyn RoomHandle>,
) -> Result<(), Error> {
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
    seated(socket, Seat::Desk, log, room).await
}

fn refuse(status: StatusCode, body: &str) -> ErrorResponse {
    let mut response = ErrorResponse::new(Some(body.to_string()));
    *response.status_mut() = status;
    response
}

/// Equal without leaking, through timing, how much of a wrong token matched.
fn same_secret(presented: &str, expected: &str) -> bool {
    let presented = presented.as_bytes();
    let expected = expected.as_bytes();
    if presented.len() != expected.len() {
        return false;
    }
    presented
        .iter()
        .zip(expected)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// One admitted socket, until either side closes it.
async fn seated<S>(
    socket: WebSocketStream<S>,
    seat: Seat,
    log: Log,
    room: Arc<dyn RoomHandle>,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut incoming) = socket.split();
    let (sender, mut outbox) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(text) = outbox.recv().await {
            if sink.send(Message::text(text)).await.is_err() {
                break;
            }
        }
    });

    let mut subscriptions: HashMap<i64, JoinHandle<()>> = HashMap::new();
    let result = loop {
        match incoming.next().await {
            None | Some(Ok(Message::Close(_))) => break Ok(()),
            Some(Err(error)) => break Err(error),
            Some(Ok(Message::Text(text))) => {
                answer(&text, seat, &log, &room, &sender, &mut subscriptions).await;
            }
            Some(Ok(_)) => {}
        }
    };

    for handle in subscriptions.into_values() {
        handle.abort();
    }
    writer.abort();
    result
}

/// One frame in, its answer queued. A frame with no id is nobody's question,
/// so there is nowhere to put an answer and it is dropped.
async fn answer(
    text: &str,
    seat: Seat,
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    sender: &mpsc::UnboundedSender<String>,
    subscriptions: &mut HashMap<i64, JoinHandle<()>>,
) {
    let Ok(frame) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let Some(id) = frame.get("id").and_then(Value::as_i64) else {
        return;
    };

    if frame.get("cmd").is_some() {
        let result = match read_command(&frame) {
            Ok(command) if !seat.permits(&command) => {
                Err("That seat may not run this command.".to_string())
            }
            Ok(command) => commands::run(command, log, room).await,
            Err(error) => Err(error),
        };
        reply(sender, id, result);
        return;
    }
    if let Some(target) = frame.get("sub") {
        // A subscription that opened answered itself, on its way past the
        // acknowledgement; only a refusal is left to say here.
        if let Err(error) = subscribe(target, seat, log, room, sender, subscriptions, id) {
            reply(sender, id, Err(error));
        }
        return;
    }
    if let Some(sub) = frame.get("unsub").and_then(Value::as_i64) {
        let result = match subscriptions.remove(&sub) {
            Some(handle) => {
                handle.abort();
                Ok(Value::Null)
            }
            None => Err(format!("Subscription {sub} is not open.")),
        };
        reply(sender, id, result);
        return;
    }
    reply(
        sender,
        id,
        Err("A frame is a command, a subscription or an unsubscribe.".to_string()),
    );
}

/// The command frame's `cmd` and `params` are this enum's tag and content, so
/// reading one is putting those two back together without the id.
fn read_command(frame: &Value) -> Result<Command, String> {
    let mut envelope = serde_json::Map::new();
    envelope.insert("cmd".into(), frame["cmd"].clone());
    // A command with nothing to say still has a `params`: absent and `{}`
    // are the same thing to every variant.
    envelope.insert(
        "params".into(),
        frame
            .get("params")
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
    );
    serde_json::from_value(Value::Object(envelope))
        .map_err(|error| format!("This room cannot read that command: {error}."))
}

fn reply(sender: &mpsc::UnboundedSender<String>, id: i64, result: Result<Value, String>) {
    let frame = match result {
        Ok(Value::Null) => json!({ "id": id, "ok": true }),
        Ok(result) => json!({ "id": id, "ok": true, "result": result }),
        Err(error) => json!({ "id": id, "ok": false, "error": error }),
    };
    let _ = sender.send(frame.to_string());
}

/// Opens a subscription.
///
/// The acknowledgement is queued before the task that forwards the stream
/// exists, because a task the runtime starts on another thread would
/// otherwise be free to queue its snapshot first, and a client is promised
/// `ok`, then one snapshot, then events. Inside that task the broadcast is
/// subscribed to before the fold is loaded, so nothing lands in between.
fn subscribe(
    target: &Value,
    seat: Seat,
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    sender: &mpsc::UnboundedSender<String>,
    subscriptions: &mut HashMap<i64, JoinHandle<()>>,
    id: i64,
) -> Result<(), String> {
    let target: Target = serde_json::from_value(target.clone())
        .map_err(|error| format!("That is not something to subscribe to: {error}."))?;
    if !seat.permits_sub(&target) {
        return Err("That seat may not subscribe to that.".to_string());
    }
    if subscriptions.contains_key(&id) {
        return Err(format!("Subscription {id} is already open."));
    }

    reply(sender, id, Ok(Value::Null));

    let stream = match target {
        Target::View(ViewName::Roster) => {
            let room_events = log.subscribe(&StreamId::Room);
            let infos = room.subscribe_info();
            let view = roster::view(
                id,
                log.clone(),
                room.clone(),
                room_events,
                infos,
                sender.clone(),
            );
            subscriptions.insert(id, tokio::spawn(view));
            return Ok(());
        }
        Target::Room => StreamId::Room,
        Target::Tape(persona_id) => StreamId::Tape(persona_id),
        Target::Thread(key) => StreamId::Thread(key),
    };

    let events = log.subscribe(&stream);
    let deltas = match &stream {
        StreamId::Tape(_) => Some(room.subscribe_deltas()),
        _ => None,
    };
    let forward = stream_events(id, stream, log.clone(), events, deltas, sender.clone());
    subscriptions.insert(id, tokio::spawn(forward));
    Ok(())
}

/// A stream's fold, then its events, and on a tape the deltas that are never
/// written beside them.
async fn stream_events(
    id: i64,
    stream: StreamId,
    log: Log,
    mut events: broadcast::Receiver<Value>,
    mut deltas: Option<broadcast::Receiver<StreamDelta>>,
    sender: mpsc::UnboundedSender<String>,
) {
    let persona_id = match &stream {
        StreamId::Tape(persona_id) => persona_id.clone(),
        _ => String::new(),
    };
    if !send(&sender, json!({ "sub": id, "snapshot": log.load(&stream) })) {
        return;
    }

    loop {
        let delta = async {
            match deltas.as_mut() {
                Some(deltas) => deltas.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) => {
                    if !send(&sender, json!({ "sub": id, "event": event })) {
                        return;
                    }
                }
                // Too far behind to be told what it missed, so it is told
                // everything instead: a second snapshot, which a client that
                // folds by id absorbs the same way it absorbed the first.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if !send(&sender, json!({ "sub": id, "snapshot": log.load(&stream) })) {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            delta = delta => match delta {
                Ok(delta) if delta_persona(&delta) == persona_id => {
                    if !send(&sender, json!({ "sub": id, "ephemeral": delta })) {
                        return;
                    }
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                // Nothing to recover: a delta nobody saw is a few characters
                // the durable line will carry anyway.
                Err(broadcast::error::RecvError::Closed) => deltas = None,
            },
        }
    }
}

fn delta_persona(delta: &StreamDelta) -> &str {
    match delta {
        StreamDelta::AgentDelta { persona_id, .. }
        | StreamDelta::ThoughtDelta { persona_id, .. } => persona_id,
    }
}

/// One frame onto the socket's queue. False means the socket is gone.
fn send(sender: &mpsc::UnboundedSender<String>, frame: Value) -> bool {
    sender.send(frame.to_string()).is_ok()
}

/// One roster row, joined out of the room stream, the tape's tail and the
/// live session.
fn roster_entry(log: &Log, room: &Arc<dyn RoomHandle>, persona: crate::contract::Persona) -> Value {
    let preview = previews::preview(log.root(), &persona.id)
        .and_then(|preview| serde_json::from_value(preview).ok());
    let session = room.info(&persona.id);
    json!(RosterEntry {
        session,
        preview,
        persona,
    })
}
