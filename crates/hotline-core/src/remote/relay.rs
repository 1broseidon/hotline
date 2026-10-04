//! A relay carries sealed records between a visitor and a desk; it never
//! holds a key that could read them.
//!
//! A desk with no reachable listener dials out to a served desk it is paired
//! with as an owner, over a sealed control socket (`/v2/relay`), and stands
//! in there under its own desk id. A visitor dials `/relay/{deskId}/v2…` on
//! the served desk exactly as it would dial the desk itself. The served desk
//! hands the desk a single-use capability over the control socket; the desk
//! dials `/relay/accept/{capability}` back, and the relay joins the two
//! sockets message for message. The Noise handshake inside runs between the
//! visitor and the desk, so a relay can drop a session but cannot read,
//! forge or redirect one: the visitor pins the desk's key, not the relay's.
use super::client::{Client, OpenError, PairedDesk};
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

type Socket = WebSocketStream<TokioIo<Upgraded>>;

pub(super) fn valid_desk(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub(super) fn record_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(RECORD_MAX))
        .max_frame_size(Some(RECORD_MAX))
}

/// The desks a served desk stands in for, and the visitors waiting for one.
#[derive(Default)]
pub(super) struct Hub(Mutex<Hosts>);
#[derive(Default)]
struct Hosts {
    desks: HashMap<String, Host>,
    waiting: HashMap<String, Waiting>,
}
struct Host {
    id: Uuid,
    notify: mpsc::Sender<String>,
    visits: Arc<Semaphore>,
}
struct Waiting {
    socket: Socket,
    _visit: OwnedSemaphorePermit,
}

/// A visitor's way in: the desk it names is standing in, and has room.
pub(super) enum Visit {
    Offline,
    Busy,
    Admitted(mpsc::Sender<String>, OwnedSemaphorePermit),
}

impl Hub {
    fn register(&self, desk: &str, notify: mpsc::Sender<String>) -> Option<Uuid> {
        let mut hosts = self.0.lock().unwrap();
        if hosts.desks.len() >= HOSTS_MAX && !hosts.desks.contains_key(desk) {
            return None;
        }
        let id = Uuid::new_v4();
        // The newest connection from an owner's device wins; the one it
        // replaces sees its notices end and closes.
        hosts.desks.insert(
            desk.to_owned(),
            Host {
                id,
                notify,
                visits: Arc::new(Semaphore::new(VISITS_PER_DESK)),
            },
        );
        Some(id)
    }

    fn unregister(&self, desk: &str, id: Uuid) {
        let mut hosts = self.0.lock().unwrap();
        if hosts.desks.get(desk).is_some_and(|host| host.id == id) {
            hosts.desks.remove(desk);
        }
    }

    pub(super) fn visit(&self, desk: &str) -> Visit {
        let hosts = self.0.lock().unwrap();
        let Some(host) = hosts.desks.get(desk) else {
            return Visit::Offline;
        };
        match host.visits.clone().try_acquire_owned() {
            Ok(permit) => Visit::Admitted(host.notify.clone(), permit),
            Err(_) => Visit::Busy,
        }
    }

    fn wait(self: &Arc<Self>, socket: Socket, visit: OwnedSemaphorePermit) -> String {
        let capability = super::secret();
        self.0.lock().unwrap().waiting.insert(
            capability.clone(),
            Waiting {
                socket,
                _visit: visit,
            },
        );
        let hub = Arc::downgrade(self);
        let unclaimed = capability.clone();
        tokio::spawn(async move {
            tokio::time::sleep(ACCEPT_WITHIN).await;
            if let Some(hub) = hub.upgrade() {
                hub.0.lock().unwrap().waiting.remove(&unclaimed);
            }
        });
        capability
    }

    fn claim(&self, capability: &str) -> Option<Waiting> {
        self.0.lock().unwrap().waiting.remove(capability)
    }

    #[cfg(test)]
    pub(super) fn counts(&self) -> (usize, usize) {
        let hosts = self.0.lock().unwrap();
        (hosts.desks.len(), hosts.waiting.len())
    }
}

/// `/relay/{deskId}/{rest}`: the desk and the path the visitor asked of it,
/// or `/relay/accept/{capability}`, a desk dialing back for its visitor.
pub(super) enum Route {
    Visit { desk: String, path: String },
    Accept(String),
}
impl Route {
    pub(super) fn of(path: &str) -> Option<Self> {
        let rest = path.strip_prefix("/relay/")?;
        let (head, tail) = rest.split_once('/')?;
        if head == "accept" {
            return (tail.len() == 64 && tail.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| Self::Accept(tail.to_owned()));
        }
        if !valid_desk(head) {
            return None;
        }
        let path = format!("/{tail}");
        let known = path == "/v2" || path == "/v2/pair" || {
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
    visit: OwnedSemaphorePermit,
    path: String,
) {
    let capability = hub.wait(socket, visit);
    let notice = json!({"type": "visit", "id": capability, "path": path}).to_string();
    if notify.try_send(notice).is_err() {
        hub.claim(&capability);
    }
}

/// The desk's socket for a capability, upgraded: join it to its visitor.
pub(super) async fn accept(hub: &Hub, capability: &str, desk: Socket, cancel: CancellationToken) {
    let Some(waiting) = hub.claim(capability) else {
        return;
    };
    let Waiting { socket, _visit } = waiting;
    join(socket, desk, cancel).await;
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
                        if b_out.send(Message::Binary(bytes)).await.is_err() { break }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    _ => break,
                },
                message = b_in.next() => match message {
                    Some(Ok(Message::Binary(bytes))) => {
                        if a_out.send(Message::Binary(bytes)).await.is_err() { break }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    _ => break,
                },
            }
        }
    };
    tokio::select! { _ = cancel.cancelled() => {}, _ = forward => {} }
    let _ = a_out.close().await;
    let _ = b_out.close().await;
}

/// A desk standing in on this served desk, over its sealed control socket,
/// until it goes, its device is revoked, or another of the owner's devices
/// stands in for the same desk.
pub(super) async fn host<S>(
    remote: Arc<Remote>,
    mut control: super::channel::Channel<S>,
    phone: Phone,
    desk: String,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (notify, mut notices) = mpsc::channel(VISITS_PER_DESK);
    let Some(id) = remote.relay.register(&desk, notify) else {
        let refusal = json!({"type": "refused", "reason": "This relay is full."});
        let _ = control
            .send(Message::Text(refusal.to_string().into()))
            .await;
        return;
    };
    let url = remote
        .served
        .as_ref()
        .map(|options| format!("{}/relay/{desk}", options.public_url.trim_end_matches('/')));
    let ready = json!({"type": "ready", "url": url}).to_string();
    if control.send(Message::Text(ready.into())).await.is_ok() {
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
        loop {
            tokio::select! {
                biased;
                _ = phone.cancel.cancelled() => break,
                notice = notices.recv() => match notice {
                    Some(text) => if control.send(Message::Text(text.into())).await.is_err() { break },
                    None => break,
                },
                message = control.next() => match message {
                    Some(Ok(Message::Text(_))) => {}
                    _ => break,
                },
                _ = ping.tick() => {
                    let ping = json!({"type": "ping"}).to_string();
                    if control.send(Message::Text(ping.into())).await.is_err() { break }
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

/// The desks this computer has paired with, as the shell keeps them.
fn paired(root: &Path) -> Vec<PairedDesk> {
    fs::read(root.join("desks.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// The relay's sessions this desk will answer at once.
const RELAYED_MAX: usize = 32;

impl Remote {
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
            && !paired(&self.root).iter().any(|desk| &desk.desk_id == id)
        {
            return Err("Pair with that desk first.".into());
        }
        {
            let mut s = self.state.lock().unwrap();
            s.saved.relay = desk_id;
            self.save(&s.saved)?;
        }
        self.stand();
        Ok(self.status())
    }

    pub(super) fn relay_status(&self, s: &Live) -> Option<RemoteRelay> {
        let id = s.saved.relay.as_ref()?;
        let name = paired(&self.root)
            .into_iter()
            .find(|desk| &desk.desk_id == id)
            .map(|desk| desk.name)
            .unwrap_or_else(|| "A desk this computer no longer pairs with".into());
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
            let mut s = self.state.lock().unwrap();
            if let Some(cancel) = s.relay.cancel.take() {
                cancel.cancel();
            }
            s.relay = Standing::default();
            let on = !s.endpoints.is_empty() && !s.cancel.is_cancelled();
            let Some(id) = s.saved.relay.clone().filter(|_| on) else {
                return;
            };
            let Some(desk) = paired(&self.root).into_iter().find(|d| d.desk_id == id) else {
                s.relay.error = Some("This computer no longer pairs with that desk.".into());
                return;
            };
            let cancel = s.cancel.child_token();
            s.relay.cancel = Some(cancel.clone());
            (desk, cancel)
        };
        let remote = self.clone();
        tokio::spawn(async move {
            let visits = Arc::new(Semaphore::new(RELAYED_MAX));
            for tries in 0u32.. {
                let stood = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    stood = remote.stand_once(&desk, &visits, &cancel) => stood,
                };
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
                                OpenError::Unreachable(message) => message,
                            }),
                        );
                        if revoked {
                            return;
                        }
                        Duration::from_millis((500u64 << tries.min(6)).min(30_000))
                    }
                };
                tokio::select! { _ = cancel.cancelled() => return, _ = tokio::time::sleep(wait) => {} }
            }
        });
    }

    fn stood_down(&self, cancel: &CancellationToken, error: Option<String>) {
        let mut s = self.state.lock().unwrap();
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
                    let mut s = self.state.lock().unwrap();
                    if cancel.is_cancelled() {
                        return Ok(());
                    }
                    s.relay.url = Some(url);
                    s.relay.error = None;
                    stood = true;
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
                    let base = desk.url.clone();
                    tokio::spawn(async move {
                        let dialed = tokio::time::timeout(
                            ACCEPT_WITHIN,
                            super::client::dial(&base, &format!("relay/accept/{id}")),
                        )
                        .await;
                        if let Ok(Ok(socket)) = dialed {
                            sealed_session(remote, socket, purpose, Seat::Relayed { _visit: seat })
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
