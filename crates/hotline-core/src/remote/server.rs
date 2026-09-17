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

pub(super) async fn run(
    remote: Arc<Remote>,
    listener: TcpListener,
    tls: TlsAcceptor,
    cancel: CancellationToken,
) {
    loop {
        let accepted = tokio::select! { biased; _ = cancel.cancelled() => break, accepted = listener.accept() => accepted };
        let (socket, _) = match accepted {
            Ok(v) => v,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(slot) = remote.slots.clone().try_acquire_owned() else {
            continue;
        };
        let slot = Arc::new(slot);
        let remote = remote.clone();
        let tls = tls.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let Ok(Ok(socket)) =
                tokio::time::timeout(Duration::from_secs(5), tls.accept(socket)).await
            else {
                return;
            };
            let service = service_fn(move |request| handle(remote.clone(), request, slot.clone()));
            let mut builder = http1::Builder::new();
            builder
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(10));
            let connection = builder
                .serve_connection(TokioIo::new(socket), service)
                .with_upgrades();
            tokio::select! { _ = cancel.cancelled() => {}, _ = connection => {} }
        });
    }
}
async fn handle(
    remote: Arc<Remote>,
    mut request: Request<Incoming>,
    slot: Arc<tokio::sync::OwnedSemaphorePermit>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    // Pairing and wire credentials belong to native clients. Browser origins
    // cannot spend them, and no CORS or query-token route is offered here.
    if request.headers().contains_key("origin") || request.uri().query().is_some() {
        return Ok(error(StatusCode::FORBIDDEN, "native_clients_only"));
    }
    if request.method() == "POST" {
        let outcome = match request.uri().path() {
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
    if let Some(persona_id) = computer_path(request.uri().path()).map(str::to_owned) {
        return Ok(computer_door(remote, request, slot, &persona_id).await);
    }
    if request.uri().path() != "/ws" {
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
        let _slot = slot;
        let Ok(Ok(upgraded)) = tokio::time::timeout(Duration::from_secs(5), upgraded).await else {
            return;
        };
        if phone.cancel.is_cancelled() {
            return;
        }
        let config = WebSocketConfig::default()
            .max_message_size(Some(65_536))
            .max_frame_size(Some(65_536));
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
    slot: Arc<tokio::sync::OwnedSemaphorePermit>,
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
        let _slot = slot;
        let Ok(Ok(upgraded)) = tokio::time::timeout(Duration::from_secs(5), upgraded).await else {
            return;
        };
        if phone.cancel.is_cancelled() {
            return;
        }
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
