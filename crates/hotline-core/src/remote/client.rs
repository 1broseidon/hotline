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
use ts_rs::TS;

type Socket = WebSocketStream<TlsStream<TcpStream>>;
pub(super) type Connection = Channel<TlsStream<TcpStream>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedDesk {
    pub desk_id: String,
    pub name: String,
    pub url: String,
    pub desk_key: String,
    /// The desk's address on its relay, tried when its own does not answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<String>,
    /// The person's public address for the desk, tried after its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public: Option<String>,
    /// The person declined keeping this server as a relay only (ADR 0006
    /// § Migration), so it is not offered again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub relay_declined: bool,
}

/// A server this computer pairs with only to stand in on its relay (ADR
/// 0006). It is not a desk: its grant opens no wire, so it has no bridge and
/// is never in `desks.json`, where an older window would list it and dial
/// its `/v2`. The shell keeps these in `relays.json` beside that file.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct PairedRelay {
    pub desk_id: String,
    pub name: String,
    pub url: String,
    pub desk_key: String,
    /// The person's public address for the server, tried after its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub public: Option<String>,
}
impl PairedRelay {
    /// The server as the sealed dialer reaches it: its own address, then its
    /// public one. A relay is not reached through another relay.
    pub(super) fn dialed(&self) -> PairedDesk {
        PairedDesk {
            desk_id: self.desk_id.clone(),
            name: self.name.clone(),
            url: self.url.clone(),
            desk_key: self.desk_key.clone(),
            relay: None,
            public: self.public.clone(),
            relay_declined: false,
        }
    }
}

/// What a claimed invitation made of this computer: a device of a desk, or
/// a stand-in on a server's relay.
#[derive(Debug)]
pub enum Paired {
    Desk(PairedDesk),
    Relay(PairedRelay),
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

/// Claims an invitation as this computer: a desk's, or a relay's.
pub async fn pair(payload: &PairingPayload, device_name: &str) -> Result<Paired, String> {
    Client::default().claim(payload, device_name).await
}

impl Client {
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    /// Claims a desk's invitation. A relay's is refused before it is spent.
    pub async fn pair(
        &self,
        payload: &PairingPayload,
        device_name: &str,
    ) -> Result<PairedDesk, String> {
        if payload.role == crate::contract::DeviceRole::Relay {
            return Err("This invitation is for using the server as a relay.".into());
        }
        match self.claim(payload, device_name).await? {
            Paired::Desk(desk) => Ok(desk),
            Paired::Relay(_) => Err("The desk granted a different pairing role.".into()),
        }
    }

    /// Claims a relay's invitation. Any other is refused before it is spent.
    pub async fn pair_relay(
        &self,
        payload: &PairingPayload,
        device_name: &str,
    ) -> Result<PairedRelay, String> {
        if payload.role != crate::contract::DeviceRole::Relay {
            return Err("This invitation is not for a relay.".into());
        }
        match self.claim(payload, device_name).await? {
            Paired::Relay(relay) => Ok(relay),
            Paired::Desk(_) => Err("The desk granted a different pairing role.".into()),
        }
    }

    async fn claim(&self, payload: &PairingPayload, device_name: &str) -> Result<Paired, String> {
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
            let _guard = super::lock(&FIRST_KEY);
            if let Some(bytes) = self.store.get(&slot).map_err(storage)? {
                read_identity(&bytes, &desk)?
            } else {
                let (private, _) = sealed::keypair()?;
                let identity = Identity { private, desk };
                // Persist before consuming the single-use invitation. A dropped
                // reply can then be recovered by pairing the same device again.
                self.store
                    .set(&slot, &serde_json::to_vec(&identity).map_err(storage)?)
                    .map_err(storage)?;
                identity
            }
        };
        let mut claim = json!({"secret":payload.secret,"name":device_name.trim()});
        // Only a relay invitation is answered with a purpose: an older server
        // never issues one, and refuses a claim with a field it does not know.
        if payload.role == crate::contract::DeviceRole::Relay {
            claim["purpose"] = json!("relay");
        }
        let claim = claim.to_string();
        let addresses = [
            Some(payload.url.as_str()),
            payload.public.as_deref(),
            payload.relay.as_deref(),
        ];
        let (_, reply) = handshake_first(addresses, "v2/pair", &identity, &claim).await?;
        let reply: Value = serde_json::from_slice(&reply)
            .map_err(|_| "The desk sent an invalid pairing reply.")?;
        match reply["refused"].as_str() {
            Some("relay") => {
                return Err("This invitation is for using the server as a relay.".into());
            }
            Some(_) => return Err("This invitation is not for a relay.".into()),
            None => {}
        }
        let desk_id = reply["deskId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Update the desk to support desktop pairing.")?;
        if reply["role"] != serde_json::to_value(payload.role).map_err(storage)? {
            return Err("The desk granted a different pairing role.".into());
        }
        let name = reply["deskName"].as_str().unwrap_or(&payload.name).into();
        if payload.role == crate::contract::DeviceRole::Relay {
            return Ok(Paired::Relay(PairedRelay {
                desk_id: desk_id.into(),
                name,
                url: payload.url.clone(),
                desk_key: payload.desk_key.clone(),
                public: payload.public.clone(),
            }));
        }
        Ok(Paired::Desk(PairedDesk {
            desk_id: desk_id.into(),
            name,
            url: payload.url.clone(),
            desk_key: payload.desk_key.clone(),
            relay: payload.relay.clone(),
            public: payload.public.clone(),
            relay_declined: false,
        }))
    }

    /// Moves this device's owner grant on `desk` to a relay grant, over its
    /// own wire (ADR 0006 § Migration), and answers the server as a relay.
    /// The server ends this device's sessions as it moves the grant, which
    /// may cut the answer off; then a `/v2` refused for its role is the
    /// answer, since only the move makes an owner key refused there.
    pub async fn become_relay(&self, desk: &PairedDesk) -> Result<PairedRelay, String> {
        let mut wire = self
            .open(desk, None)
            .await
            .map_err(|error| error.to_string())?;
        let hello = match tokio::time::timeout(Duration::from_secs(5), wire.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<Value>(&text).ok(),
            _ => None,
        }
        .ok_or("The server did not say hello.")?;
        let serves = serves(&hello);
        if !serves.iter().any(|capability| capability == "relay") {
            return Err("That server does not run a relay.".into());
        }
        let command = json!({"id": 1, "cmd": "remote.become_relay", "params": {}});
        wire.send(Message::text(command.to_string()))
            .await
            .map_err(|_| "The server disconnected.")?;
        let answer = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(Ok(message)) = wire.next().await {
                if let Message::Text(text) = message
                    && let Ok(reply) = serde_json::from_str::<Value>(&text)
                    && reply["id"] == 1
                {
                    return Some(reply);
                }
            }
            None
        })
        .await
        .ok()
        .flatten();
        drop(wire);
        match answer {
            Some(reply) if reply["ok"] == true => {}
            Some(reply) => {
                return Err(reply["error"]
                    .as_str()
                    .unwrap_or("The server refused that.")
                    .into());
            }
            None => match self.open(desk, None).await {
                Err(OpenError::NotPermitted) => {}
                _ => {
                    return Err(
                        "The server did not say whether it kept this computer as a relay. Try again."
                            .into(),
                    );
                }
            },
        }
        Ok(PairedRelay {
            desk_id: desk.desk_id.clone(),
            name: desk.name.clone(),
            url: desk.url.clone(),
            desk_key: desk.desk_key.clone(),
            public: desk.public.clone(),
        })
    }

    pub(super) async fn open(
        &self,
        desk: &PairedDesk,
        persona: Option<&str>,
    ) -> Result<Connection, OpenError> {
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
        self.sealed(desk, &path, &payload).await
    }

    /// Stands in for this computer's own desk, `desk_id`, on `desk`'s relay.
    pub(super) async fn host_relay(
        &self,
        desk: &PairedDesk,
        desk_id: &str,
    ) -> Result<Connection, OpenError> {
        let payload = json!({"purpose":"relay","deskId":desk_id}).to_string();
        self.sealed(desk, "v2/relay", &payload).await
    }

    pub(super) async fn accept_relay(
        &self,
        desk: &PairedDesk,
        capability: &str,
    ) -> Result<Socket, OpenError> {
        let payload = json!({"purpose":"relay-accept","capability":capability}).to_string();
        Ok(self
            .sealed(desk, "v2/relay/accept", &payload)
            .await?
            .into_socket())
    }

    /// Opens `desk`'s link door as this computer's own desk, `desk_id`, with
    /// that desk's identity key (ADR 0003). The other desk's link record
    /// trusts that key, not this device's: a link is between two desks.
    pub(super) async fn link(
        &self,
        desk: &PairedDesk,
        private: &[u8; 32],
        desk_id: &str,
    ) -> Result<Connection, OpenError> {
        let identity = Identity {
            private: *private,
            desk: decode_key(&desk.desk_key)?,
        };
        let payload = json!({"purpose":"link","deskId":desk_id}).to_string();
        sealed_as(desk, &identity, "v2/link", &payload).await
    }

    async fn sealed(
        &self,
        desk: &PairedDesk,
        path: &str,
        payload: &str,
    ) -> Result<Connection, OpenError> {
        let key = decode_key(&desk.desk_key)?;
        let bytes = self
            .store
            .get(&identity_slot(&desk.desk_key))
            .map_err(storage)?
            .ok_or("This desk's device key is missing. Pair this device again.")?;
        let identity = read_identity(&bytes, &key)?;
        sealed_as(desk, &identity, path, payload).await
    }
}

/// A sealed socket to `desk` under `identity`, through its public address
/// and then its relay when its own address does not answer.
async fn sealed_as(
    desk: &PairedDesk,
    identity: &Identity,
    path: &str,
    payload: &str,
) -> Result<Connection, OpenError> {
    let addresses = [
        Some(desk.url.as_str()),
        desk.public.as_deref(),
        desk.relay.as_deref(),
    ];
    let (channel, response) = handshake_first(addresses, path, identity, payload).await?;
    // handshake() returns this payload only after Noise verifies the
    // pinned responder. Network/TLS/unauthenticated failures cannot revoke.
    if response == sealed::DEVICE_REJECTED {
        return Err(OpenError::Revoked);
    }
    // The key is known and its grant stands; this door is not its role's.
    if response == sealed::ROLE_REFUSED {
        return Err(OpenError::NotPermitted);
    }
    if !response.is_empty() {
        return Err("The desk refused this session.".into());
    }
    Ok(channel)
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    #[test]
    fn relay_addresses_match_the_phones_path_and_security_rules() {
        assert_eq!(
            relay_endpoint(" https://RELAY.example:443/room.v2/relay/desk-1/// "),
            Some("https://relay.example/room.v2/relay/desk-1".into())
        );
        for invalid in [
            "https://desk.example",
            "http://relay.example/relay/desk",
            "https://user@relay.example/relay/desk",
            "https://relay.example/relay/desk?token=t",
            "https://relay.example/relay/desk#fragment",
            "https://relay.example/relay/desk/v2",
            "https://relay.example/relay/desk_1",
            "https://relay.example//relay/desk",
            "https://relay.example/base%20path/relay/desk",
        ] {
            assert_eq!(relay_endpoint(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn discovery_requires_the_expected_v2_team_hello_and_can_clear_a_relay() {
        let mut hello = json!({
            "type":"hello", "protocolVersion":2, "desktopId":"desk", "mode":"team",
            "endpoints":["https://desk.example","https://relay.example/base/relay/desk/"]
        });
        assert_eq!(
            hello_relay(&hello.to_string(), "desk").unwrap().relay,
            Some(Some("https://relay.example/base/relay/desk".into()))
        );
        assert!(hello_relay(&hello.to_string(), "other-desk").is_none());
        for (field, value) in [
            ("protocolVersion", json!(1)),
            ("type", json!("reply")),
            ("mode", json!("computer")),
        ] {
            let mut malformed = hello.clone();
            malformed[field] = value;
            assert!(hello_relay(&malformed.to_string(), "desk").is_none());
        }
        hello["endpoints"] = json!(["https://desk.example"]);
        assert_eq!(
            hello_relay(&hello.to_string(), "desk").unwrap().relay,
            Some(None)
        );
        hello["endpoints"] = json!([]);
        assert_eq!(
            hello_relay(&hello.to_string(), "desk").unwrap().relay,
            Some(None)
        );
    }

    #[test]
    fn malformed_or_missing_endpoints_do_not_clear_a_known_relay() {
        let mut hello = json!({
            "type":"hello", "protocolVersion":2, "desktopId":"desk", "mode":"team"
        });
        assert!(
            hello_relay(&hello.to_string(), "desk")
                .unwrap()
                .relay
                .is_none()
        );
        for endpoints in [
            json!(null),
            json!("https://relay.example/relay/desk"),
            json!([null]),
            json!(["https://desk.example", 42]),
            json!(["http://relay.example/relay/desk"]),
            json!(["https://relay.example/relay/desk?token=secret"]),
        ] {
            hello["endpoints"] = endpoints;
            assert!(
                hello_relay(&hello.to_string(), "desk")
                    .unwrap()
                    .relay
                    .is_none()
            );
        }
    }
}

#[derive(Debug)]
pub(super) enum OpenError {
    Unreachable(String),
    Revoked,
    /// The grant stands but does not open this door: a relay grant at the
    /// wire. Never a reason to forget a pairing.
    NotPermitted,
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
            Self::NotPermitted => {
                f.write_str("This computer is paired with that server only to use its relay.")
            }
        }
    }
}

/// The capabilities a pair answer or a hello says the server runs.
fn serves(said: &Value) -> Vec<String> {
    said["serves"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
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

/// The same shape accepted by the phone: HTTPS, an optional base path, and
/// `/relay/<deskId>`. Discovery comes from Noise, never from TLS or a redirect.
fn relay_endpoint(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.len() > 512 {
        return None;
    }
    endpoint(raw, "v2").ok()?;
    let url = url::Url::parse(raw).ok()?;
    let path = url.path().trim_end_matches('/');
    let parts: Vec<_> = path.strip_prefix('/')?.split('/').collect();
    let (id, base) = parts.split_last()?;
    let (relay, prefix) = base.split_last()?;
    if *relay != "relay"
        || id.is_empty()
        || id.len() > 64
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || !prefix.iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
        })
    {
        return None;
    }
    Some(format!("{}{path}", url.origin().ascii_serialization()))
}

struct Hello {
    /// None means no valid discovery; Some(None) explicitly clears a relay.
    relay: Option<Option<String>>,
}

fn hello_relay(text: &str, desk_id: &str) -> Option<Hello> {
    let hello: Value = serde_json::from_str(text).ok()?;
    if hello["type"] != "hello"
        || hello["protocolVersion"] != 2
        || hello["desktopId"] != desk_id
        || hello["mode"] != "team"
    {
        return None;
    }
    let relay = hello["endpoints"]
        .as_array()
        .filter(|endpoints| {
            endpoints.iter().all(|value| {
                value
                    .as_str()
                    .is_some_and(|url| url.len() <= 512 && endpoint(url.trim(), "v2").is_ok())
            })
        })
        .map(|endpoints| {
            endpoints
                .iter()
                .filter_map(Value::as_str)
                .find_map(relay_endpoint)
        });
    Some(Hello { relay })
}

/// A WebSocket to `base`/`path` over TLS that accepts any certificate: the
/// trust comes from the pinned Noise handshake inside it. Relay capabilities
/// also travel only in an authenticated Noise claim.
pub(super) async fn dial(base: &str, path: &str) -> Result<Socket, String> {
    let url = endpoint(base, path)?;
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
    let name = rustls::pki_types::ServerName::try_from(host.trim_matches(['[', ']']).to_owned())
        .map_err(|_| "Invalid desk hostname.")?;
    let stream = TlsConnector::from(Arc::new(config))
        .connect(name, stream)
        .await
        .map_err(|_| "Could not open TLS to the desk.")?;
    let mut target = url.clone();
    target.set_scheme("wss").map_err(|()| "Invalid desk URL.")?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(65535))
        .max_frame_size(Some(65535));
    let (socket, _) =
        tokio_tungstenite::client_async_with_config(target.as_str(), stream, Some(config))
            .await
            .map_err(|_| "The desk refused the connection.")?;
    Ok(socket)
}

/// The first of `addresses` that completes a handshake, in order; the last
/// failure when none does.
async fn handshake_first(
    addresses: [Option<&str>; 3],
    path: &str,
    identity: &Identity,
    payload: &str,
) -> Result<(Connection, Vec<u8>), String> {
    let mut failure = String::new();
    for base in addresses.into_iter().flatten() {
        match handshake(base, path, identity, payload).await {
            Ok(opened) => return Ok(opened),
            Err(error) => failure = error,
        }
    }
    Err(failure)
}

async fn handshake(
    base: &str,
    path: &str,
    identity: &Identity,
    payload: &str,
) -> Result<(Connection, Vec<u8>), String> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut socket = dial(base, path).await?;
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

/// A stamp is assigned when the pinned hello is authenticated, before any
/// bounded incoming queue or local writer can delay its bridge's publication.
#[derive(Clone)]
pub(super) struct Discovery {
    pub(super) sequence: u64,
    pub(super) relay: Option<String>,
}

/// Bounded queues apply backpressure. Only subscriptions survive a reconnect;
/// commands interrupted by a disconnect get an uncertain-outcome error.
pub struct Session {
    pub outgoing: tokio::sync::mpsc::Sender<String>,
    pub incoming: tokio::sync::mpsc::Receiver<String>,
    pub state: tokio::sync::watch::Receiver<State>,
    /// The desk's latest relay, learned only from a pinned, authenticated hello.
    pub relay: tokio::sync::watch::Receiver<Option<String>>,
    pub(super) discovery: tokio::sync::watch::Receiver<Option<Discovery>>,
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
        let (relay, endpoints) = tokio::sync::watch::channel(desk.relay.clone());
        let (discovery, discovered) = tokio::sync::watch::channel(None);
        let cancel = tokio_util::sync::CancellationToken::new();
        let client = self.clone();
        let desk = desk.clone();
        let stopped = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = stopped.cancelled() => {},
                () = client.reconnect(desk, outgoing, incoming, state, relay, discovery) => {},
            }
        });
        Session {
            outgoing: send,
            incoming: receive,
            state: watch,
            relay: endpoints,
            discovery: discovered,
            cancel,
        }
    }

    async fn reconnect(
        &self,
        mut desk: PairedDesk,
        mut outgoing: tokio::sync::mpsc::Receiver<String>,
        incoming: tokio::sync::mpsc::Sender<String>,
        state: tokio::sync::watch::Sender<State>,
        relay: tokio::sync::watch::Sender<Option<String>>,
        discovery: tokio::sync::watch::Sender<Option<Discovery>>,
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
                    && let Some(hello) = hello_relay(&text, &desk.desk_id)
                {
                    // This frame has passed the pinned Noise handshake and
                    // record authentication. Keep it for this session's next
                    // reconnect as well as for the shell's persistent registry.
                    if let Some(discovered) = hello.relay {
                        static SEQUENCE: std::sync::atomic::AtomicU64 =
                            std::sync::atomic::AtomicU64::new(1);
                        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        desk.relay.clone_from(&discovered);
                        relay.send_if_modified(|known| {
                            if *known == discovered {
                                false
                            } else {
                                known.clone_from(&discovered);
                                true
                            }
                        });
                        discovery.send_replace(Some(Discovery {
                            sequence,
                            relay: discovered,
                        }));
                    }
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
                    () = &mut sleep => break,
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
