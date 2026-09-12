use super::*;
use bytes::Bytes;
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
        handshake::server::create_response,
        protocol::{Role, WebSocketConfig},
    },
};

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
    if request.method() != "GET" || request.uri().path() != "/ws" {
        return Ok(error(StatusCode::NOT_FOUND, "not_found"));
    }
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let Some(phone) = remote.authenticate(token) else {
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
