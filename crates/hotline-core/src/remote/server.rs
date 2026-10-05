use super::*;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Request, Response, StatusCode, body::Incoming, server::conn::http1, service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{convert::Infallible, time::Duration};
use tokio::net::TcpListener;
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        self,
        pki_types::{CertificateDer, PrivatePkcs8KeyDer},
    },
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        handshake::server::create_response,
        protocol::{Role, WebSocketConfig},
    },
};

/// A screen frame is a whole PNG; the wire's 64 KiB would cut one in half.
const COMPUTER_MESSAGE_MAX: usize = 16 * 1024 * 1024;

pub(super) fn tls(identity: &Identity) -> Result<TlsAcceptor, String> {
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(message)?
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(identity.certificate.clone())],
        PrivatePkcs8KeyDer::from(identity.key.clone()).into(),
    )
    .map_err(message)?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}
fn response(status: StatusCode, value: Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("cache-control", "no-store")
        .body(Full::new(Bytes::from(value.to_string())))
        .unwrap()
}
fn error(status: StatusCode, code: &str) -> Response<Full<Bytes>> {
    response(status, json!({"error": code}))
}
/// A small JSON body, read within a deadline. Anything else is a bad claim.
async fn body<T: serde::de::DeserializeOwned>(
    request: Request<Incoming>,
) -> Result<T, &'static str> {
    let body = tokio::time::timeout(
        Duration::from_secs(5),
        Limited::new(request.into_body(), 8192).collect(),
    )
    .await;
    let Ok(Ok(body)) = body else {
        return Err("invalid_claim");
    };
    serde_json::from_slice(&body.to_bytes()).map_err(|_| "invalid_claim")
}

pub(super) fn run(
    remote: Arc<Remote>,
    listener: TcpListener,
    tls: TlsAcceptor,
    cancel: CancellationToken,
) -> futures_util::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        loop {
            let accepted = tokio::select! { biased; _ = cancel.cancelled() => break, accepted = listener.accept() => accepted };
            let (socket, peer) = match accepted {
                Ok(v) => v,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let accepted = tokio::time::Instant::now();
            let slot = tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                slot = remote.admission.accept(peer.ip(), accepted) => slot,
            };
            let remote = remote.clone();
            let tls = tls.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                let accepted = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    _ = slot.expired() => return,
                    accepted = tls.accept(socket) => accepted,
                };
                let Ok(socket) = accepted else {
                    return;
                };
                let pending = slot.clone();
                let service =
                    service_fn(move |request| handle(remote.clone(), request, slot.clone()));
                let mut builder = http1::Builder::new();
                builder
                    .timer(TokioTimer::new())
                    .header_read_timeout(Duration::from_secs(10));
                let connection = builder
                    .serve_connection(TokioIo::new(socket), service)
                    .with_upgrades();
                tokio::select! { biased; _ = cancel.cancelled() => {}, _ = pending.expired() => {}, _ = connection => {} }
            });
        }
    })
}
async fn handle(
    remote: Arc<Remote>,
    mut request: Request<Incoming>,
    slot: Arc<admission::Permit>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    // Pairing and wire credentials belong to native clients. Browser origins
    // cannot spend them, and no CORS or query-token route is offered here.
    if request.headers().contains_key("origin") || request.uri().query().is_some() {
        return Ok(error(StatusCode::FORBIDDEN, "native_clients_only"));
    }
    let Some(path) = remote.route_path(request.uri().path()).map(str::to_owned) else {
        return Ok(error(StatusCode::NOT_FOUND, "not_found"));
    };
    if request.method() == "GET"
        && (path == "/v2"
            || path == "/v2/pair"
            || path == "/v2/relay"
            || path == "/v2/relay/accept"
            || path.starts_with("/v2/computer/"))
    {
        return Ok(sealed_door(remote, request, slot, &path).await);
    }
    if request.method() == "GET"
        && remote.served.is_some()
        && let Some(route) = super::relay::Route::of(&path)
    {
        return Ok(relay_door(remote, request, slot, route).await);
    }
    // Served listeners have no bearer or manual pairing surface, including
    // when an old desktop grant remains on disk for a later desktop launch.
    if remote.served.is_some() {
        return Ok(error(StatusCode::NOT_FOUND, "not_found"));
    }
    if request.method() == "POST" {
        let outcome = match path.as_str() {
            "/pair" => body::<Claim>(request)
                .await
                .and_then(|claim| remote.claim(claim)),
            "/pair/manual/start" => body::<ManualStart>(request)
                .await
                .and_then(|start| remote.manual_start(start)),
            "/pair/manual/finish" => body::<ManualFinish>(request)
                .await
                .and_then(|finish| remote.manual_finish(finish)),
            _ => return Ok(error(StatusCode::NOT_FOUND, "not_found")),
        };
        return Ok(match outcome {
            Ok(value) => response(StatusCode::OK, value),
            Err("invalid_claim") => error(StatusCode::BAD_REQUEST, "invalid_claim"),
            Err(code @ ("rate_limited" | "too_many_attempts")) => {
                error(StatusCode::TOO_MANY_REQUESTS, code)
            }
            Err("storage_unavailable") => {
                error(StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable")
            }
            Err(code) => error(StatusCode::FORBIDDEN, code),
        });
    }
    if request.method() != "GET" {
        return Ok(error(StatusCode::NOT_FOUND, "not_found"));
    }
    if let Some(persona_id) = computer_path(&path).map(str::to_owned) {
        return Ok(computer_door(remote, request, slot, &persona_id).await);
    }
    if path != "/ws" {
        return Ok(error(StatusCode::NOT_FOUND, "not_found"));
    }
    let Some(phone) = remote.authenticate(bearer(&request)) else {
        return Ok(error(StatusCode::UNAUTHORIZED, "unauthorized"));
    };
    let upgraded = hyper::upgrade::on(&mut request);
    let Ok(reply) = create_response(&request.map(|_| ())) else {
        return Ok(error(StatusCode::BAD_REQUEST, "invalid_upgrade"));
    };
    tokio::spawn(async move {
        let upgraded = tokio::select! {
            biased;
            _ = phone.cancel.cancelled() => return,
            _ = slot.expired() => return,
            upgraded = upgraded => upgraded,
        };
        let Ok(upgraded) = upgraded else {
            return;
        };
        if phone.cancel.is_cancelled() || !slot.authenticate(&phone.id) {
            return;
        }
        let _slot = slot;
        // Authentication has finished. Owners can send bounded voice clips;
        // companion input retains the smaller legacy cap.
        let message_max = if phone.role == DeviceRole::Owner {
            3 * 1024 * 1024
        } else {
            65_536
        };
        let config = WebSocketConfig::default()
            .max_message_size(Some(message_max))
            .max_frame_size(Some(message_max));
        let socket =
            WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, Some(config))
                .await;
        let desktop_id = remote.state.lock().unwrap().saved.desktop_id.clone();
        let _ = crate::wire::seated_phone(
            socket,
            remote.log.clone(),
            remote.room.clone(),
            phone,
            &desktop_id,
        )
        .await;
    });
    Ok(reply.map(|_| Full::new(Bytes::new())))
}

fn bearer<B>(request: &Request<B>) -> &str {
    request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
}

/// `/computer/{personaId}/ws`, and the persona it names. A phone names a
/// teammate and nothing else: no runtime, no port, no path of its own.
fn computer_path(path: &str) -> Option<&str> {
    let persona_id = path.strip_prefix("/computer/")?.strip_suffix("/ws")?;
    let named = !persona_id.is_empty()
        && persona_id.len() <= 128
        && persona_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    named.then_some(persona_id)
}

/// Where a running computer's viewer socket is, from the status the desk
/// keeps for its own window. The bearer stays here: the phone is handed a
/// pipe, never the address.
pub(crate) fn computer_target(status: &crate::contract::ComputerStatus) -> Option<(u16, String)> {
    if status.state != crate::contract::ComputerState::Running {
        return None;
    }
    let rest = status
        .viewer
        .as_deref()?
        .strip_prefix("http://127.0.0.1:")?;
    let (port, token) = rest.split_once("/#")?;
    let port = port.parse().ok()?;
    (!token.is_empty()).then(|| (port, token.to_string()))
}

/// A door to a teammate's computer for a paired phone. The desk checks the
/// grant and that the computer is up, then carries bytes between the phone
/// and the container's own viewer socket on loopback, presenting the bearer
/// it holds. Frames go to the phone as they are; what the phone sends goes
/// to the computer as it is, text only, since input is text. Revoking the
/// device drops the pipe.
async fn computer_door(
    remote: Arc<Remote>,
    mut request: Request<Incoming>,
    slot: Arc<admission::Permit>,
    persona_id: &str,
) -> Response<Full<Bytes>> {
    let Some(phone) = remote.authenticate(bearer(&request)) else {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let Ok(status) = remote.room.computer_status(persona_id).await else {
        return error(StatusCode::NOT_FOUND, "not_found");
    };
    let Some((port, token)) = computer_target(&status) else {
        return error(StatusCode::CONFLICT, "computer_not_running");
    };
    let upgraded = hyper::upgrade::on(&mut request);
    let Ok(reply) = create_response(&request.map(|_| ())) else {
        return error(StatusCode::BAD_REQUEST, "invalid_upgrade");
    };
    tokio::spawn(async move {
        let upgraded = tokio::select! {
            biased;
            _ = phone.cancel.cancelled() => return,
            _ = slot.expired() => return,
            upgraded = upgraded => upgraded,
        };
        let Ok(upgraded) = upgraded else {
            return;
        };
        if phone.cancel.is_cancelled() || !slot.authenticate(&phone.id) {
            return;
        }
        let _slot = slot;
        let config = WebSocketConfig::default()
            .max_message_size(Some(COMPUTER_MESSAGE_MAX))
            .max_frame_size(Some(COMPUTER_MESSAGE_MAX));
        let mut phone_socket =
            WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, Some(config))
                .await;
        let address = format!("ws://127.0.0.1:{port}/ws?token={token}");
        let Ok(Ok((computer, _))) = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_tungstenite::connect_async(address),
        )
        .await
        else {
            let _ = phone_socket.close(None).await;
            return;
        };
        pipe(phone_socket, computer, phone.cancel).await;
    });
    reply.map(|_| Full::new(Bytes::new()))
}

async fn pipe<P, C>(
    mut phone: WebSocketStream<P>,
    mut computer: WebSocketStream<C>,
    revoked: CancellationToken,
) where
    P: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    C: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        tokio::select! {
            _ = revoked.cancelled() => break,
            from_phone = phone.next() => match from_phone {
                Some(Ok(Message::Text(text))) => {
                    if computer.send(Message::Text(text)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            from_computer = computer.next() => match from_computer {
                Some(Ok(message @ (Message::Binary(_) | Message::Text(_)))) => {
                    if phone.send(message).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
        }
    }
    let _ = phone.close(None).await;
    let _ = computer.close(None).await;
}

async fn sealed_door(
    remote: Arc<Remote>,
    mut request: Request<Incoming>,
    slot: Arc<admission::Permit>,
    path: &str,
) -> Response<Full<Bytes>> {
    let Some(purpose) = Purpose::of(&remote, path) else {
        return error(StatusCode::NOT_FOUND, "not_found");
    };
    let upgraded = hyper::upgrade::on(&mut request);
    let Ok(reply) = create_response(&request.map(|_| ())) else {
        return error(StatusCode::BAD_REQUEST, "invalid_upgrade");
    };
    tokio::spawn(async move {
        let socket = async {
            let upgraded = upgraded.await.ok()?;
            Some(
                WebSocketStream::from_raw_socket(
                    TokioIo::new(upgraded),
                    Role::Server,
                    Some(handshake_config()),
                )
                .await,
            )
        };
        let socket = tokio::select! {
            biased;
            _ = slot.expired() => return,
            socket = socket => socket,
        };
        if let Some(socket) = socket {
            sealed_session(remote, socket, purpose, Seat::Direct(slot)).await;
        }
    });
    reply.map(|_| Full::new(Bytes::new()))
}

/// A visitor waits for the registered desk's sealed callback. The upgrade
/// releases its pending transport permit; the hub bounds visits instead.
async fn relay_door(
    remote: Arc<Remote>,
    mut request: Request<Incoming>,
    slot: Arc<admission::Permit>,
    route: super::relay::Route,
) -> Response<Full<Bytes>> {
    use super::relay::{Route, Visit};
    let Route::Visit { desk, path } = route;
    let (notify, permit) = match remote.relay.visit(&desk) {
        Visit::Offline => return error(StatusCode::NOT_FOUND, "desk_offline"),
        Visit::Busy => return error(StatusCode::SERVICE_UNAVAILABLE, "desk_busy"),
        Visit::RateLimited => return error(StatusCode::TOO_MANY_REQUESTS, "desk_busy"),
        Visit::Admitted(notify, permit) => (notify, permit),
    };
    let cancel = remote.state.lock().unwrap().cancel.clone();
    let upgraded = hyper::upgrade::on(&mut request);
    let Ok(reply) = create_response(&request.map(|_| ())) else {
        return error(StatusCode::BAD_REQUEST, "invalid_upgrade");
    };
    tokio::spawn(async move {
        let upgraded = tokio::select! {
            biased;
            _ = cancel.cancelled() => return,
            _ = slot.expired() => return,
            upgraded = upgraded => upgraded,
        };
        let Ok(upgraded) = upgraded else {
            return;
        };
        drop(slot);
        let socket = WebSocketStream::from_raw_socket(
            TokioIo::new(upgraded),
            Role::Server,
            Some(super::relay::record_config()),
        )
        .await;
        super::relay::arrive(remote.relay.clone(), socket, notify, permit, path).await;
    });
    reply.map(|_| Full::new(Bytes::new()))
}

/// The initial WebSocket cap prevents allocation of an unbounded handshake.
pub(super) fn handshake_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(65535))
        .max_frame_size(Some(65535))
}

/// What a sealed socket is for, from its path. Pairing needs an open window;
/// hosting a relay is a served desk's door only.
#[derive(Clone)]
pub(super) enum Purpose {
    Wire,
    Pair,
    Computer(String),
    Relay,
    RelayAccept,
}
impl Purpose {
    pub(super) fn of(remote: &Remote, path: &str) -> Option<Self> {
        match path {
            "/v2" => Some(Self::Wire),
            "/v2/pair" => remote.pairing_open().then_some(Self::Pair),
            "/v2/relay" => remote.served.is_some().then_some(Self::Relay),
            "/v2/relay/accept" => remote.served.is_some().then_some(Self::RelayAccept),
            _ => path
                .strip_prefix("/v2")
                .and_then(computer_path)
                .map(|persona| Self::Computer(persona.to_owned())),
        }
    }
}

/// Where a sealed session sits. A direct socket holds an admission seat; a
/// socket the desk dialed out to its relay for a visitor has no TCP peer of
/// its own, only the relay's bound on visits and the same handshake deadline.
pub(super) enum Seat {
    Direct(Arc<admission::Permit>),
    Relayed {
        _visit: tokio::sync::OwnedSemaphorePermit,
        cancel: CancellationToken,
    },
}
impl Seat {
    pub(super) async fn expired(&self, deadline: tokio::time::Instant) {
        match self {
            Self::Direct(slot) => slot.expired().await,
            Self::Relayed { cancel, .. } => tokio::select! {
                _ = cancel.cancelled() => {},
                _ = tokio::time::sleep_until(deadline) => {},
            },
        }
    }
}

/// Both Noise messages, then the session the path asked for. The same code
/// answers a phone on the desk's own listener and one carried by a relay,
/// which sees only these ciphertexts.
pub(super) async fn sealed_session<S>(
    remote: Arc<Remote>,
    mut socket: WebSocketStream<S>,
    purpose: Purpose,
    seat: Seat,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    if matches!(purpose, Purpose::RelayAccept) {
        super::relay::accept(remote, socket, seat).await;
        return;
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Binding {
        purpose: String,
        #[serde(default)]
        persona_id: Option<String>,
        #[serde(default)]
        desk_id: Option<String>,
    }

    let cancel = remote.state.lock().unwrap().cancel.clone();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    // The same deadline covers TCP acceptance through both Noise messages.
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
        let public = noise.get_remote_static()?.to_vec();
        let mut desk = None;
        let (phone, answer) = if let Purpose::Pair = purpose {
            let role = remote.claim_v2(&public, &payload[..size]).ok()?;
            (
                None,
                serde_json::to_vec(&json!({"role": role, "deskName": desktop_name(), "deskId": remote.status_desktop_id()})).ok()?,
            )
        } else {
            // A TLS proxy or a relay can rewrite the HTTP target, but not
            // this authenticated Noise payload. Bind it before looking up or
            // connecting to anything; ordinary wire payloads stay empty.
            match &purpose {
                Purpose::Wire if size != 0 => return None,
                Purpose::Wire | Purpose::Pair => {}
                Purpose::RelayAccept => return None,
                Purpose::Computer(persona) => {
                    let binding: Binding = serde_json::from_slice(&payload[..size]).ok()?;
                    if binding.purpose != "computer"
                        || binding.persona_id.as_ref() != Some(persona)
                        || binding.desk_id.is_some()
                    {
                        return None;
                    }
                }
                Purpose::Relay => {
                    let binding: Binding = serde_json::from_slice(&payload[..size]).ok()?;
                    if binding.purpose != "relay" || binding.persona_id.is_some() {
                        return None;
                    }
                    desk = Some(binding.desk_id.filter(|id| super::relay::valid_desk(id))?);
                }
            }
            let phone = remote.authenticate_v2(&public);
            // Disabling Remote also refuses authentication, but does not
            // revoke saved grants. Do not turn shutdown into a key refusal.
            if phone.is_none() && cancel.is_cancelled() {
                return None;
            }
            // Only an owner's device may stand in for a desk on the relay.
            let refused = phone.as_ref().is_none_or(|phone| {
                matches!(purpose, Purpose::Relay) && phone.role != DeviceRole::Owner
            });
            let answer = if !refused {
                Vec::new()
            } else {
                // Message 1 decrypted successfully: the initiator knows the
                // desk key. Seal the refusal under that same Noise identity.
                sealed::DEVICE_REJECTED.to_vec()
            };
            (phone.filter(|_| !refused), answer)
        };
        let mut response = [0u8; 4096];
        let size = noise.write_message(&answer, &mut response).ok()?;
        socket
            .send(Message::Binary(response[..size].to_vec().into()))
            .await
            .ok()?;
        let phone = phone?;
        let state = noise.into_transport_mode().ok()?;
        if phone.cancel.is_cancelled() {
            return None;
        }
        // Hosting takes the hub's separate 64 slots, never a direct-device seat.
        let authenticated = match (&purpose, &seat) {
            (Purpose::Relay, _) => None,
            (_, Seat::Direct(slot)) => {
                if !slot.authenticate(&phone.id) {
                    return None;
                }
                None
            }
            (_, Seat::Relayed { .. }) => Some(remote.admission.seat(&phone.id)?),
        };
        Some((
            super::channel::Channel::new(socket, state),
            phone,
            desk,
            authenticated,
        ))
    };
    let established = tokio::select! {
        biased;
        _ = cancel.cancelled() => return,
        _ = seat.expired(deadline) => return,
        result = handshake => result,
    };
    let Some((socket, phone, desk, _authenticated)) = established else {
        return;
    };
    // Forward standing cancellation to the wire's own cleanup path. Dropping
    // seated_phone_v2 while it owns writer/subscription tasks would leak them.
    let cancellation = if let Seat::Relayed { cancel, .. } = &seat {
        let cancel = cancel.clone();
        let phone_cancel = phone.cancel.clone();
        Some(tokio::spawn(async move {
            tokio::select! {
                _ = cancel.cancelled() => phone_cancel.cancel(),
                _ = phone_cancel.cancelled() => {},
            }
        }))
    } else {
        None
    };
    // Retain a control's pending permit until its relay slot is acquired,
    // including a replacement waiting for its predecessor's bounded cleanup.
    let mut seat = Some(seat);
    match purpose {
        Purpose::Computer(persona) => {
            let revoked = phone.cancel.clone();
            tokio::select! {
                biased;
                _ = revoked.cancelled() => {},
                _ = sealed_computer(remote.clone(), socket, &persona) => {},
            }
        }
        Purpose::Relay => {
            if let Some(desk) = desk {
                super::relay::host(remote, socket, phone, desk, seat.take().unwrap()).await;
            }
        }
        Purpose::RelayAccept => {}
        Purpose::Wire | Purpose::Pair => {
            // The wire owns its writer and subscription tasks and must reach
            // their cleanup on revocation; dropping that future leaks them.
            let desktop_id = remote.state.lock().unwrap().saved.desktop_id.clone();
            let _ = crate::wire::seated_phone_v2(
                socket,
                remote.log.clone(),
                remote.room.clone(),
                phone,
                &desktop_id,
            )
            .await;
        }
    }
    if let Some(cancellation) = cancellation {
        cancellation.abort();
    }
}

/// A viewer uses the very same sealed connection, but its plaintext frame is
/// an envelope: {"type":"text","data":"…"} or {"type":"binary","data":"<base64>"}.
/// Inputs are text envelopes only. Container addresses and bearers never leave.
async fn sealed_computer<S>(
    remote: Arc<Remote>,
    mut phone: super::channel::Channel<S>,
    persona: &str,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use base64::{Engine, engine::general_purpose::STANDARD};
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        r#type: String,
        data: String,
    }
    let Ok(status) = remote.room.computer_status(persona).await else {
        return;
    };
    let Some((port, token)) = computer_target(&status) else {
        return;
    };
    let address = format!("ws://127.0.0.1:{port}/ws?token={token}");
    let config = WebSocketConfig::default()
        .max_message_size(Some(COMPUTER_MESSAGE_MAX))
        .max_frame_size(Some(COMPUTER_MESSAGE_MAX));
    let Ok(Ok((mut computer, _))) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async_with_config(address, Some(config), false),
    )
    .await
    else {
        return;
    };
    let Ok(mut files) =
        super::viewer_files::Relay::start(port, token, remote.log.root().join("viewer-uploads"))
    else {
        return;
    };
    loop {
        tokio::select! {
            input = phone.next() => match input {
                Some(Ok(Message::Text(text))) => {
                    let Ok(input) = serde_json::from_str::<Input>(&text) else { break; };
                    if input.r#type != "text" || input.data.len() > super::viewer_files::MAX_REQUEST { break; }
                    if let Ok(request) = serde_json::from_str::<Value>(&input.data)
                        && request["type"] == "files" {
                        if let Some(reply) = files.enqueue(request)
                            && phone.send(Message::text(json!({"type":"text","data":reply.to_string()}).to_string())).await.is_err() { break; }
                        continue;
                    }
                    if input.data.len() > 65536 { break; }
                    if computer.send(Message::Text(input.data.into())).await.is_err() { break; }
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {},
                _ => break,
            },
            reply = files.replies.recv() => {
                let Some(reply) = reply else { break; };
                if phone.send(Message::text(json!({"type":"text","data":reply.to_string()}).to_string())).await.is_err() { break; }
            },
            output = computer.next() => {
                let value = match output {
                    Some(Ok(Message::Text(text))) => json!({"type":"text", "data":text.as_str()}),
                    Some(Ok(Message::Binary(bytes))) => json!({"type":"binary", "data":STANDARD.encode(bytes)}),
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    _ => break,
                };
                if phone.send(Message::Text(value.to_string().into())).await.is_err() { break; }
            }
        }
    }
}
