//! A desk is authenticated by its pinned Noise identity, independently of TLS.
//! Secrets never belong in the shell's desk registry or the loopback window.
use super::{PairingPayload, channel::Channel, sealed};
use crate::credentials::{NativeStore, SecretStore};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::net::TcpStream;
use tokio_rustls::{TlsConnector, client::TlsStream, rustls};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, protocol::WebSocketConfig},
};

type Socket = WebSocketStream<TlsStream<TcpStream>>;
pub(super) type Connection = Channel<TlsStream<TcpStream>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedDesk {
    pub desk_id: String,
    pub name: String,
    pub url: String,
    pub desk_key: String,
}

#[derive(Serialize, Deserialize)]
struct Identity {
    private: [u8; 32],
    desk: [u8; 32],
}

/// Injectable storage keeps harnesses and client-only shells independent of a room.
#[derive(Clone)]
pub struct Client {
    store: Arc<dyn SecretStore>,
}
impl Default for Client {
    fn default() -> Self {
        Self::new(Arc::new(NativeStore))
    }
}

pub async fn pair(payload: &PairingPayload, device_name: &str) -> Result<PairedDesk, String> {
    Client::default().pair(payload, device_name).await
}

impl Client {
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    pub async fn pair(
        &self,
        payload: &PairingPayload,
        device_name: &str,
    ) -> Result<PairedDesk, String> {
        if payload.version != 2 {
            return Err("This pairing invitation needs a newer Hotline.".into());
        }
        if payload.expires_at <= super::now() {
            return Err("This pairing invitation has expired. Open a new one on the desk.".into());
        }
        if device_name.trim().is_empty() || device_name.len() > 80 {
            return Err("Choose a device name of 1–80 bytes.".into());
        }
        let desk = decode_key(&payload.desk_key)?;
        endpoint(&payload.url, "v2/pair")?;
        if URL_SAFE_NO_PAD
            .decode(&payload.secret)
            .map_or(true, |s| s.len() != 32)
        {
            return Err("Invalid pairing secret.".into());
        }
        let slot = identity_slot(&payload.desk_key);
        let identity = {
            // SecretStore exposes individual operations, not compare-and-set.
            // Keep concurrent first claims in this client process on one key.
            static FIRST_KEY: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _guard = FIRST_KEY
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match self.store.get(&slot).map_err(storage)? {
                Some(bytes) => read_identity(&bytes, &desk)?,
                None => {
                    let (private, _) = sealed::keypair()?;
                    let identity = Identity { private, desk };
                    // Persist before consuming the single-use invitation. A dropped
                    // reply can then be recovered by pairing the same device again.
                    self.store
                        .set(&slot, &serde_json::to_vec(&identity).map_err(storage)?)
                        .map_err(storage)?;
                    identity
                }
            }
        };
        let claim = json!({"secret":payload.secret,"name":device_name.trim()});
        let (_, reply) = handshake(&payload.url, "v2/pair", &identity, &claim.to_string()).await?;
        let reply: Value = serde_json::from_slice(&reply)
            .map_err(|_| "The desk sent an invalid pairing reply.")?;
        let desk_id = reply["deskId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Update the desk to support desktop pairing.")?;
        if reply["role"] != serde_json::to_value(payload.role).map_err(storage)? {
            return Err("The desk granted a different pairing role.".into());
        }
        Ok(PairedDesk {
            desk_id: desk_id.into(),
            name: reply["deskName"].as_str().unwrap_or(&payload.name).into(),
            url: payload.url.clone(),
            desk_key: payload.desk_key.clone(),
        })
    }

    pub(super) async fn open(
        &self,
        desk: &PairedDesk,
        persona: Option<&str>,
    ) -> Result<Connection, OpenError> {
        let key = decode_key(&desk.desk_key)?;
        let bytes = self
            .store
            .get(&identity_slot(&desk.desk_key))
            .map_err(storage)?
            .ok_or("This desk's device key is missing. Pair this device again.")?;
        let identity = read_identity(&bytes, &key)?;
        let (path, payload) = match persona {
            Some(id) => {
                if id.is_empty()
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    return Err("Invalid teammate ID.".into());
                }
                (
                    format!("v2/computer/{id}/ws"),
                    json!({"purpose":"computer","personaId":id}).to_string(),
                )
            }
            None => ("v2".into(), String::new()),
        };
        let (channel, response) = handshake(&desk.url, &path, &identity, &payload).await?;
        // handshake() returns this payload only after Noise verifies the
        // pinned responder. Network/TLS/unauthenticated failures cannot revoke.
        if response == sealed::DEVICE_REJECTED {
            return Err(OpenError::Revoked);
        }
        if !response.is_empty() {
            return Err("The desk refused this session.".into());
        }
        Ok(channel)
    }
}

#[derive(Debug)]
pub(super) enum OpenError {
    Unreachable(String),
    Revoked,
}
impl From<String> for OpenError {
    fn from(message: String) -> Self {
        Self::Unreachable(message)
    }
}
impl From<&str> for OpenError {
    fn from(message: &str) -> Self {
        Self::Unreachable(message.into())
    }
}
impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(message) => f.write_str(message),
            Self::Revoked => f.write_str("This device is no longer authorized. Pair it again."),
        }
    }
}

fn storage(error: impl std::fmt::Display) -> String {
    format!("Could not access this device's secret store: {error}")
}
fn identity_slot(key: &str) -> String {
    format!("remote-client-{}", super::hash(key))
}
fn read_identity(bytes: &[u8], desk: &[u8; 32]) -> Result<Identity, String> {
    let identity: Identity = serde_json::from_slice(bytes)
        .map_err(|_| "The saved device identity is unreadable. Restore its secret store.")?;
    if &identity.desk != desk {
        return Err("The saved desk identity does not match. Pair again explicitly.".into());
    }
    Ok(identity)
}
fn decode_key(key: &str) -> Result<[u8; 32], String> {
    URL_SAFE_NO_PAD
        .decode(key)
        .ok()
        .and_then(|k| k.try_into().ok())
        .ok_or("Invalid desk identity key.".into())
}
fn endpoint(base: &str, path: &str) -> Result<url::Url, String> {
    let mut url = url::Url::parse(base).map_err(|_| "Invalid desk URL.")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("A desk URL must use HTTPS without credentials, query or fragment.".into());
    }
    url.set_path(&format!("{}/{path}", url.path().trim_end_matches('/')));
    Ok(url)
}

async fn handshake(
    base: &str,
    path: &str,
    identity: &Identity,
    payload: &str,
) -> Result<(Connection, Vec<u8>), String> {
    let url = endpoint(base, path)?;
    tokio::time::timeout(Duration::from_secs(10), async {
        let host = url.host_str().ok_or("Invalid desk URL.")?;
        let stream = TcpStream::connect((
            host.trim_matches(['[', ']']),
            url.port_or_known_default().unwrap_or(443),
        ))
        .await
        .map_err(|_| "The desk is unreachable.")?;
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|_| "TLS is unavailable.")?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoiseIdentity(provider)))
            .with_no_client_auth();
        let name =
            rustls::pki_types::ServerName::try_from(host.trim_matches(['[', ']']).to_owned())
                .map_err(|_| "Invalid desk hostname.")?;
        let stream = TlsConnector::from(Arc::new(config))
            .connect(name, stream)
            .await
            .map_err(|_| "Could not open TLS to the desk.")?;
        let mut target = url.clone();
        target.set_scheme("wss").map_err(|_| "Invalid desk URL.")?;
        let config = WebSocketConfig::default()
            .max_message_size(Some(65535))
            .max_frame_size(Some(65535));
        let (mut socket, _): (Socket, _) =
            tokio_tungstenite::client_async_with_config(target.as_str(), stream, Some(config))
                .await
                .map_err(|_| "The desk refused the connection.")?;
        let mut noise = sealed::initiator(&identity.private, &identity.desk)?;
        let mut buffer = [0; 4096];
        let n = noise
            .write_message(payload.as_bytes(), &mut buffer)
            .map_err(|_| "Could not seal the handshake.")?;
        socket
            .send(Message::Binary(buffer[..n].to_vec().into()))
            .await
            .map_err(|_| "The desk disconnected.")?;
        let Some(Ok(Message::Binary(bytes))) = socket.next().await else {
            return Err("The desk did not authenticate this device.".into());
        };
        let n = noise
            .read_message(&bytes, &mut buffer)
            .map_err(|_| "The desk identity did not match its pinned key.")?;
        let reply = buffer[..n].to_vec();
        let transport = noise
            .into_transport_mode()
            .map_err(|_| "The sealed handshake did not finish.")?;
        Ok((Channel::new(socket, transport), reply))
    })
    .await
    .map_err(|_| "The desk did not answer within ten seconds.".to_string())?
}

// TLS still verifies possession of the certificate key. The pinned Noise key,
// checked before any application frame, supplies server authentication. This
// supports self-signed desks and TLS proxies without trusting their plaintext.
#[derive(Debug)]
struct NoiseIdentity(Arc<rustls::crypto::CryptoProvider>);
impl rustls::client::danger::ServerCertVerifier for NoiseIdentity {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signature,
            &self.0.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &self.0.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Connecting,
    Open,
    Unreachable,
    Revoked,
}

/// Bounded queues apply backpressure. Only subscriptions survive a reconnect;
/// commands interrupted by a disconnect get an uncertain-outcome error.
pub struct Session {
    pub outgoing: tokio::sync::mpsc::Sender<String>,
    pub incoming: tokio::sync::mpsc::Receiver<String>,
    pub state: tokio::sync::watch::Receiver<State>,
    cancel: tokio_util::sync::CancellationToken,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl Client {
    pub fn connect(&self, desk: &PairedDesk) -> Session {
        let (send, outgoing) = tokio::sync::mpsc::channel(64);
        let (incoming, receive) = tokio::sync::mpsc::channel(64);
        let (state, watch) = tokio::sync::watch::channel(State::Connecting);
        let cancel = tokio_util::sync::CancellationToken::new();
        let client = self.clone();
        let desk = desk.clone();
        let stopped = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = stopped.cancelled() => {},
                _ = client.reconnect(desk, outgoing, incoming, state) => {},
            }
        });
        Session {
            outgoing: send,
            incoming: receive,
            state: watch,
            cancel,
        }
    }

    async fn reconnect(
        &self,
        desk: PairedDesk,
        mut outgoing: tokio::sync::mpsc::Receiver<String>,
        incoming: tokio::sync::mpsc::Sender<String>,
        state: tokio::sync::watch::Sender<State>,
    ) {
        let mut subscriptions = std::collections::BTreeMap::<i64, String>::new();
        let mut delay = Duration::from_millis(250);
        loop {
            state.send_replace(State::Connecting);
            let opened = match self.open(&desk, None).await {
                Err(OpenError::Revoked) => {
                    state.send_replace(State::Revoked);
                    return;
                }
                other => other,
            };
            if let Ok(mut socket) = opened {
                let hello = tokio::time::timeout(Duration::from_secs(5), socket.next()).await;
                if let Ok(Some(Ok(Message::Text(text)))) = hello
                    && serde_json::from_str::<Value>(&text)
                        .is_ok_and(|h| h["type"] == "hello" && h["desktopId"] == desk.desk_id)
                {
                    state.send_replace(State::Open);
                    delay = Duration::from_millis(250);
                    if incoming.send(text.to_string()).await.is_err() {
                        return;
                    }
                    let mut replayed = true;
                    for frame in subscriptions.values() {
                        if socket.send(Message::text(frame)).await.is_err() {
                            replayed = false;
                            break;
                        }
                    }
                    let mut pending = std::collections::BTreeSet::new();
                    loop {
                        if !replayed {
                            break;
                        }
                        tokio::select! {
                            frame = outgoing.recv() => {
                                let Some(frame) = frame else { return; };
                                if let Ok(value) = serde_json::from_str::<Value>(&frame) {
                                    if let Some(id) = value["id"].as_i64() {
                                        if value.get("sub").is_some() { subscriptions.insert(id, frame.clone()); }
                                        if value.get("sub").is_none() { pending.insert(id); }
                                    }
                                    if let Some(id) = value["unsub"].as_i64() { subscriptions.remove(&id); }
                                }
                                if socket.send(Message::text(frame)).await.is_err() { break; }
                            }
                            frame = socket.next() => {
                                let Some(Ok(Message::Text(text))) = frame else { break; };
                                if let Ok(value) = serde_json::from_str::<Value>(&text)
                                    && let Some(id) = value["id"].as_i64() {
                                        pending.remove(&id);
                                        if value["ok"] == false { subscriptions.remove(&id); }
                                    }
                                if incoming.send(text.to_string()).await.is_err() { return; }
                            }
                        }
                    }
                    for id in pending {
                        if incoming.send(json!({"id":id,"ok":false,"error":"The desk disconnected. This action may have completed; check before trying again."}).to_string()).await.is_err() { return; }
                    }
                }
            }
            state.send_replace(State::Unreachable);
            let sleep = tokio::time::sleep(delay);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    _ = &mut sleep => break,
                    frame = outgoing.recv() => {
                        let Some(frame) = frame else { return; };
                        if let Ok(value) = serde_json::from_str::<Value>(&frame) {
                            if let Some(id) = value["unsub"].as_i64() { subscriptions.remove(&id); }
                            if let Some(id) = value["id"].as_i64()
                                && incoming.send(json!({"id":id,"ok":false,"error":"The desk is unreachable. Try again when it reconnects."}).to_string()).await.is_err() { return; }
                        }
                    }
                }
            }
            delay = (delay * 2).min(Duration::from_secs(30));
        }
    }
}
