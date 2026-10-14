//! The channel between two linked desks (ADR 0003).
//!
//! The desk that made a link dials the other one's `/v2/link`, directly or
//! through its relay, with its own desk identity: the key the other desk's
//! link record names, never a device key, so a link is a grant of its own,
//! narrower than the owner pairing it was made over. Once the two Noise
//! messages have passed, the channel is symmetric. Either side sends
//! `{id, cmd, params}` and answers the other's by id; a linked desk's
//! commands are answered by `wire::federation` under `Seat::Link`, which
//! permits its roster, its messages and nothing else.
//!
//! The channel is durable by being dialed again: a dial that fails waits
//! with jittered backoff and tries again, for as long as the link exists.
//! Messages meanwhile wait in each desk's exchange queue. An authenticated
//! refusal from the other desk means it ended the link, and this side ends
//! it too.
use super::client::{Client, OpenError};
use super::server::Seat;
use super::*;
use crate::linked::{self, Call, CallError, DeskLink, LinkedTeammate, Record};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

/// How often each side looks at its own roster, and publishes it if it moved.
const ROSTER_EVERY: Duration = Duration::from_secs(2);
/// A quiet channel is pinged this often, and given up after three.
const PING_EVERY: Duration = Duration::from_secs(25);
/// The handshake, and each step of making or ending a link, is bounded.
const STEP_WITHIN: Duration = Duration::from_secs(10);
/// The largest frame either desk reads or sends over a link, as for a phone.
/// A roster of 200 rows, a whole message or a whole answer fits in it.
const FRAME_MAX: usize = 1024 * 1024;

impl Remote {
    fn link_registry(&self) -> Result<Arc<linked::Links>, String> {
        self.room
            .links()
            .ok_or_else(|| "This desk cannot link with another.".to_string())
    }

    /// Every desk this one is linked with, and how each link is doing.
    pub fn desk_links(&self) -> Result<Vec<DeskLink>, String> {
        let links = self.link_registry()?;
        Ok(linked::records(&self.log)
            .into_iter()
            .map(|record| {
                let (state, error) = links.state(&record.desk_id);
                let teammates = links.roster(&record.desk_id).unwrap_or_else(|| {
                    record
                        .teammates
                        .iter()
                        .map(|known| LinkedTeammate {
                            persona_id: known.persona_id.clone(),
                            name: known.name.clone(),
                            state: "offline".into(),
                            activity: None,
                            working_on: linked::WorkingOn {
                                title: String::new(),
                                last_turn_at: None,
                            },
                        })
                        .collect()
                });
                DeskLink {
                    desk_id: record.desk_id,
                    name: record.name,
                    dials: record.dials,
                    state,
                    error,
                    teammates,
                }
            })
            .collect())
    }

    fn desk_link(&self, desk_id: &str) -> Result<DeskLink, String> {
        self.desk_links()?
            .into_iter()
            .find(|link| link.desk_id == desk_id)
            .ok_or_else(|| "This desk is not linked with that one.".to_string())
    }

    /// Links this desk with `desk_id`, a desk this computer is paired with as
    /// an owner. The other desk records the link over that owner seat, both
    /// sides keep it as a room event, and this side dials from then on.
    pub async fn link(self: &Arc<Self>, desk_id: &str) -> Result<DeskLink, String> {
        let links = self.link_registry()?;
        if linked::record(&self.log, desk_id).is_some() {
            return self.desk_link(desk_id);
        }
        let me = self.status_desktop_id();
        if desk_id == me {
            return Err("A desk cannot link with itself.".into());
        }
        let desk = relay::paired(&self.root)
            .into_iter()
            .find(|desk| desk.desk_id == desk_id)
            .ok_or("Connect to that desk first, then link with it.")?;
        let (_, public) = self.noise_keys()?;
        let mut owner = Client::new(self.store.clone())
            .open(&desk, None)
            .await
            .map_err(|error| error.to_string())?;
        let ask = json!({"id": 1, "cmd": "desks.link_accept", "params": {
            "deskId": me, "name": desktop_name(), "key": URL_SAFE_NO_PAD.encode(public),
        }});
        let answer = tokio::time::timeout(STEP_WITHIN, async {
            owner
                .send(Message::text(ask.to_string()))
                .await
                .map_err(|_| "The desk disconnected.".to_string())?;
            loop {
                let Some(Ok(Message::Text(text))) = owner.next().await else {
                    return Err("The desk disconnected.".to_string());
                };
                let Ok(frame) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if frame["id"] == 1 {
                    return Ok(frame);
                }
            }
        })
        .await
        .map_err(|_| "The desk did not answer within ten seconds.".to_string())??;
        let _ = tokio::time::timeout(STEP_WITHIN, owner.close()).await;
        if answer["ok"] != true {
            return Err(answer["error"]
                .as_str()
                .unwrap_or("That desk would not link.")
                .to_string());
        }
        if answer["result"]["deskId"] != desk_id {
            return Err("That desk answered as another desk.".into());
        }
        let name = Some(linked::plain(
            answer["result"]["name"].as_str().unwrap_or(""),
            80,
        ))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| linked::plain(&desk.name, 80));
        linked::save(
            &self.log,
            &Record {
                desk_id: desk_id.to_string(),
                name,
                dials: true,
                key: None,
                teammates: Vec::new(),
            },
        )?;
        // A link made again after an unlink starts from nothing.
        links.forget(desk_id);
        self.dial(desk_id.to_string());
        self.desk_link(desk_id)
    }

    /// The other half of [`Self::link`]: trust `key`, the other desk's own
    /// identity, for `/v2/link`. Asked over an owner seat here, so it is the
    /// owner's decision on this desk too.
    pub async fn accept_link(&self, desk_id: &str, name: &str, key: &str) -> Result<Value, String> {
        let name = linked::plain(name, 80);
        if !linked::valid_desk_id(desk_id) || desk_id == self.status_desktop_id() {
            return Err("That is not another desk's id.".into());
        }
        if name.is_empty() || name.chars().count() > 80 {
            return Err("Name the desk in 1–80 characters.".into());
        }
        if URL_SAFE_NO_PAD
            .decode(key)
            .map_or(true, |key| key.len() != 32)
        {
            return Err("That is not a desk identity key.".into());
        }
        if linked::record(&self.log, desk_id).is_some_and(|record| record.dials) {
            return Err("This desk already links with that one from here.".into());
        }
        // A link accepted again — the other desk ended it while this one
        // could not hear, and linked again — ends the old one first, with
        // everything it held, so the new one starts from nothing.
        if linked::record(&self.log, desk_id).is_some() {
            self.room.unlinked(desk_id).await?;
        }
        // Nor does anything a frame of an earlier link left behind here.
        if let Ok(links) = self.link_registry() {
            links.forget(desk_id);
        }
        linked::save(
            &self.log,
            &Record {
                desk_id: desk_id.to_string(),
                name,
                dials: false,
                key: Some(key.to_string()),
                teammates: Vec::new(),
            },
        )?;
        Ok(json!({"deskId": self.status_desktop_id(), "name": desktop_name()}))
    }

    /// Ends a link: the other desk is told when it can be, and this one
    /// ends it now whatever the other says.
    pub async fn unlink(self: &Arc<Self>, desk_id: &str) -> Result<(), String> {
        let links = self.link_registry()?;
        let record =
            linked::record(&self.log, desk_id).ok_or("This desk is not linked with that one.")?;
        if links.is_open(desk_id) {
            let _ = tokio::time::timeout(
                STEP_WITHIN,
                links.call(desk_id, "federation.unlink", json!({})),
            )
            .await;
        } else if record.dials {
            // The other desk only hears from this one by being dialed, so
            // one last dial says goodbye. If it is away it will refuse this
            // desk's key when it next answers, and is told then: no, it
            // keeps a link that never dials again, and the person ends it
            // there too.
            let _ = tokio::time::timeout(STEP_WITHIN * 2, self.say_goodbye(desk_id)).await;
        }
        self.room.unlinked(desk_id).await
    }

    async fn say_goodbye(&self, desk_id: &str) -> Result<(), OpenError> {
        let mut channel = self.open_link(desk_id).await?;
        let goodbye = json!({"id": 1, "cmd": "federation.unlink", "params": {}});
        channel
            .send(Message::text(goodbye.to_string()))
            .await
            .map_err(|_| OpenError::Unreachable("The desk disconnected.".into()))?;
        while let Some(Ok(message)) = channel.next().await {
            // Its answer, not its own first command, which may share the id.
            if let Message::Text(text) = message
                && serde_json::from_str::<Value>(&text)
                    .is_ok_and(|frame| frame["id"] == 1 && frame.get("ok").is_some())
            {
                break;
            }
        }
        let _ = channel.close().await;
        Ok(())
    }

    /// Dials every link this desk made. Called once the desk is up.
    pub(super) fn restore_links(self: &Arc<Self>) {
        for record in linked::records(&self.log) {
            if record.dials {
                self.dial(record.desk_id);
            }
        }
    }

    async fn open_link(&self, desk_id: &str) -> Result<client::Connection, OpenError> {
        let desk = relay::paired(&self.root)
            .into_iter()
            .find(|desk| desk.desk_id == desk_id)
            .ok_or("This computer no longer pairs with that desk. Connect to it again.")?;
        let (private, _) = self.noise_keys()?;
        Client::new(self.store.clone())
            .link(&desk, &private, &self.status_desktop_id())
            .await
    }

    /// Keeps dialing a link this desk made, for as long as it exists.
    fn dial(self: &Arc<Self>, desk_id: String) {
        let Ok(links) = self.link_registry() else {
            return;
        };
        // One dialer per link, however many times it is asked for.
        let Some(cancel) = links.dialer(&desk_id) else {
            return;
        };
        let remote = self.clone();
        tokio::spawn(async move {
            let mut tries = 0u32;
            loop {
                if !linked::record(&remote.log, &desk_id).is_some_and(|record| record.dials) {
                    return;
                }
                let opened = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    opened = remote.open_link(&desk_id) => opened,
                };
                match opened {
                    Ok(channel) => {
                        let opened = tokio::time::Instant::now();
                        session(remote.clone(), channel, desk_id.clone(), cancel.clone()).await;
                        // A channel that stood a while starts the backoff over;
                        // one closed at once is a desk refusing, and waits longer.
                        if opened.elapsed() > STEP_WITHIN {
                            tries = 0;
                        }
                    }
                    // Only the pinned desk can say this, and it says it when
                    // its link record is gone: it ended the link.
                    Err(OpenError::Revoked) => {
                        if !cancel.is_cancelled()
                            && let Err(error) = remote.room.unlinked(&desk_id).await
                        {
                            eprintln!("[link] {desk_id} could not be ended here: {error}");
                        }
                        return;
                    }
                    Err(OpenError::Unreachable(error)) => {
                        links.closed(&desk_id, None, Some(error));
                    }
                }
                let wait = Duration::from_millis((500u64 << tries.min(6)).min(30_000));
                tries = tries.saturating_add(1);
                let jitter = 0.8 + (Uuid::new_v4().as_bytes()[0] as f64 / 255.0) * 0.4;
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(wait.mul_f64(jitter)) => {}
                }
            }
        });
    }

    /// The token a dialing desk's channel runs under, if `public` is the key
    /// this desk's link record for `desk_id` trusts.
    fn authenticate_link(&self, desk_id: &str, public: &[u8]) -> Option<CancellationToken> {
        if self.state.lock().unwrap().endpoints.is_empty() {
            return None;
        }
        let public = URL_SAFE_NO_PAD.encode(public);
        linked::record(&self.log, desk_id)
            .filter(|record| !record.dials && record.key.as_deref() == Some(public.as_str()))?;
        Some(self.room.links()?.token(desk_id).child_token())
    }
}

/// `/v2/link` on the desk being dialed: both Noise messages, then the
/// channel, for a desk whose key this desk's link record names. Any other
/// key is told so under this desk's identity, which is how the dialer learns
/// the link has ended.
pub(super) async fn answer<S>(remote: Arc<Remote>, mut socket: WebSocketStream<S>, seat: Seat)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Binding {
        purpose: String,
        desk_id: String,
    }
    let cancel = remote.state.lock().unwrap().cancel.clone();
    let deadline = tokio::time::Instant::now() + STEP_WITHIN;
    let handshake = async {
        let (private, _) = remote.noise_keys().ok()?;
        let mut noise = sealed::responder(&private).ok()?;
        let Message::Binary(first) = socket.next().await?.ok()? else {
            return None;
        };
        if first.len() > 4096 {
            return None;
        }
        let mut payload = [0u8; 4096];
        let size = noise.read_message(&first, &mut payload).ok()?;
        let binding: Binding = serde_json::from_slice(&payload[..size]).ok()?;
        if binding.purpose != "link" || !linked::valid_desk_id(&binding.desk_id) {
            return None;
        }
        let public = noise.get_remote_static()?.to_vec();
        let link = remote.authenticate_link(&binding.desk_id, &public);
        // Turning Remote off refuses every key; that is not the link ending.
        if link.is_none() && cancel.is_cancelled() {
            return None;
        }
        let answer = if link.is_some() {
            Vec::new()
        } else {
            sealed::DEVICE_REJECTED.to_vec()
        };
        let mut response = [0u8; 4096];
        let size = noise.write_message(&answer, &mut response).ok()?;
        socket
            .send(Message::Binary(response[..size].to_vec().into()))
            .await
            .ok()?;
        let link = link?;
        let state = noise.into_transport_mode().ok()?;
        // A linked desk takes an authenticated seat like a device does, under
        // a name no device can have, so the same limits bound it.
        let admitted = format!("link:{}", binding.desk_id);
        let seated = match &seat {
            Seat::Direct(slot) => {
                if !slot.authenticate(&admitted) {
                    return None;
                }
                None
            }
            Seat::Relayed { .. } => Some(remote.admission.seat(&admitted)?),
        };
        Some((
            super::channel::Channel::new(socket, state),
            binding.desk_id,
            link,
            seated,
        ))
    };
    let established = tokio::select! {
        biased;
        _ = cancel.cancelled() => return,
        _ = seat.expired(deadline) => return,
        result = handshake => result,
    };
    let Some((channel, desk_id, link, _seated)) = established else {
        return;
    };
    // Remote going off, or the relay that carried this, ends it too.
    let ended = link.clone();
    let relayed = match &seat {
        Seat::Relayed { cancel, .. } => cancel.clone(),
        Seat::Direct(_) => CancellationToken::new(),
    };
    let watch = tokio::spawn(async move {
        tokio::select! {
            _ = cancel.cancelled() => {},
            _ = relayed.cancelled() => {},
            _ = ended.cancelled() => {},
        }
        ended.cancel();
    });
    session(remote, channel, desk_id, link).await;
    watch.abort();
    drop(seat);
}

/// One open channel between two linked desks, from either end, until it
/// closes or the link ends.
async fn session<S>(
    remote: Arc<Remote>,
    channel: super::channel::Channel<S>,
    desk_id: String,
    cancel: CancellationToken,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let Some(links) = remote.room.links() else {
        return;
    };
    let mut channel = channel.limited(FRAME_MAX);
    let mut throttled = false;
    let (calls, mut waiting) = mpsc::channel::<Call>(32);
    let generation = links.opened(&desk_id, calls);
    let mut answers: HashMap<i64, oneshot::Sender<Result<Value, CallError>>> = HashMap::new();
    let mut next = 0i64;
    let mut published: Option<Vec<LinkedTeammate>> = None;
    let mut roster = tokio::time::interval(ROSTER_EVERY);
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut heard = tokio::time::Instant::now();
    // The roster goes before anything else that crosses: a message names its
    // sender, and the other desk takes only senders it has been shown.
    let mut publish = |next: &mut i64| {
        let now = remote.room.federation_roster_entries();
        if published.as_ref() == Some(&now) {
            return None;
        }
        *next += 1;
        let frame = json!({"id": *next, "cmd": "federation.roster", "params": {"teammates": now}});
        published = Some(now);
        Some(frame)
    };
    let why = loop {
        let mut frames = Vec::new();
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break None,
            _ = roster.tick() => frames.extend(publish(&mut next)),
            call = waiting.recv() => {
                let Some(call) = call else { break None };
                frames.extend(publish(&mut next));
                next += 1;
                answers.insert(next, call.answer);
                frames.push(json!({"id": next, "cmd": call.cmd, "params": call.params}));
            }
            _ = ping.tick() => {
                if heard.elapsed() > PING_EVERY * 3 {
                    break Some("The other desk stopped answering.".to_string());
                }
                next += 1;
                frames.push(json!({"id": next, "cmd": "ping"}));
            }
            message = channel.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    // A desk sending faster than its budget is read more
                    // slowly, which holds it back through the socket.
                    if let Some(wait) = links.frame_wait(&desk_id) {
                        if !throttled {
                            eprintln!(
                                "[link] {desk_id} is sending more than {} frames a second; reading it no faster",
                                linked::FRAMES_EVERY_SECOND
                            );
                            throttled = true;
                        }
                        tokio::select! {
                            _ = cancel.cancelled() => break None,
                            _ = tokio::time::sleep(wait) => {}
                        }
                    } else {
                        throttled = false;
                    }
                    heard = tokio::time::Instant::now();
                    let Ok(received) = serde_json::from_str::<Value>(&text) else { continue };
                    if received.get("ok").is_none() {
                        frames.extend(crate::wire::federation(&received, &desk_id, &remote.room).await);
                    } else if let Some(answer) = received["id"].as_i64().and_then(|id| answers.remove(&id)) {
                        let _ = answer.send(if received["ok"] == true {
                            Ok(received.get("result").cloned().unwrap_or(Value::Null))
                        } else {
                            Err(CallError::Refused(
                                received["error"].as_str().unwrap_or("The other desk refused.").to_string(),
                            ))
                        });
                    }
                }
                Some(Ok(Message::Close(_))) | None => break Some("The other desk closed the link's channel.".to_string()),
                Some(Ok(_)) => {}
                Some(Err(_)) => break Some("The link's channel failed.".to_string()),
            },
        }
        let mut stalled = false;
        for frame in frames {
            let sent =
                tokio::time::timeout(STEP_WITHIN, channel.send(Message::text(frame.to_string())))
                    .await;
            if !matches!(sent, Ok(Ok(()))) {
                stalled = true;
                break;
            }
        }
        if stalled {
            break Some("The other desk stopped reading.".to_string());
        }
    };
    links.closed(&desk_id, Some(generation), why);
    for (_, answer) in answers {
        let _ = answer.send(Err(CallError::Offline));
    }
    let _ = tokio::time::timeout(STEP_WITHIN, channel.close()).await;
}
