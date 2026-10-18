//! A relay carries sealed records between a visitor and a desk; it never
//! holds a key that could read them.
//!
//! A desk with no reachable listener dials out to a served desk it is paired
//! with, as an owner or only as a relay (ADR 0006), over a sealed control
//! socket (`/v2/relay`), and stands
//! in there under its own desk id. A visitor dials `/relay/{deskId}/v2…` on
//! the served desk exactly as it would dial the desk itself. The served desk
//! hands the desk a single-use capability over the control socket; the desk
//! claims it with sealed IK at `/v2/relay/accept`, and the relay joins the two
//! sockets message for message. The Noise handshake inside runs between the
//! visitor and the desk, so a relay can drop a session but cannot read,
//! forge or redirect one: the visitor pins the desk's key, not the relay's.
use super::client::{Client, OpenError, PairedDesk, PairedRelay};
use super::server::{Purpose, Seat, sealed_session};
use super::*;
use futures_util::{SinkExt, StreamExt};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, protocol::WebSocketConfig},
};

/// Desks one served desk will stand in for at a time.
const HOSTS_MAX: usize = 64;
/// Sessions, waiting or joined, one desk may have through the relay.
const VISITS_PER_DESK: usize = 16;
/// How long a visitor waits for its desk to dial back.
const ACCEPT_WITHIN: Duration = Duration::from_secs(10);
/// A quiet control socket is pinged this often, and given up after three.
const PING_EVERY: Duration = Duration::from_secs(25);
/// The largest Noise message either end sends (see `sealed.rs`).
const RECORD_MAX: usize = 65_535;
/// Cleanup cannot wait forever on a peer that stops reading.
const IO_WITHIN: Duration = Duration::from_secs(10);

type Socket = WebSocketStream<TokioIo<Upgraded>>;

pub(super) fn valid_desk(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub(super) fn record_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(RECORD_MAX))
        .max_frame_size(Some(RECORD_MAX))
}

/// Budgets survive registrations while visits or rate debt remain. The
/// bounded registry also prevents owner-driven desk-id churn growing memory.
const VISITS_MAX: usize = 256;
const BUDGETS_MAX: usize = 1024;
const RATE_BURST: f64 = 10.0;
const RATE_PER_SECOND: f64 = 0.5;

/// Every write to `hosts` cancels a generation before dropping it, and
/// inserts a desk's budget before the desk, so a panic between any two of
/// them leaves no live desk without a budget and no waiting socket for a
/// generation that is gone. A poisoned registry is therefore recovered.
pub(super) struct Hub {
    hosts: Mutex<Hosts>,
    visits: Arc<Semaphore>,
    controls: Arc<Semaphore>,
}
impl Default for Hub {
    fn default() -> Self {
        Self {
            hosts: Mutex::default(),
            visits: Arc::new(Semaphore::new(VISITS_MAX)),
            controls: Arc::new(Semaphore::new(HOSTS_MAX)),
        }
    }
}
#[derive(Default)]
struct Hosts {
    desks: HashMap<String, Host>,
    budgets: HashMap<String, Budget>,
    waiting: HashMap<String, Waiting>,
}
struct Budget {
    visits: Arc<Semaphore>,
    tokens: f64,
    updated: tokio::time::Instant,
}
impl Budget {
    fn refill(&mut self, now: tokio::time::Instant) {
        self.tokens = (self.tokens
            + now.duration_since(self.updated).as_secs_f64() * RATE_PER_SECOND)
            .min(RATE_BURST);
        self.updated = now;
    }
}
struct Host {
    generation: Arc<Generation>,
    notify: mpsc::Sender<String>,
}
struct Generation {
    desk: String,
    id: Uuid,
    device: String,
    cancel: CancellationToken,
}
pub(super) struct Ticket {
    generation: Arc<Generation>,
    _desk: OwnedSemaphorePermit,
    _total: OwnedSemaphorePermit,
}
struct Waiting {
    socket: Socket,
    ticket: Ticket,
    expires_at: tokio::time::Instant,
}

/// A visitor's way in: the desk it names is standing in, and has room.
pub(super) enum Visit {
    Offline,
    Busy,
    RateLimited,
    Admitted(mpsc::Sender<String>, Ticket),
}

impl Hub {
    fn register(
        &self,
        desk: &str,
        device: &str,
        notify: mpsc::Sender<String>,
        cancel: CancellationToken,
    ) -> Option<Uuid> {
        let mut hosts = lock(&self.hosts);
        if hosts.desks.len() >= HOSTS_MAX && !hosts.desks.contains_key(desk) {
            return None;
        }
        let now = tokio::time::Instant::now();
        let active: std::collections::HashSet<_> = hosts.desks.keys().cloned().collect();
        hosts.budgets.retain(|desk, budget| {
            budget.refill(now);
            active.contains(desk)
                || budget.visits.available_permits() != VISITS_PER_DESK
                || budget.tokens < RATE_BURST
        });
        if !hosts.budgets.contains_key(desk) {
            if hosts.budgets.len() >= BUDGETS_MAX {
                return None;
            }
            hosts.budgets.insert(
                desk.to_owned(),
                Budget {
                    visits: Arc::new(Semaphore::new(VISITS_PER_DESK)),
                    tokens: RATE_BURST,
                    updated: now,
                },
            );
        }
        let id = Uuid::new_v4();
        if let Some(old) = hosts.desks.remove(desk) {
            old.generation.cancel.cancel();
            hosts
                .waiting
                .retain(|_, waiting| waiting.ticket.generation.id != old.generation.id);
        }
        hosts.desks.insert(
            desk.to_owned(),
            Host {
                generation: Arc::new(Generation {
                    desk: desk.to_owned(),
                    id,
                    device: device.to_owned(),
                    cancel,
                }),
                notify,
            },
        );
        Some(id)
    }

    /// A replacement may retire its predecessor even when every control
    /// slot is occupied. Its pending TCP permit bounds the wait for cleanup.
    fn retire(&self, desk: &str) -> bool {
        let mut hosts = lock(&self.hosts);
        let Some(host) = hosts.desks.get(desk) else {
            return false;
        };
        let id = host.generation.id;
        host.generation.cancel.cancel();
        hosts
            .waiting
            .retain(|_, waiting| waiting.ticket.generation.id != id);
        true
    }

    fn unregister(&self, desk: &str, id: Uuid) {
        let mut hosts = lock(&self.hosts);
        if hosts
            .desks
            .get(desk)
            .is_some_and(|host| host.generation.id == id)
        {
            if let Some(host) = hosts.desks.remove(desk) {
                host.generation.cancel.cancel();
            }
            hosts
                .waiting
                .retain(|_, waiting| waiting.ticket.generation.id != id);
        }
    }

    pub(super) fn visit(&self, desk: &str) -> Visit {
        let mut hosts = lock(&self.hosts);
        let Some(host) = hosts.desks.get(desk) else {
            return Visit::Offline;
        };
        if host.generation.cancel.is_cancelled() {
            return Visit::Offline;
        }
        let generation = host.generation.clone();
        let notify = host.notify.clone();
        let budget = hosts.budgets.get_mut(desk).unwrap();
        budget.refill(tokio::time::Instant::now());
        if budget.tokens < 1.0 {
            return Visit::RateLimited;
        }
        let Ok(desk) = budget.visits.clone().try_acquire_owned() else {
            return Visit::Busy;
        };
        let Ok(total) = self.visits.clone().try_acquire_owned() else {
            return Visit::Busy;
        };
        budget.tokens -= 1.0;
        Visit::Admitted(
            notify,
            Ticket {
                generation,
                _desk: desk,
                _total: total,
            },
        )
    }

    fn current(hosts: &Hosts, ticket: &Ticket) -> bool {
        !ticket.generation.cancel.is_cancelled()
            && hosts
                .desks
                .get(&ticket.generation.desk)
                .is_some_and(|host| host.generation.id == ticket.generation.id)
    }

    fn wait(self: &Arc<Self>, socket: Socket, ticket: Ticket) -> Option<String> {
        let capability = super::secret();
        let expires_at = tokio::time::Instant::now() + ACCEPT_WITHIN;
        {
            let mut hosts = lock(&self.hosts);
            if !Self::current(&hosts, &ticket) {
                return None;
            }
            hosts.waiting.insert(
                capability.clone(),
                Waiting {
                    socket,
                    ticket,
                    expires_at,
                },
            );
        }
        let hub = Arc::downgrade(self);
        let unclaimed = capability.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(expires_at).await;
            if let Some(hub) = hub.upgrade() {
                lock(&hub.hosts).waiting.remove(&unclaimed);
            }
        });
        Some(capability)
    }

    fn claim(&self, capability: &str, device: &str) -> Option<Waiting> {
        let mut hosts = lock(&self.hosts);
        let waiting = hosts.waiting.get(capability)?;
        if tokio::time::Instant::now() >= waiting.expires_at
            || !Self::current(&hosts, &waiting.ticket)
        {
            hosts.waiting.remove(capability);
            return None;
        }
        if device != waiting.ticket.generation.device {
            return None;
        }
        hosts.waiting.remove(capability)
    }

    #[cfg(test)]
    pub(super) fn expire_capability(&self, capability: &str) {
        lock(&self.hosts)
            .waiting
            .get_mut(capability)
            .unwrap()
            .expires_at = tokio::time::Instant::now();
    }

    #[cfg(test)]
    pub(super) fn active_visits(&self) -> usize {
        VISITS_MAX - self.visits.available_permits()
    }

    #[cfg(test)]
    pub(super) fn counts(&self) -> (usize, usize) {
        let hosts = lock(&self.hosts);
        (hosts.desks.len(), hosts.waiting.len())
    }
}

/// `/relay/{deskId}/{rest}`: the desk and the path the visitor asked of it.
pub(super) enum Route {
    Visit { desk: String, path: String },
}
impl Route {
    pub(super) fn of(path: &str) -> Option<Self> {
        let rest = path.strip_prefix("/relay/")?;
        let (head, tail) = rest.split_once('/')?;
        if !valid_desk(head) {
            return None;
        }
        let path = format!("/{tail}");
        let known = path == "/v2" || path == "/v2/pair" || path == "/v2/link" || {
            let computer = path
                .strip_prefix("/v2/computer/")
                .and_then(|p| p.strip_suffix("/ws"));
            computer.is_some_and(|id| {
                !id.is_empty()
                    && id.len() <= 128
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
        };
        known.then(|| Self::Visit {
            desk: head.to_owned(),
            path,
        })
    }
}

/// A visitor's socket, upgraded: it waits for its desk to dial back.
pub(super) async fn arrive(
    hub: Arc<Hub>,
    socket: Socket,
    notify: mpsc::Sender<String>,
    visit: Ticket,
    path: String,
) {
    let Some(capability) = hub.wait(socket, visit) else {
        return;
    };
    let notice = json!({"type": "visit", "id": capability, "path": path}).to_string();
    if notify.try_send(notice).is_err() {
        lock(&hub.hosts).waiting.remove(&capability);
    }
}

/// A callback proves the generation's device and the relay's pinned identity
/// before either endpoint switches to the visitor's end-to-end records.
pub(super) async fn accept<S>(remote: Arc<Remote>, mut socket: WebSocketStream<S>, seat: Seat)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Claim {
        purpose: String,
        capability: String,
    }
    let cancel = lock(&remote.state).cancel.clone();
    let deadline = tokio::time::Instant::now() + ACCEPT_WITHIN;
    let claimed = async {
        let (private, _) = remote.noise_keys().ok()?;
        let mut noise = sealed::responder(&private).ok()?;
        let Message::Binary(first) = socket.next().await?.ok()? else {
            return None;
        };
        if first.len() > 4096 {
            return None;
        }
        let mut payload = [0; 4096];
        let size = noise.read_message(&first, &mut payload).ok()?;
        let claim: Claim = serde_json::from_slice(&payload[..size]).ok()?;
        if claim.purpose != "relay-accept" {
            return None;
        }
        let public = noise.get_remote_static()?;
        let phone = remote.authenticate_v2(public)?;
        if !Purpose::RelayAccept.admits(phone.role) {
            return None;
        }
        let waiting = remote.relay.claim(&claim.capability, &phone.id)?;
        if waiting.ticket.generation.cancel.is_cancelled() {
            return None;
        }
        let n = noise.write_message(&[], &mut payload).ok()?;
        let sent = tokio::select! {
            biased;
            () = waiting.ticket.generation.cancel.cancelled() => return None,
            sent = socket.send(Message::Binary(payload[..n].to_vec().into())) => sent,
        };
        sent.ok()?;
        Some(waiting)
    };
    let waiting = tokio::select! {
        biased;
        () = cancel.cancelled() => return,
        () = seat.expired(deadline) => return,
        waiting = claimed => waiting,
    };
    drop(seat);
    let Some(Waiting {
        socket: visitor,
        ticket,
        ..
    }) = waiting
    else {
        return;
    };
    let generation = ticket.generation.cancel.clone();
    tokio::select! {
        biased;
        () = cancel.cancelled() => {},
        () = join(visitor, socket, generation) => {},
    }
}

/// Records pass as they are, binary only; either end closing closes both.
async fn join<A, B>(a: WebSocketStream<A>, b: WebSocketStream<B>, cancel: CancellationToken)
where
    A: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    B: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut a_out, mut a_in) = a.split();
    let (mut b_out, mut b_in) = b.split();
    let forward = async {
        loop {
            tokio::select! {
                message = a_in.next() => match message {
                    Some(Ok(Message::Binary(bytes))) => {
                        if !matches!(tokio::time::timeout(IO_WITHIN, b_out.send(Message::Binary(bytes))).await, Ok(Ok(()))) { break }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    _ => break,
                },
                message = b_in.next() => match message {
                    Some(Ok(Message::Binary(bytes))) => {
                        if !matches!(tokio::time::timeout(IO_WITHIN, a_out.send(Message::Binary(bytes))).await, Ok(Ok(()))) { break }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    _ => break,
                },
            }
        }
    };
    tokio::select! { () = cancel.cancelled() => {}, () = forward => {} }
    let _ = tokio::join!(
        tokio::time::timeout(IO_WITHIN, a_out.close()),
        tokio::time::timeout(IO_WITHIN, b_out.close()),
    );
}

/// A desk standing in on this served desk, over its sealed control socket,
/// until it goes, its device is revoked, or another of the owner's devices
/// stands in for the same desk.
pub(super) async fn host<S>(
    remote: Arc<Remote>,
    mut control: super::channel::Channel<S>,
    phone: Phone,
    desk: String,
    seat: Seat,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    enum ControlSlot {
        Ready(OwnedSemaphorePermit),
        Replacing,
        Full,
    }
    let Seat::Direct(pending) = &seat else {
        return;
    };
    // Replaced generations keep their slot until cleanup finishes. Both
    // retirement and registration share pending eviction's lock, so an
    // expired replacement cannot cancel or replace the current generation.
    let Some(slot) =
        pending.while_pending(|| match remote.relay.controls.clone().try_acquire_owned() {
            Ok(host) => ControlSlot::Ready(host),
            Err(_) if remote.relay.retire(&desk) => ControlSlot::Replacing,
            Err(_) => ControlSlot::Full,
        })
    else {
        return;
    };
    let _host = match slot {
        ControlSlot::Ready(host) => host,
        ControlSlot::Full => return,
        ControlSlot::Replacing => tokio::select! {
            biased;
            () = phone.cancel.cancelled() => return,
            () = seat.expired(tokio::time::Instant::now() + IO_WITHIN) => return,
            host = remote.relay.controls.clone().acquire_owned() => match host {
                Ok(host) => host,
                Err(_) => return,
            },
        },
    };
    let (notify, mut notices) = mpsc::channel(VISITS_PER_DESK);
    let generation = phone.cancel.child_token();
    let Some(registered) = pending.while_pending(|| {
        remote
            .relay
            .register(&desk, &phone.id, notify, generation.clone())
    }) else {
        return;
    };
    drop(seat);
    let Some(id) = registered else {
        let refusal = json!({"type": "refused", "reason": "This relay is full."});
        let _ = tokio::time::timeout(
            IO_WITHIN,
            control.send(Message::Text(refusal.to_string().into())),
        )
        .await;
        return;
    };
    let url = remote
        .served
        .as_ref()
        .map(|options| format!("{}/relay/{desk}", options.public_url.trim_end_matches('/')));
    let ready = json!({"type": "ready", "url": url, "serves": remote.serves()}).to_string();
    if matches!(
        tokio::time::timeout(IO_WITHIN, control.send(Message::Text(ready.into()))).await,
        Ok(Ok(()))
    ) {
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
        loop {
            tokio::select! {
                biased;
                () = generation.cancelled() => break,
                notice = notices.recv() => match notice {
                    Some(text) => if !matches!(tokio::time::timeout(IO_WITHIN, control.send(Message::Text(text.into()))).await, Ok(Ok(()))) { break },
                    None => break,
                },
                message = control.next() => match message {
                    Some(Ok(Message::Text(_))) => {}
                    _ => break,
                },
                _ = ping.tick() => {
                    let ping = json!({"type": "ping"}).to_string();
                    if !matches!(tokio::time::timeout(IO_WITHIN, control.send(Message::Text(ping.into()))).await, Ok(Ok(()))) { break }
                }
            }
        }
    }
    remote.relay.unregister(&desk, id);
}

/// What the desk shows about the relay it stands in on.
#[derive(Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct RemoteRelay {
    /// The paired desk carrying this one's visitors.
    pub desk_id: String,
    pub name: String,
    /// Where a visitor reaches this desk, once the relay has said.
    pub url: Option<String>,
    pub error: Option<String>,
}

#[derive(Default)]
pub(super) struct Standing {
    pub(super) url: Option<String>,
    pub(super) error: Option<String>,
    cancel: Option<CancellationToken>,
}

/// The desks this computer has paired with as an owner, as the shell keeps
/// them in `desks.json`.
pub(super) fn paired(root: &Path) -> Vec<PairedDesk> {
    read_list(&root.join("desks.json"))
}

/// The servers this computer is paired with only as a relay. The shell
/// keeps them in `relays.json`, apart from the desks, so an older window
/// never lists one as a desk and dials its `/v2` (ADR 0006).
fn relay_servers(root: &Path) -> Vec<PairedRelay> {
    read_list(&root.join("relays.json"))
}

/// Every server this desk may stand in on: a desk it owns, or a relay.
fn stand_ins(root: &Path) -> Vec<PairedDesk> {
    let mut servers = paired(root);
    servers.extend(relay_servers(root).iter().map(PairedRelay::dialed));
    servers
}

fn read_list<T: serde::de::DeserializeOwned>(path: &Path) -> Vec<T> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// The relay's sessions this desk will answer at once.
pub(super) const RELAYED_MAX: usize = 32;

impl Remote {
    /// `remote.relays`: the servers this desk may stand in on as a relay only.
    pub fn relay_servers(&self) -> Vec<PairedRelay> {
        relay_servers(&self.root)
    }

    /// Stand in on a paired desk's relay, or stop. The choice is kept, and
    /// the desk stands in whenever Remote is on.
    pub fn relay_through(
        self: &Arc<Self>,
        desk_id: Option<String>,
    ) -> Result<RemoteStatus, String> {
        if self.served.is_some() {
            return Err("A served desk is a relay; it does not use one.".into());
        }
        if let Some(id) = &desk_id
            && !stand_ins(&self.root).iter().any(|desk| &desk.desk_id == id)
        {
            return Err("Pair with that desk first.".into());
        }
        {
            let mut s = lock(&self.state);
            s.saved.relay = desk_id;
            self.save(&s.saved)?;
        }
        self.stand();
        Ok(self.status())
    }

    pub(super) fn relay_status(&self, s: &Live) -> Option<RemoteRelay> {
        let id = s.saved.relay.as_ref()?;
        let name = stand_ins(&self.root)
            .into_iter()
            .find(|desk| &desk.desk_id == id)
            .map_or_else(
                || "A desk this computer no longer pairs with".into(),
                |desk| desk.name,
            );
        Some(RemoteRelay {
            desk_id: id.clone(),
            name,
            url: s.relay.url.clone(),
            error: s.relay.error.clone(),
        })
    }

    /// (Re)starts standing in to match the saved choice and whether Remote
    /// is on. Called after either changes.
    pub(super) fn stand(self: &Arc<Self>) {
        let (desk, cancel) = {
            let mut s = lock(&self.state);
            if let Some(cancel) = s.relay.cancel.take() {
                cancel.cancel();
            }
            s.relay = Standing::default();
            let on = !s.endpoints.is_empty() && !s.cancel.is_cancelled();
            let Some(id) = s.saved.relay.clone().filter(|_| on) else {
                return;
            };
            let Some(desk) = stand_ins(&self.root).into_iter().find(|d| d.desk_id == id) else {
                s.relay.error = Some("This computer no longer pairs with that desk.".into());
                return;
            };
            let cancel = s.cancel.child_token();
            s.relay.cancel = Some(cancel.clone());
            (desk, cancel)
        };
        let remote = self.clone();
        tokio::spawn(async move {
            let visits = remote.relayed.clone();
            let mut tries = 0u32;
            loop {
                let mut ready = false;
                let stood = tokio::select! {
                    biased;
                    () = cancel.cancelled() => return,
                    stood = remote.stand_once(&desk, &visits, &cancel, &mut ready) => stood,
                };
                if ready {
                    tries = 0;
                }
                remote.stood_down(&cancel, None);
                let wait = match stood {
                    // A session that stood in at all starts its backoff over.
                    Ok(()) => Duration::from_millis(500),
                    Err(error) => {
                        let revoked = matches!(error, OpenError::Revoked);
                        remote.stood_down(
                            &cancel,
                            Some(match error {
                                OpenError::Revoked => {
                                    "That desk no longer accepts this computer. Pair with it again."
                                        .into()
                                }
                                // A role refusal is not a revocation: keep trying.
                                other => other.to_string(),
                            }),
                        );
                        if revoked {
                            return;
                        }
                        let wait = Duration::from_millis((500u64 << tries.min(6)).min(30_000));
                        tries = tries.saturating_add(1);
                        wait
                    }
                };
                let jitter = 0.8 + (f64::from(Uuid::new_v4().as_bytes()[0]) / 255.0) * 0.4;
                let wait = wait.mul_f64(jitter);
                tokio::select! { () = cancel.cancelled() => return, () = tokio::time::sleep(wait) => {} }
            }
        });
    }

    fn stood_down(&self, cancel: &CancellationToken, error: Option<String>) {
        let mut s = lock(&self.state);
        if cancel.is_cancelled() {
            return;
        }
        s.relay.url = None;
        s.relay.error = error;
    }

    async fn stand_once(
        self: &Arc<Self>,
        desk: &PairedDesk,
        visits: &Arc<Semaphore>,
        cancel: &CancellationToken,
        ready: &mut bool,
    ) -> Result<(), OpenError> {
        #[derive(Deserialize)]
        struct Notice {
            r#type: String,
            #[serde(default)]
            url: Option<String>,
            #[serde(default)]
            id: Option<String>,
            #[serde(default)]
            path: Option<String>,
            #[serde(default)]
            reason: Option<String>,
        }
        let me = self.status_desktop_id();
        let client = Client::new(self.store.clone());
        let mut control = client.host_relay(desk, &me).await?;
        // Every reconnect is a new local standing generation. Losing its
        // control socket cancels its callbacks before another generation starts.
        let generation = cancel.child_token();
        let _generation = generation.clone().drop_guard();
        let mut stood = false;
        loop {
            let message = tokio::time::timeout(PING_EVERY * 3, control.next())
                .await
                .map_err(|_| "The relay stopped answering.")?;
            let Some(Ok(Message::Text(text))) = message else {
                break;
            };
            let Ok(notice) = serde_json::from_str::<Notice>(&text) else {
                continue;
            };
            match notice.r#type.as_str() {
                "ready" => {
                    let url = notice.url.ok_or("The relay did not say where it is.")?;
                    let mut s = lock(&self.state);
                    if cancel.is_cancelled() {
                        return Ok(());
                    }
                    s.relay.url = Some(url);
                    s.relay.error = None;
                    stood = true;
                    *ready = true;
                }
                "refused" => {
                    return Err(notice.reason.unwrap_or("The relay refused.".into()).into());
                }
                "visit" => {
                    let (Some(id), Some(path)) = (notice.id, notice.path) else {
                        continue;
                    };
                    // A visit past the bound is left for the relay to drop.
                    let Ok(seat) = visits.clone().try_acquire_owned() else {
                        continue;
                    };
                    let Some(purpose) = Purpose::of(self, &path) else {
                        continue;
                    };
                    let remote = self.clone();
                    let desk = desk.clone();
                    let client = client.clone();
                    let cancel = generation.clone();
                    tokio::spawn(async move {
                        let dialed = tokio::select! {
                            biased;
                            () = cancel.cancelled() => return,
                            dialed = tokio::time::timeout(ACCEPT_WITHIN, client.accept_relay(&desk, &id)) => dialed,
                        };
                        if let Ok(Ok(socket)) = dialed {
                            sealed_session(
                                remote,
                                socket,
                                purpose,
                                Seat::Relayed {
                                    _visit: seat,
                                    cancel,
                                },
                            )
                            .await;
                        }
                    });
                }
                _ => {}
            }
        }
        self.stood_down(cancel, Some("The relay closed the connection.".into()));
        if stood {
            Ok(())
        } else {
            Err("The relay closed the connection.".into())
        }
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use crate::remote::poison;

    fn register(hub: &Hub, desk: &str) -> Uuid {
        let (notify, _) = mpsc::channel(VISITS_PER_DESK);
        hub.register(desk, "owner", notify, CancellationToken::new())
            .unwrap()
    }
    fn replenish(hub: &Hub, desk: &str) {
        let mut hosts = lock(&hub.hosts);
        hosts.budgets.get_mut(desk).unwrap().updated -= Duration::from_secs(20);
    }
    fn ticket(hub: &Hub, desk: &str) -> Ticket {
        let Visit::Admitted(_, ticket) = hub.visit(desk) else {
            panic!("visit refused");
        };
        ticket
    }

    #[tokio::test]
    async fn an_expired_pending_control_cannot_retire_or_replace_a_host() {
        let hub = Hub::default();
        let current = register(&hub, "desk");
        let admission = Arc::new(admission::Admission::default());
        let pending = admission
            .accept(
                "127.0.0.1".parse().unwrap(),
                tokio::time::Instant::now() - Duration::from_secs(5),
            )
            .await;
        assert_eq!(pending.while_pending(|| hub.retire("desk")), None);
        let (notify, _) = mpsc::channel(VISITS_PER_DESK);
        assert_eq!(
            pending.while_pending(|| hub.register(
                "desk",
                "replacement",
                notify,
                CancellationToken::new()
            )),
            None
        );
        let visit = ticket(&hub, "desk");
        assert_eq!(visit.generation.id, current);
        assert!(!visit.generation.cancel.is_cancelled());
    }

    #[test]
    fn a_poisoned_registry_still_ends_generations_and_refuses_their_visitors() {
        let hub = Hub::default();
        let id = register(&hub, "desk");
        let visit = ticket(&hub, "desk");
        poison(&hub.hosts);
        hub.unregister("desk", id);
        assert!(visit.generation.cancel.is_cancelled());
        assert!(matches!(hub.visit("desk"), Visit::Offline));
        let fresh = register(&hub, "desk");
        assert!(hub.retire("desk"));
        assert!(matches!(hub.visit("desk"), Visit::Offline));
        hub.unregister("desk", fresh);
        assert_eq!(hub.counts(), (0, 0));
    }

    #[test]
    fn reregistration_keeps_live_permits_and_invalidates_the_old_generation() {
        let hub = Hub::default();
        let old = register(&hub, "desk");
        let mut visits = Vec::new();
        for _ in 0..VISITS_PER_DESK {
            replenish(&hub, "desk");
            visits.push(ticket(&hub, "desk"));
        }
        register(&hub, "desk");
        replenish(&hub, "desk");
        assert!(matches!(hub.visit("desk"), Visit::Busy));
        assert!(!Hub::current(&lock(&hub.hosts), &visits[0]));
        hub.unregister("desk", old);
        assert_eq!(hub.counts().0, 1);
        visits.pop();
        let resumed = ticket(&hub, "desk");
        assert!(Hub::current(&lock(&hub.hosts), &resumed));
    }

    #[test]
    fn visit_rate_survives_replacement_and_offline_registration() {
        let hub = Hub::default();
        let id = register(&hub, "desk");
        for _ in 0..10 {
            drop(ticket(&hub, "desk"));
        }
        assert!(matches!(hub.visit("desk"), Visit::RateLimited));
        hub.unregister("desk", id);
        register(&hub, "desk");
        assert!(matches!(hub.visit("desk"), Visit::RateLimited));
        replenish(&hub, "desk");
        drop(ticket(&hub, "desk"));
    }

    #[test]
    fn all_desks_share_the_relay_wide_session_ceiling() {
        let hub = Hub::default();
        let mut visits = Vec::new();
        for desk in 0..16 {
            let desk = desk.to_string();
            register(&hub, &desk);
            for _ in 0..16 {
                replenish(&hub, &desk);
                visits.push(ticket(&hub, &desk));
            }
        }
        register(&hub, "overflow");
        assert!(matches!(hub.visit("overflow"), Visit::Busy));
        visits.pop();
        drop(ticket(&hub, "overflow"));
    }
}
