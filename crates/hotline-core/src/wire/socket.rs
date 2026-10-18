//! One seated socket: its outbox, its lanes and the commands it answers.

use super::phone::{PHONE_WINDOW, effort_config, phone_budget};
use super::seat::{PLAIN_REASONS, PRIVATE_REASON, Seat};
use super::subscribe::subscribe;
use super::{RoomHandle, commands, files, threads};
use crate::contract::Command;
use crate::log::Log;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Error, Message};

/// One admitted socket, until either side closes it.
#[derive(Clone)]
pub(super) struct Outbox {
    pub(super) sender: Outgoing,
    pub(super) cancel: tokio_util::sync::CancellationToken,
    pub(super) max: usize,
    /// Attempts are private to the socket that started them, not bearer UUIDs.
    pub(super) auth_attempts: Arc<std::sync::Mutex<HashMap<String, String>>>,
    /// Only this socket's latest invitation is cancelled on disconnect.
    pub(super) pairing: Arc<std::sync::Mutex<Option<String>>>,
    pub(super) uploads: Arc<std::sync::Mutex<files::Uploads>>,
    /// Whether this socket said, with `client.hello`, that it reads `threads2`:
    /// `link` events and `ThreadDelta`s, in place of the per-kind markers and
    /// deltas. Read as each frame is made, so a declaration reaches the
    /// subscriptions already open.
    pub(super) threads2: Arc<std::sync::atomic::AtomicBool>,
    /// Whether this socket said, with `client.hello`, that it is a phone
    /// wanting phone-sized frames (`lean`): history cut the way a
    /// companion's is, whatever the device's role. An owner's phone is an
    /// owner by authority and a phone by its link.
    pub(super) lean: Arc<std::sync::atomic::AtomicBool>,
}
#[derive(Clone)]
pub(super) enum Outgoing {
    Desk(mpsc::UnboundedSender<String>),
    Phone(mpsc::Sender<String>),
}
enum IncomingOutput {
    Desk(mpsc::UnboundedReceiver<String>),
    Phone(mpsc::Receiver<String>),
}
impl IncomingOutput {
    async fn recv(&mut self) -> Option<String> {
        match self {
            Self::Desk(rx) => rx.recv().await,
            Self::Phone(rx) => rx.recv().await,
        }
    }
}
impl Outbox {
    pub(super) fn threads2(&self) -> bool {
        self.threads2.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(super) fn lean(&self) -> bool {
        self.lean.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// A frame that may be lost without harm: live words the durable line
    /// will carry anyway. A phone's queue that is full drops it rather than
    /// the connection, so a slow link falls behind on typing, not offline.
    pub(super) fn send_droppable(&self, text: String) -> Result<(), ()> {
        match &self.sender {
            Outgoing::Phone(sender) if text.len() <= self.max => match sender.try_send(text) {
                Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => Ok(()),
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    self.cancel.cancel();
                    Err(())
                }
            },
            _ => self.send(text),
        }
    }

    pub(super) fn send(&self, text: String) -> Result<(), ()> {
        let oversized = text.len() > self.max;
        let failed = oversized
            || match &self.sender {
                Outgoing::Desk(sender) => sender.send(text).is_err(),
                Outgoing::Phone(sender) => sender.try_send(text).is_err(),
            };
        if failed {
            self.cancel.cancel();
            return Err(());
        }
        Ok(())
    }
}

/// Only the authenticated sealed transport calls this: the hello itself is encrypted.
pub(crate) async fn seated_phone_v2<S>(
    mut socket: S,
    log: Log,
    room: Arc<dyn RoomHandle>,
    phone: crate::remote::Phone,
    role: crate::remote::WireRole,
    desktop_id: &str,
) -> Result<(), Error>
where
    S: Stream<Item = Result<Message, Error>>
        + Sink<Message, Error = Error>
        + Unpin
        + Send
        + 'static,
{
    let mut hello = json!({
        "type": "hello",
        "protocolVersion": 2,
        "desktopId": desktop_id,
        // Every address the desk answers on, a relay's included, so a
        // phone paired before the relay was chosen learns of it here.
        "endpoints": phone.endpoints(),
        "mode": "team",
        "capabilities": Seat::for_phone(role).capabilities_for(room.as_ref()),
    });
    // Which of those endpoints is public, so a phone can tell it from the
    // VPN or LAN ones without the owner-only `remote.status`.
    if let Some(public) = phone.public_endpoint() {
        hello["publicEndpoint"] = json!(public);
    }
    // What kind of desk this is, how new, and what it runs: the owner's to know.
    if role == crate::remote::WireRole::Owner {
        hello["deskKind"] = json!(phone.desk_kind());
        hello["serves"] = json!(phone.serves());
        hello["version"] = json!(env!("CARGO_PKG_VERSION"));
    }
    tokio::select! {
        biased;
        () = phone.cancel.cancelled() => return Ok(()),
        result = socket.send(Message::text(hello.to_string())) => result?,
    }
    seated_inner(socket, Seat::for_phone(role), log, room, Some(phone)).await
}

pub(super) async fn seated<S>(
    socket: WebSocketStream<S>,
    seat: Seat,
    log: Log,
    room: Arc<dyn RoomHandle>,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    seated_inner(socket, seat, log, room, None).await
}

async fn seated_inner<S>(
    socket: S,
    seat: Seat,
    log: Log,
    room: Arc<dyn RoomHandle>,
    phone: Option<crate::remote::Phone>,
) -> Result<(), Error>
where
    S: Stream<Item = Result<Message, Error>>
        + Sink<Message, Error = Error>
        + Unpin
        + Send
        + 'static,
{
    let (mut sink, mut incoming) = socket.split();
    let cancel = phone.as_ref().map(|p| p.cancel.clone()).unwrap_or_default();
    let (sender, mut outbox) = if seat.is_remote() {
        let (tx, rx) = mpsc::channel::<String>(256);
        (Outgoing::Phone(tx), IncomingOutput::Phone(rx))
    } else {
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        (Outgoing::Desk(tx), IncomingOutput::Desk(rx))
    };
    let sender = Outbox {
        auth_attempts: Arc::default(),
        pairing: Arc::default(),
        uploads: Arc::default(),
        threads2: Arc::default(),
        lean: Arc::default(),
        sender,
        cancel: cancel.clone(),
        max: match seat {
            Seat::Phone | Seat::Link => 1_048_576,
            Seat::Owner => 32 * 1024 * 1024,
            Seat::Desk => usize::MAX,
        },
    };
    let writer_cancel = cancel.clone();
    let writer = tokio::spawn(async move {
        while let Some(text) = outbox.recv().await {
            if sink.send(Message::text(text)).await.is_err() {
                break;
            }
        }
        writer_cancel.cancel();
    });

    let mut subscriptions: HashMap<i64, JoinHandle<()>> = HashMap::new();
    let mut lanes: HashMap<String, mpsc::UnboundedSender<Value>> = HashMap::new();
    let result = loop {
        // A lean phone pings; one that has said nothing for this long is a
        // link that died without closing, and its subscriptions are work
        // for nobody.
        let silent = async {
            if sender.lean() {
                tokio::time::sleep(LEAN_IDLE).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        let incoming = tokio::select! {
            biased;
            () = cancel.cancelled() => break Ok(()),
            frame = incoming.next() => frame,
            () = silent => break Ok(()),
        };
        match incoming {
            None | Some(Ok(Message::Close(_))) => break Ok(()),
            Some(Err(error)) => break Err(error),
            Some(Ok(Message::Text(text))) => {
                let Ok(frame) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                // A command about one teammate waits behind that teammate's
                // earlier commands and nobody else's: an agent that never
                // finishes starting holds up its own lane, not the desk.
                if let Some(teammate) = lane_of(&frame) {
                    let lane = lanes.entry(teammate).or_insert_with(|| {
                        open_lane(
                            seat,
                            log.clone(),
                            room.clone(),
                            sender.clone(),
                            phone.clone(),
                        )
                    });
                    let _ = lane.send(frame);
                    continue;
                }
                let answer = scoped(
                    seat,
                    &sender,
                    answer(
                        &frame,
                        seat,
                        &log,
                        &room,
                        &sender,
                        &mut subscriptions,
                        phone.as_ref(),
                    ),
                );
                if seat.is_remote() {
                    // Revocation cannot wait for a driver or computer to answer.
                    // Drop this socket's pending call, not the cleanup below.
                    // This is not rollback: persisted choices and dispatched
                    // effects survive; an interrupted mobile prompt keeps its
                    // durable unknown receipt. Desk mutations still run to completion.
                    tokio::select! {
                        biased;
                        () = cancel.cancelled() => break Ok(()),
                        () = answer => {}
                    }
                } else {
                    answer.await;
                }
            }
            Some(Ok(_)) => {}
        }
    };

    cancel.cancel();
    if let Some(id) = sender
        .pairing
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        && let Some(remote) = room.remote()
    {
        let _ = remote.cancel_pairing(&id);
    }
    for handle in subscriptions.values() {
        handle.abort();
    }
    writer.abort();
    for handle in subscriptions.into_values() {
        let _ = handle.await;
    }
    let _ = writer.await;
    result
}

/// The teammate a command frame is about, when it names one: by `personaId`,
/// by a thread (a DM is its teammate's), by a side thread, or as the record a
/// `persona.*` command changes. Subscriptions are never on a lane; they wait
/// on nothing.
fn lane_of(frame: &Value) -> Option<String> {
    let cmd = frame.get("cmd")?.as_str()?;
    let params = frame.get("params")?;
    let named = |field: &str| params.get(field).and_then(Value::as_str);
    if let Some(persona_id) = named("personaId") {
        return Some(persona_id.to_owned());
    }
    if let Some(thread) = params.get("thread") {
        let kind = thread.get("kind")?.as_str()?;
        let key = thread.get("key")?.as_str()?;
        return Some(if kind == "dm" {
            key.to_owned()
        } else {
            format!("{kind}:{key}")
        });
    }
    if let Some(side_id) = named("sideId") {
        return Some(format!("side:{side_id}"));
    }
    if cmd.starts_with("persona.") {
        return named("id").map(str::to_owned);
    }
    None
}

/// One teammate's commands from one socket, answered in the order they came.
/// The lane ends when the socket's loop drops its sender: a desk finishes what
/// it was asked, and a revoked companion stops at once.
fn open_lane(
    seat: Seat,
    log: Log,
    room: Arc<dyn RoomHandle>,
    sender: Outbox,
    phone: Option<crate::remote::Phone>,
) -> mpsc::UnboundedSender<Value> {
    let (lane, mut frames) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        while let Some(frame) = frames.recv().await {
            let Some(id) = frame.get("id").and_then(Value::as_i64) else {
                continue;
            };
            let answered = scoped(
                seat,
                &sender,
                command(id, &frame, seat, &log, &room, &sender, phone.as_ref()),
            );
            if seat.is_remote() {
                tokio::select! {
                    biased;
                    () = sender.cancel.cancelled() => break,
                    () = answered => {}
                }
            } else {
                answered.await;
            }
        }
    });
    lane
}

/// A frame's work, told who sent it: the person at this computer in the
/// desktop app, or away from it on a paired device, and whether that client
/// reads threads.
fn scoped<F: std::future::Future<Output = ()>>(
    seat: Seat,
    sender: &Outbox,
    work: F,
) -> impl std::future::Future<Output = ()> {
    let client = if seat.is_remote() {
        crate::contract::Client::Phone
    } else {
        crate::contract::Client::Desktop
    };
    commands::PROMPT_CLIENT.scope(client, commands::THREADS2.scope(sender.threads2(), work))
}

/// One frame in, its answer queued. A frame with no id is nobody's question,
/// so there is nowhere to put an answer and it is dropped.
pub(super) async fn answer(
    frame: &Value,
    seat: Seat,
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    sender: &Outbox,
    subscriptions: &mut HashMap<i64, JoinHandle<()>>,
    phone: Option<&crate::remote::Phone>,
) {
    let Some(id) = frame.get("id").and_then(Value::as_i64) else {
        return;
    };

    if frame.get("cmd").is_some() {
        command(id, frame, seat, log, room, sender, phone).await;
        return;
    }
    if let Some(target) = frame.get("sub") {
        // A subscription that opened answered itself, on its way past the
        // acknowledgement; only a refusal is left to say here.
        let window = frame
            .get("window")
            .and_then(Value::as_u64)
            .map(|window| (window as usize).clamp(1, PHONE_WINDOW));
        if let Err(refused) = subscribe(target, window, seat, log, room, sender, subscriptions, id)
        {
            decline(sender, id, refused);
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

/// A command frame's answer, queued on the socket's outbox.
async fn command(
    id: i64,
    frame: &Value,
    seat: Seat,
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    sender: &Outbox,
    phone: Option<&crate::remote::Phone>,
) {
    match read_command(frame) {
        Ok(command) if !seat.permits(&command) => {
            decline(
                sender,
                id,
                Refused::because(FORBIDDEN, "That seat may not run this command."),
            );
        }
        // A card waits on somebody else's server, and the read loop or a lane answers one
        // frame before it reads the next. Replies are matched by id, so
        // this one may arrive after the commands that followed it.
        Ok(Command::LinkPreview { url }) => {
            let sender = sender.clone();
            tokio::spawn(async move {
                let card = crate::link_preview::preview(&url).await;
                reply(&sender, id, Ok(json!(card)));
            });
        }
        // Making or ending a link waits on the other desk, for seconds when
        // it is far or away; the window's other commands do not wait behind it.
        Ok(command @ (Command::DesksLink { .. } | Command::DesksUnlink { .. })) => {
            let (sender, log, room) = (sender.clone(), log.clone(), Arc::clone(room));
            tokio::spawn(async move {
                let result = commands::run(command, &log, &room).await;
                reply(&sender, id, result);
            });
        }
        // A picture is read from disk, and a conversation full of them
        // asks for many at once; none of them may hold up the commands
        // behind it, such as the thread list of the teammate just opened.
        Ok(command @ (Command::FileRead { .. } | Command::AvatarRead { .. })) => {
            let (sender, log, room) = (sender.clone(), log.clone(), Arc::clone(room));
            tokio::spawn(async move {
                let result = commands::run(command, &log, &room).await;
                reply(&sender, id, result);
            });
        }
        Ok(command) => {
            // `teammate.tools` answers JSON null when there is no ledger,
            // and that null is a value, not a void — collapsing it would
            // make a missing ledger look like delete or stop.
            let keep_null = matches!(
                command,
                Command::TeammateTools { .. }
                    | Command::RemotePairing {
                        id: Some(_),
                        cancel: false,
                        ..
                    }
            );
            let result = match (&command, phone) {
                (
                    Command::FilesUploadStart(_)
                    | Command::FilesUploadChunk { .. }
                    | Command::FilesUploadFinish { .. }
                    | Command::FilesUploadCancel { .. },
                    _,
                ) => {
                    files::upload(
                        command,
                        log.root().to_path_buf(),
                        sender.uploads.clone(),
                        sender.cancel.clone(),
                    )
                    .await
                }
                (
                    Command::RemotePairing {
                        id: None,
                        cancel: false,
                        ..
                    },
                    _,
                ) => {
                    let result = commands::run(command, log, room).await;
                    if let Ok(value) = &result
                        && let Some(id) = value.get("id").and_then(Value::as_str)
                    {
                        *sender
                            .pairing
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Some(id.to_owned());
                    }
                    result
                }
                (
                    Command::AgentAuthStart {
                        persona_id,
                        method_id,
                    },
                    _,
                ) => room
                    .agent_auth_start(persona_id, method_id, sender.cancel.clone())
                    .await
                    .map(|id| {
                        sender
                            .auth_attempts
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .insert(persona_id.clone(), id.clone());
                        json!({"id": id})
                    }),
                (
                    Command::AgentAuthPoll {
                        persona_id,
                        id: attempt_id,
                    }
                    | Command::AgentAuthInput {
                        persona_id,
                        id: attempt_id,
                        ..
                    }
                    | Command::AgentAuthCancel {
                        persona_id,
                        id: attempt_id,
                    },
                    _,
                ) => {
                    let owned = sender
                        .auth_attempts
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .get(persona_id)
                        .is_some_and(|owned| owned == attempt_id);
                    if !owned || sender.cancel.is_cancelled() {
                        Err("That sign-in belongs to another desktop connection.".into())
                    } else {
                        match &command {
                            Command::AgentAuthPoll { .. } => room
                                .agent_auth_poll(persona_id, attempt_id)
                                .and_then(|status| {
                                    serde_json::to_value(status)
                                        .map_err(|_| "Could not read sign-in status.".into())
                                }),
                            Command::AgentAuthInput { input, .. } => room
                                .agent_auth_input(persona_id, attempt_id, input)
                                .map(|()| Value::Null),
                            Command::AgentAuthCancel { .. } => room
                                .agent_auth_cancel(persona_id, attempt_id)
                                .map(|()| Value::Null),
                            _ => unreachable!(
                                "the arm above matched only the three sign-in commands"
                            ),
                        }
                    }
                }
                (
                    Command::MobilePrompt {
                        operation_id,
                        persona_id,
                        text,
                        attachment_ids,
                        reply_to,
                        thread,
                    },
                    Some(phone),
                ) => {
                    phone
                        .prompt(
                            operation_id,
                            persona_id,
                            text,
                            attachment_ids,
                            reply_to.as_deref(),
                            thread.as_ref(),
                        )
                        .await
                }
                (Command::MobileAttachment { upload }, Some(phone)) => phone.upload(upload).await,
                // A companion is listed what it could read: no pairs, which
                // `peers.list` would not show it either, and no calls.
                (Command::ThreadList { persona_id }, _) => {
                    threads::list(log, room, persona_id.as_deref(), seat == Seat::Phone)
                }
                (Command::ClientHello { capabilities }, _) => {
                    sender.threads2.store(
                        capabilities.iter().any(|name| name == THREADS2),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    sender.lean.store(
                        seat.is_remote() && capabilities.iter().any(|name| name == LEAN),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    Ok(json!({ "capabilities": seat.capabilities_for(room.as_ref()) }))
                }
                (Command::RemoteDevices {}, Some(phone)) => Ok(json!(phone.devices())),
                // Seat::Phone does not permit it, so this is an owner's.
                (Command::RemoteBecomeRelay {}, Some(phone)) => phone.become_relay(),
                (Command::MobilePushRegister { token, platform }, Some(phone)) => {
                    phone.register_push(token.clone(), platform.clone())
                }
                // A harness may serve anything it likes as a config, and a
                // phone cannot know which of them decide what a teammate
                // may do. Only the one the contract names is allowed.
                (
                    Command::SessionSetConfig {
                        persona_id,
                        config_id,
                        ..
                    },
                    Some(_),
                ) if seat == Seat::Phone && !effort_config(room, persona_id, config_id) => {
                    Err("Change that on your desktop.".to_string())
                }
                // A page of older lines is cut for the phone the way its
                // snapshot is: long text shortened, frames made phone-sized.
                (Command::TapePage { .. } | Command::ThreadPage { .. }, Some(_))
                    if seat == Seat::Phone || sender.lean() =>
                {
                    commands::run(command, log, room).await.map(|mut page| {
                        if let Some(events) = page.get_mut("events").and_then(Value::as_array_mut) {
                            *events = phone_budget(std::mem::take(events));
                        }
                        page
                    })
                }
                // Why a harness is unavailable can name the machine's
                // accounts and folders. That is the owner's to read; a
                // companion sees that it is unavailable, not why.
                (Command::BackendsList {}, _) if seat == Seat::Phone => {
                    commands::run(command, log, room).await.map(|mut list| {
                        for backend in list.as_array_mut().into_iter().flatten() {
                            if let Some(reason) = backend.get_mut("unavailable")
                                && !PLAIN_REASONS.contains(&reason.as_str().unwrap_or_default())
                            {
                                *reason = Value::from(PRIVATE_REASON);
                            }
                        }
                        list
                    })
                }
                // The viewer is a loopback URL with the computer's bearer
                // in its fragment; it never leaves this machine.
                (Command::ComputerStatus { .. }, Some(_)) => {
                    commands::run(command, log, room).await.map(|mut status| {
                        if let Some(fields) = status.as_object_mut() {
                            fields.remove("viewer");
                        }
                        status
                    })
                }
                (Command::VoiceCallStart { call_id, .. }, _) => {
                    let call_id = call_id.clone();
                    let result = commands::run(command, log, room).await;
                    if result.is_ok()
                        && let Some(voice) = room.voice()
                    {
                        voice.bind_connection(call_id, sender.cancel.clone());
                    }
                    result
                }
                _ => commands::run(command, log, room).await,
            };
            reply_to(sender, id, result, keep_null);
        }
        Err(error) => reply(sender, id, Err(error)),
    }
}

/// The command frame's `cmd` and `params` are this enum's tag and content, so
/// reading one is putting those two back together without the id.
pub(super) fn read_command(frame: &Value) -> Result<Command, String> {
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

pub(super) fn reply(sender: &Outbox, id: i64, result: Result<Value, String>) {
    reply_to(sender, id, result, false);
}

fn reply_to(sender: &Outbox, id: i64, result: Result<Value, String>, keep_null: bool) {
    let frame = match result {
        Ok(Value::Null) if !keep_null => json!({ "id": id, "ok": true }),
        Ok(result) => json!({ "id": id, "ok": true, "result": result }),
        Err(error) => json!({ "id": id, "ok": false, "error": error }),
    };
    let _ = sender.send(frame.to_string());
}

/// What a phone's hello says this desk can do beyond protocol 1, so that a
/// phone asks only for what the desk it reached understands. `schedules`: the
/// `{"schedules": "<personaId>"}` subscription. A desk from before this list
/// sends none, and a phone must read that as "not here", never as "nothing".
/// `threads`: the `{"thread": "<key>"}` subscription, to read two
/// teammates' conversation. `runs`: the `{"run": "<runId>"}` subscription,
/// to read a subagent's run. `personaEdit`: `mobile.persona_update` and
/// `persona.delete`, renaming, re-aiming and removing a teammate.
/// `personaAccess`, sent to the owner phone only: `mobile.persona_access`,
/// a teammate's reach or mode and its background work.
/// `personaComputer`, also owner-only: `mobile.persona_computer`, enabling
/// a computer and choosing resource limits. `computer.capacity` is read-only
/// and available to every phone seat. `turnRetry`: `session.retry`, trying
/// a failed turn again, so a phone shows Try again only where it works.
pub(crate) const PHONE_CAPABILITIES: &[&str] = &[
    "personaCreate",
    "personaEdit",
    "schedules",
    "threads",
    "runs",
    "threads2",
    "lean",
    "thumbnails",
    "turnRetry",
];

/// The capability a client names in `client.hello` to be sent `link` events
/// and `ThreadDelta`s, and the one a hello advertises that the core can.
const THREADS2: &str = "threads2";
/// The capability a phone names in `client.hello` to be sent phone-sized
/// frames whatever its role, and that a remote seat's hello advertises:
/// see [`Outbox::lean`]. With it, a subscription may also name a `window`.
const LEAN: &str = "lean";
/// How long a lean phone may say nothing before its link is taken as dead.
const LEAN_IDLE: std::time::Duration = std::time::Duration::from_secs(90);

/// The seat may not do this, whoever asks and whatever the room holds.
pub(super) const FORBIDDEN: &str = "forbidden";
/// A room that keeps no links answers a linked desk with this.
pub(super) const NOT_LINKED: &str = "This desk is not linked with that one.";
/// The command or subscription names a teammate the room does not hold.
pub(super) const UNKNOWN_TEAMMATE: &str = "unknown_teammate";
/// The room could not be read, so there is no answer to give — which is not
/// the same as an empty one.
pub(super) const UNREADABLE: &str = "unreadable";

/// A refusal, and, when a client should act on why rather than show the
/// sentence, the code that says why.
pub(super) struct Refused {
    error: String,
    code: Option<&'static str>,
}

impl Refused {
    pub(super) fn because(code: &'static str, error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            code: Some(code),
        }
    }
}

impl From<String> for Refused {
    fn from(error: String) -> Self {
        Self { error, code: None }
    }
}

/// `{"id": n, "ok": false, "error": …}`, with `code` beside the sentence
/// when the refusal has one.
fn decline(sender: &Outbox, id: i64, refused: Refused) {
    let mut frame = json!({ "id": id, "ok": false, "error": refused.error });
    if let Some(code) = refused.code {
        frame["code"] = json!(code);
    }
    let _ = sender.send(frame.to_string());
}

/// A linked desk's frame (ADR 0003), answered under [`Seat::Link`]: a ping
/// and the `federation.*` commands, held to the same [`Seat::permits`] as
/// every other seat. Anything else, a subscription included, is forbidden.
/// `desk_id` is the desk the channel authenticated, never one a frame names.
/// A frame with no id is nobody's question and gets no answer.
pub(crate) async fn federation(
    frame: &Value,
    desk_id: &str,
    room: &Arc<dyn RoomHandle>,
) -> Option<Value> {
    let id = frame.get("id").and_then(Value::as_i64)?;
    let forbidden = || {
        json!({"id": id, "ok": false, "code": FORBIDDEN,
            "error": "A linked desk may only publish its teammates and deliver messages."})
    };
    if frame.get("cmd").is_none() {
        return Some(forbidden());
    }
    let command = match read_command(frame) {
        Ok(command) => command,
        Err(error) => return Some(json!({"id": id, "ok": false, "error": error})),
    };
    if !Seat::Link.permits(&command) {
        return Some(forbidden());
    }
    let result = match command {
        Command::FederationRoster { teammates } => room
            .federation_roster(desk_id, teammates)
            .map(|()| Value::Null),
        Command::FederationDeliver { delivery } => room.federation_deliver(desk_id, delivery).await,
        Command::FederationUnlink {} => room.unlinked(desk_id).await.map(|()| Value::Null),
        _ => Ok(Value::Null),
    };
    Some(match result {
        Ok(Value::Null) => json!({"id": id, "ok": true}),
        Ok(result) => json!({"id": id, "ok": true, "result": result}),
        Err(error) => json!({"id": id, "ok": false, "error": error}),
    })
}
