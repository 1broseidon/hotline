//! One seated socket: its outbox, its lanes and the commands it answers.

use super::phone::{effort_config, phone_budget};
use super::queue;
use super::seat::{PLAIN_REASONS, PRIVATE_REASON, Seat};
use super::subscribe::subscribe;
use super::transport::AbortiveClose;
use super::{Agents, RoomHandle, commands, files, threads};
use crate::contract::Command;
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
    pub(super) call_owner: String,
    pub(super) pairing_cancel: tokio_util::sync::CancellationToken,
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
    /// The client retains omitted roster catalogs between rows.
    pub(super) roster_options: Arc<std::sync::atomic::AtomicBool>,
}
#[derive(Clone)]
pub(super) enum Outgoing {
    Desk(mpsc::UnboundedSender<String>),
    Phone(queue::Sender),
}
enum IncomingOutput {
    Desk(mpsc::UnboundedReceiver<String>),
    Phone(queue::Receiver),
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
    pub(super) fn roster_options(&self) -> bool {
        self.roster_options
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(super) fn threads2(&self) -> bool {
        self.threads2.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(super) fn lean(&self) -> bool {
        self.lean.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Live words or tool activity may be shed on a slow link. Their durable
    /// record remains in the log; chat, cards and replies retain their place.
    pub(super) fn send_droppable(&self, text: String) -> Result<(), ()> {
        match &self.sender {
            Outgoing::Phone(sender) => {
                self.sent(sender.send(queue::Frame::Droppable(text), self.max))
            }
            Outgoing::Desk(_) => self.send(text),
        }
    }

    pub(super) fn send_roster(&self, sub: i64, row: Value) -> Result<(), ()> {
        match &self.sender {
            Outgoing::Phone(sender) => {
                self.sent(sender.send(queue::Frame::Roster { sub, row }, self.max))
            }
            Outgoing::Desk(_) => self.send(json!({ "sub": sub, "event": row }).to_string()),
        }
    }

    pub(super) fn send(&self, text: String) -> Result<(), ()> {
        let oversized = text.len() > self.max;
        let failed = oversized
            || match &self.sender {
                Outgoing::Desk(sender) => sender.send(text).is_err(),
                Outgoing::Phone(sender) => {
                    sender.send(queue::Frame::Required(text), self.max).is_err()
                }
            };
        self.sent(if failed { Err(()) } else { Ok(()) })
    }

    fn sent(&self, result: Result<(), ()>) -> Result<(), ()> {
        if result.is_err() {
            self.cancel.cancel();
        }
        result
    }
}

/// Only the authenticated sealed transport calls this: the hello itself is encrypted.
pub(crate) async fn seated_phone_v2<S>(
    mut socket: S,
    desk: Agents,
    phone: crate::remote::Phone,
    role: crate::remote::WireRole,
    desktop_id: &str,
) -> Result<(), Error>
where
    S: Stream<Item = Result<Message, Error>>
        + Sink<Message, Error = Error>
        + AbortiveClose
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
        "capabilities": Seat::for_phone(role).capabilities_for(&desk.parts),
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
    seated_inner(socket, Seat::for_phone(role), desk, Some(phone)).await
}

pub(super) async fn seated<S>(
    socket: WebSocketStream<S>,
    seat: Seat,
    desk: Agents,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + AbortiveClose + Unpin + Send + 'static,
{
    seated_inner(socket, seat, desk, None).await
}

async fn seated_inner<S>(
    socket: S,
    seat: Seat,
    desk: Agents,
    phone: Option<crate::remote::Phone>,
) -> Result<(), Error>
where
    S: Stream<Item = Result<Message, Error>>
        + Sink<Message, Error = Error>
        + AbortiveClose
        + Unpin
        + Send
        + 'static,
{
    let (mut sink, mut incoming) = socket.split();
    let cancel = phone.as_ref().map(|p| p.cancel.clone()).unwrap_or_default();
    let (sender, mut outbox) = if seat.is_remote() {
        let (tx, rx) = queue::channel();
        (Outgoing::Phone(tx), IncomingOutput::Phone(rx))
    } else {
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        (Outgoing::Desk(tx), IncomingOutput::Desk(rx))
    };
    let sender = Outbox {
        call_owner: phone
            .as_ref()
            .map_or_else(|| "desk".into(), crate::remote::Phone::call_owner),
        pairing_cancel: phone
            .as_ref()
            .map(crate::remote::Phone::pairing_cancel)
            .unwrap_or_default(),
        auth_attempts: Arc::default(),
        pairing: Arc::default(),
        uploads: Arc::default(),
        threads2: Arc::default(),
        lean: Arc::default(),
        roster_options: Arc::default(),
        sender,
        cancel: cancel.clone(),
        max: match seat {
            Seat::Phone | Seat::Link => 1_048_576,
            Seat::Owner => 32 * 1024 * 1024,
            Seat::Desk => usize::MAX,
        },
    };
    // The seat owns both halves even when its future is dropped. A spawned
    // writer can outlive that future and the admission permit held by its caller.
    let _cancel_on_drop = cancel.clone().drop_guard();
    let ordered = open_ordered(seat, desk.clone(), sender.clone(), phone.clone());
    let mut lanes: HashMap<String, mpsc::UnboundedSender<Value>> = HashMap::new();
    let (result, stalled) = {
        let writer = async {
            while let Some(text) = outbox.recv().await {
                // A peer that stopped reading without closing (a phone that left
                // the network) takes no more bytes, and TCP may keep the socket
                // up for many minutes before it says so. A frame it has not taken
                // in this long ends the seat: a queue that never fills, because
                // the seat is quiet, cannot end it, nor can a silence timer the
                // client never asked for.
                let sent = tokio::time::timeout(WRITE_STALL, sink.send(Message::text(text))).await;
                if !matches!(sent, Ok(Ok(()))) {
                    return sent.is_err();
                }
            }
            false
        };
        let reader = async {
            loop {
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
                                open_lane(seat, desk.clone(), sender.clone(), phone.clone())
                            });
                            let _ = lane.send(frame);
                            continue;
                        }
                        if frame["cmd"] == "client.hello" {
                            // Negotiation changes how the very next frame is read,
                            // including whether the silence deadline applies.
                            if let Some(id) = frame["id"].as_i64() {
                                command(id, &frame, seat, &desk, &sender, phone.as_ref()).await;
                            }
                        } else if independent(&frame) {
                            let (desk, sender, phone) =
                                (desk.clone(), sender.clone(), phone.clone());
                            tokio::spawn(async move {
                                run_command(&frame, seat, &desk, &sender, phone.as_ref()).await;
                            });
                        } else {
                            let _ = ordered.send(frame);
                        }
                    }
                    Some(Ok(_)) => {}
                }
            }
        };
        tokio::select! {
            biased;
            () = cancel.cancelled() => (Ok(()), false),
            stalled = writer => (Ok(()), stalled),
            result = reader => (result, false),
        }
    };

    cancel.cancel();
    let socket = sink
        .reunite(incoming)
        .expect("the reader and writer came from the same seated socket");
    // Close before any other cleanup or the caller's admission release. Normal
    // closes still drain final replies; only a write timeout discards them.
    let closed = if stalled {
        socket.abortive_close()
    } else {
        drop(socket);
        Ok(())
    };
    if let Some(id) = sender
        .pairing
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        && let Some(remote) = desk.parts.remote.upgrade()
    {
        let _ = remote.cancel_pairing(&id);
    }
    closed?;
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
    desk: Agents,
    sender: Outbox,
    phone: Option<crate::remote::Phone>,
) -> mpsc::UnboundedSender<Value> {
    let (lane, mut frames) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        while let Some(frame) = frames.recv().await {
            if seat.is_remote() && sender.cancel.is_cancelled() {
                break;
            }
            run_command(&frame, seat, &desk, &sender, phone.as_ref()).await;
        }
    });
    lane
}

/// Global mutations and subscription controls retain their arrival order.
/// This worker owns the subscriptions so cancellation can remove them even
/// while a command is waiting on a driver. Accepted desk writes still finish.
fn open_ordered(
    seat: Seat,
    desk: Agents,
    sender: Outbox,
    phone: Option<crate::remote::Phone>,
) -> mpsc::UnboundedSender<Value> {
    let (tx, mut frames) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        let mut subscriptions: HashMap<i64, JoinHandle<()>> = HashMap::new();
        let mut closed = false;
        loop {
            let frame = tokio::select! {
                biased;
                () = sender.cancel.cancelled(), if !closed => {
                    closed = true;
                    for (_, handle) in subscriptions.drain() {
                        handle.abort();
                    }
                    if seat.is_remote() { break; }
                    continue;
                }
                frame = frames.recv() => match frame {
                    Some(frame) => frame,
                    None => break,
                },
            };
            if frame.get("cmd").is_some() {
                let work = run_command(&frame, seat, &desk, &sender, phone.as_ref());
                tokio::pin!(work);
                tokio::select! {
                    biased;
                    () = sender.cancel.cancelled(), if !closed => {
                        closed = true;
                        for (_, handle) in subscriptions.drain() {
                            handle.abort();
                        }
                        if seat.is_remote() { break; }
                        work.await;
                    }
                    () = &mut work => {}
                }
            } else if !closed {
                answer(
                    &frame,
                    seat,
                    &desk,
                    &sender,
                    &mut subscriptions,
                    phone.as_ref(),
                )
                .await;
            }
        }
        for (_, handle) in subscriptions {
            handle.abort();
        }
    });
    tx
}

/// These commands do not order room writes or subscription controls. A read
/// sees committed state when it runs; callers needing a preceding write must
/// await that write's reply (or the ordered `ping`) before asking.
fn independent(frame: &Value) -> bool {
    matches!(
        read_command(frame),
        Ok(Command::LinkPreview { .. }
            | Command::DesksLink { .. }
            | Command::DesksUnlink { .. }
            | Command::FileRead { .. }
            | Command::AvatarRead { .. }
            | Command::VoiceStatus { .. }
            | Command::VoiceModels {}
            | Command::VoiceTranscribe { .. }
            | Command::ImagesStatus {}
            | Command::WebsearchStatus {}
            | Command::CapabilitiesOptions {}
            | Command::RemoteStatus {}
            | Command::RemoteRelays {}
            | Command::RemoteDevices {}
            | Command::DesksLinks {}
            | Command::CredentialList {}
            | Command::SkillsList { persona_id: None }
            | Command::ProvidersList {}
            | Command::ModelsList {}
            | Command::ModelsCatalog { .. }
            | Command::ModelsEfforts { .. }
            | Command::SearchAll { .. }
            | Command::ScheduleList {}
            | Command::BackendsList {}
            | Command::ComputerRuntimes {}
            | Command::ComputerCapacity {}
            | Command::ComputerReleases {}
            | Command::ComputerReleasesCheck {}
            | Command::ComputerBrowsersList {}
            | Command::ComputerCookieSettings {}
            | Command::ComputerCookieSites {}
            | Command::SecretsList {}
            | Command::ThreadList { persona_id: None }
            | Command::FilesBrowse { .. }
            | Command::FilesDownload { .. })
    )
}

async fn run_command(
    frame: &Value,
    seat: Seat,
    desk: &Agents,
    sender: &Outbox,
    phone: Option<&crate::remote::Phone>,
) {
    let Some(id) = frame.get("id").and_then(Value::as_i64) else {
        return;
    };
    let work = scoped(seat, sender, command(id, frame, seat, desk, sender, phone));
    if seat.is_remote() {
        // A revoked or disconnected remote cannot leave work running. This
        // is cancellation, not rollback of effects already dispatched.
        tokio::select! {
            biased;
            () = sender.cancel.cancelled() => {},
            () = work => {},
        }
    } else {
        work.await;
    }
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
    desk: &Agents,
    sender: &Outbox,
    subscriptions: &mut HashMap<i64, JoinHandle<()>>,
    phone: Option<&crate::remote::Phone>,
) {
    let Some(id) = frame.get("id").and_then(Value::as_i64) else {
        return;
    };

    if frame.get("cmd").is_some() {
        command(id, frame, seat, desk, sender, phone).await;
        return;
    }
    if frame.get("sub").is_some() {
        // A subscription that opened answered itself, on its way past the
        // acknowledgement; only a refusal is left to say here.
        if let Err(refused) = subscribe(id, frame, seat, desk, sender, subscriptions) {
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
    desk: &Agents,
    sender: &Outbox,
    phone: Option<&crate::remote::Phone>,
) {
    let Agents { log, room, parts } = desk;
    match read_command(frame) {
        Ok(command) if !seat.permits(&command) => {
            decline(
                sender,
                id,
                Refused::because(FORBIDDEN, "That seat may not run this command."),
            );
        }
        Ok(Command::LinkPreview { url }) => {
            let card = crate::link_preview::preview(&url).await;
            reply(sender, id, Ok(json!(card)));
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
                    let result = commands::run(command, log, room, parts).await;
                    if let Ok(value) = &result
                        && let Some(id) = value.get("id").and_then(Value::as_str)
                    {
                        let mut pairing = sender
                            .pairing
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        // Accepted desk commands may finish after disconnect.
                        // Serialize ownership with cleanup so a late invitation
                        // cannot outlive the socket that requested it.
                        if sender.cancel.is_cancelled() {
                            if let Some(remote) = parts.remote.upgrade() {
                                let _ = remote.cancel_pairing(id);
                            }
                        } else {
                            *pairing = Some(id.to_owned());
                        }
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
                    sender.roster_options.store(
                        capabilities.iter().any(|name| name == "rosterOptions"),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    sender.threads2.store(
                        capabilities.iter().any(|name| name == THREADS2),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    sender.lean.store(
                        seat.is_remote() && capabilities.iter().any(|name| name == LEAN),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    Ok(json!({ "capabilities": seat.capabilities_for(parts) }))
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
                ) if seat == Seat::Phone && !effort_config(log, room, persona_id, config_id) => {
                    Err("Change that on your desktop.".to_string())
                }
                // A page of older lines is cut for the phone the way its
                // snapshot is: long text shortened, frames made phone-sized.
                (Command::TapePage { .. } | Command::ThreadPage { .. }, Some(_))
                    if seat == Seat::Phone || sender.lean() =>
                {
                    commands::run(command, log, room, parts)
                        .await
                        .map(|mut page| {
                            if let Some(events) =
                                page.get_mut("events").and_then(Value::as_array_mut)
                            {
                                *events = phone_budget(std::mem::take(events));
                            }
                            page
                        })
                }
                // Why a harness is unavailable can name the machine's
                // accounts and folders. That is the owner's to read; a
                // companion sees that it is unavailable, not why.
                (Command::BackendsList {}, _) if seat == Seat::Phone => {
                    commands::run(command, log, room, parts)
                        .await
                        .map(|mut list| {
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
                    commands::run(command, log, room, parts)
                        .await
                        .map(|mut status| {
                            if let Some(fields) = status.as_object_mut() {
                                fields.remove("viewer");
                            }
                            status
                        })
                }
                (Command::VoiceCallStart { call_id, .. }, _) => {
                    let call_id = call_id.clone();
                    let result = commands::run(command, log, room, parts).await;
                    if result.is_ok()
                        && let Some(voice) = &parts.voice
                    {
                        let _ = voice.bind_connection(
                            call_id,
                            &sender.call_owner,
                            sender.cancel.clone(),
                            sender.pairing_cancel.clone(),
                        );
                    }
                    result
                }
                _ => commands::run(command, log, room, parts).await,
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
    serde_json::from_value(Value::Object(envelope)).map_err(|error| {
        if frame["cmd"] == "secrets.capture.answer"
            || (frame["cmd"] == "thread.answer"
                && frame["params"]["answer"]["kind"] == "credential")
        {
            "This room cannot read that credential answer. Check its fields.".to_string()
        } else {
            format!("This room cannot read that command: {error}.")
        }
    })
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
/// `agentSwitch`: `session.set_backend`, moving a teammate to another agent,
/// and `backends.list` naming each agent's last reported models; a phone
/// offers the agents in its picker only where the desk says this.
pub(crate) const PHONE_CAPABILITIES: &[&str] = &[
    "personaCreate",
    "personaEdit",
    "schedules",
    "threads",
    "runs",
    "threads2",
    "rosterOptions",
    "lean",
    "thumbnails",
    "turnRetry",
    "agentSwitch",
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
/// How long one frame may wait for the peer to take it before the seat is
/// taken as dead. A phone's largest frame crosses a slow link well within it.
pub(crate) const WRITE_STALL: std::time::Duration = std::time::Duration::from_secs(60);

/// The seat may not do this, whoever asks and whatever the room holds.
pub(super) const FORBIDDEN: &str = "forbidden";
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
