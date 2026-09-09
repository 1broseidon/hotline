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

use crate::contract::{
    Command, Preview, RosterEntry, SessionInfo, SessionState, StreamDelta, Target, ToolStatus,
    TranscriptEvent, ViewName,
};
use crate::log::{Log, StreamId};
use crate::store::previews;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, TcpListener as StdTcpListener};
use std::sync::Arc;
use std::time::Duration;
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
/// not as the answer to the command. The async ones wait on something short:
/// a driver coming up, or, for `prompt`, the agent whose chapter closed being
/// replaced before it hears the message. The read loop awaits them, so a
/// socket's commands are still answered one at a time, in order.
#[async_trait]
pub trait RoomHandle: Send + Sync + 'static {
    /// Serializes policy persistence and reattachment across client sockets.
    fn policy_update_lock(&self) -> Arc<tokio::sync::Mutex<()>>;
    async fn start(&self, persona_id: &str) -> Result<SessionInfo, String>;
    fn stop(&self, persona_id: &str) -> Result<(), String>;
    /// Revokes existing execution before a new policy is written to the log.
    fn invalidate(&self, persona_id: &str) -> Result<(), String>;
    /// A gateway change revokes every session, including cached peer sessions.
    fn invalidate_all(&self) -> Result<(), String>;
    /// Rebuilds a live session from the teammate's current record.
    async fn reattach(&self, persona_id: &str) -> Result<(), String>;
    /// Every live session: a change to the room's servers reaches all of them.
    async fn reattach_all(&self) -> Result<(), String>;
    async fn prompt(
        &self,
        persona_id: &str,
        text: &str,
        reply_to: Option<String>,
        attachments: Option<Vec<crate::contract::Attachment>>,
    ) -> Result<(), String>;
    fn cancel(&self, persona_id: &str) -> Result<(), String>;
    async fn set_model(&self, persona_id: &str, model_id: &str) -> Result<SessionInfo, String>;
    async fn set_mode(&self, persona_id: &str, mode_id: &str) -> Result<SessionInfo, String>;
    async fn set_config(
        &self,
        persona_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<SessionInfo, String>;

    /// The effort levels a catalogue model offers, as picker choices.
    fn models_efforts(&self, model_id: &str) -> Vec<crate::contract::ConfigChoice>;

    /// Answers a permission the agent is waiting behind, refusing when there
    /// is nothing left to answer.
    async fn answer_permission(
        &self,
        persona_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), String>;

    /// Answers a `request_human` card, refusing when nothing is waiting.
    fn answer_human(
        &self,
        persona_id: &str,
        action_id: &str,
        status: crate::contract::HumanAnswer,
        note: Option<String>,
    ) -> Result<(), String>;

    /// Closes the teammate's open chapter, answering with what it became.
    async fn start_fresh_chapter(
        &self,
        persona_id: &str,
    ) -> Result<crate::contract::ChapterSummary, String>;

    /// Reopens the previous chapter's context in place of the current one.
    async fn resume_chapter(
        &self,
        persona_id: &str,
    ) -> Result<crate::contract::ChapterSummary, String>;

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
    /// Starts a device-code login. Returns the prompt as soon as the provider
    /// issues a code; the login keeps running until the person signs in.
    async fn credential_login(
        &self,
        provider_id: &str,
    ) -> Result<crate::contract::LoginPrompt, String>;
    /// How far a login started by [`Self::credential_login`] has got.
    fn login_status(&self, login_id: &str) -> Result<crate::contract::LoginStatus, String>;
    /// What the room knows of every credential, the secrets left in the vault.
    fn credentials(&self) -> Vec<crate::contract::Credential>;

    /// Every harness a teammate could run on here, the built-in one first.
    async fn backends(&self) -> Vec<crate::contract::BackendChoice>;

    /// Every model the desk's keys can reach, for the model picker.
    fn models(&self) -> Vec<crate::contract::ConfigChoice>;

    /// Every model the catalogue lists for this provider, flagged by the
    /// saved filter. An unwired provider is an error; a credential is not
    /// required, because a filter is about the catalogue.
    fn models_catalog(
        &self,
        provider_id: &str,
    ) -> Result<Vec<crate::contract::CatalogModel>, String>;

    /// Copies an existing Toad data directory into this room. The source is
    /// never written.
    fn import(&self, from: &std::path::Path) -> Result<crate::import::Report, String>;

    /// What tools this teammate was given the last time it started.
    fn teammate_tools(&self, persona_id: &str) -> Option<crate::contract::TeammateToolLedger>;

    /// Writes a job onto the room stream and wakes the clock. Times are
    /// milliseconds; `when` is a one-shot's fire, `every` a loop's interval.
    fn schedule_create(
        &self,
        persona_id: &str,
        kind: crate::contract::ScheduleKind,
        when: Option<i64>,
        every: Option<i64>,
        prompt: &str,
        quiet: bool,
    ) -> Result<crate::contract::ScheduledJob, String>;

    fn schedule_list(&self) -> Vec<crate::contract::ScheduledJob>;

    fn schedule_cancel(&self, id: &str) -> Result<(), String>;

    fn schedule_set_quiet(&self, id: &str, quiet: bool) -> Result<(), String>;

    /// Every thread this teammate has with another teammate, newest first.
    /// The events of one are a `{"thread": key}` subscription, which is a
    /// stream like any other.
    fn peer_threads(&self, persona_id: &str) -> Vec<crate::contract::PeerThreadSummary>;

    /// Marks messages in a peer thread read, answering how many moved.
    fn mark_peer_read(&self, key: &str, event_ids: &[String]) -> usize;

    /// Re-reads the models a subscription login can run and answers with
    /// that provider's catalogue as [`Self::models_catalog`] would. A
    /// provider without a login, or whose credential is a key, is an error.
    async fn credential_refresh_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<crate::contract::CatalogModel>, String>;

    /// A teammate is gone: its agent is stopped, every peer session it was a
    /// side of is dropped, and nothing is kept for its id.
    fn forget(&self, persona_id: &str);

    async fn computer_runtimes(&self) -> Vec<crate::contract::RuntimeReport>;
    async fn computer_status(
        &self,
        persona_id: &str,
    ) -> Result<crate::contract::ComputerStatus, String>;
    async fn computer_stop(&self, persona_id: &str) -> Result<(), String>;
    async fn computer_remove(&self, persona_id: &str) -> Result<(), String>;

    /// Starts OAuth discovery and a native browser callback for one HTTP MCP
    /// server. The result contains only a login id, URL and status.
    async fn mcp_auth_start(&self, _server_id: &str) -> Result<Value, String> {
        Err("MCP OAuth sign-in is unavailable on this room.".to_string())
    }

    /// Delivers an OAuth callback to a pending native login. The callback URL
    /// is validated against the listener that created it.
    async fn mcp_auth_callback(
        &self,
        _login_id: &str,
        _callback_url: &str,
    ) -> Result<Value, String> {
        Err("MCP OAuth sign-in is unavailable on this room.".to_string())
    }

    /// Answers a secret-free OAuth status snapshot.
    async fn mcp_auth_status(&self, _server_id: &str) -> Result<Value, String> {
        Err("MCP OAuth sign-in is unavailable on this room.".to_string())
    }

    /// Clears protected MCP OAuth registrations and tokens.
    fn mcp_secret_set(&self, _server_id: &str, _url: &str, _secret: &str) -> Result<(), String> {
        Err("This room keeps no MCP credentials.".to_string())
    }

    async fn mcp_auth_sign_out(&self, _server_id: &str) -> Result<(), String> {
        Err("MCP OAuth sign-out is unavailable on this room.".to_string())
    }
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

/// Whether an accept error is something to wait out, and whether to pause
/// first so a process that has run out of file descriptors is not a spin.
/// `None` means the listener itself is gone.
#[derive(Debug, PartialEq, Eq)]
enum AcceptAgain {
    Now,
    AfterPause,
}

fn accept_again(error: &io::Error) -> Option<AcceptAgain> {
    #[cfg(unix)]
    {
        // accept(2): a network error already pending on the new socket is
        // reported by accept and is the peer's problem, not the listener's.
        match error.raw_os_error() {
            Some(libc::ECONNABORTED)
            | Some(libc::EINTR)
            | Some(libc::ECONNRESET)
            | Some(libc::EPROTO)
            | Some(libc::ENETDOWN)
            | Some(libc::ENETUNREACH)
            | Some(libc::EHOSTDOWN)
            | Some(libc::EHOSTUNREACH)
            | Some(libc::ENOPROTOOPT)
            | Some(libc::EOPNOTSUPP) => return Some(AcceptAgain::Now),
            Some(libc::EMFILE) | Some(libc::ENFILE) => return Some(AcceptAgain::AfterPause),
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
#[allow(clippy::result_large_err)]
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
pub(crate) fn same_secret(presented: &str, expected: &str) -> bool {
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
        match read_command(&frame) {
            Ok(command) if !seat.permits(&command) => {
                reply(
                    sender,
                    id,
                    Err("That seat may not run this command.".to_string()),
                );
            }
            Ok(command) => {
                // `teammate.tools` answers JSON null when there is no ledger,
                // and that null is a value, not a void — collapsing it would
                // make a missing ledger look like delete or stop.
                let keep_null = matches!(command, Command::TeammateTools { .. });
                let result = commands::run(command, log, room).await;
                reply_to(sender, id, result, keep_null);
            }
            Err(error) => reply(sender, id, Err(error)),
        }
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
    reply_to(sender, id, result, false);
}

fn reply_to(
    sender: &mpsc::UnboundedSender<String>,
    id: i64,
    result: Result<Value, String>,
    keep_null: bool,
) {
    let frame = match result {
        Ok(Value::Null) if !keep_null => json!({ "id": id, "ok": true }),
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
    // A task that ended on its own — the stream closed, the socket's writer
    // went away — still occupies this map unless we notice. Reusing the id
    // would otherwise be "already open" for a subscription that will never
    // deliver.
    subscriptions.retain(|_, handle| !handle.is_finished());
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
    let preview: Option<Preview> = previews::preview(log.root(), &persona.id)
        .and_then(|preview| serde_json::from_value(preview).ok());
    let latest = preview.as_ref().map(|preview| preview.at);
    let session = room.info(&persona.id);
    json!(RosterEntry {
        activity: activity_on(log, &persona.id, &session),
        session,
        preview,
        latest,
        persona,
    })
}

/// The title of the tool still running on this tape, if the session is
/// thinking.
///
/// Read from the tape rather than remembered on the view, so a lagged pump
/// or a second snapshot cannot disagree with what the tape says. A call
/// that is in progress sets the title; any other status for that call, a
/// turn ending, or the session leaving thinking clears it. Only the tail is
/// read: a tool still running is by definition near the end, and a tape is
/// only bounded by how much has been said.
fn activity_on(log: &Log, persona_id: &str, session: &SessionInfo) -> Option<String> {
    if session.state != SessionState::Thinking {
        return None;
    }
    let mut activity: Option<(String, String)> = None;
    for event in previews::tail(log.root(), persona_id) {
        let Ok(event) = serde_json::from_value::<TranscriptEvent>(event) else {
            continue;
        };
        match event {
            TranscriptEvent::Tool {
                tool_call_id,
                title,
                status,
                ..
            } => {
                if status == ToolStatus::InProgress {
                    activity = Some((tool_call_id, title));
                } else if activity.as_ref().is_some_and(|(id, _)| *id == tool_call_id) {
                    activity = None;
                }
            }
            TranscriptEvent::Turn { .. } => activity = None,
            _ => {}
        }
    }
    activity.map(|(_, title)| title)
}
