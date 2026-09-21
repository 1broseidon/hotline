//! A paired phone enters the real wire through a separate, opt-in TLS listener.
//! Its identity key uses the same OS credential store as provider credentials.
mod attachments;
mod network;
mod pake;
mod server;

use crate::contract::MobileAttachmentChunk;
use crate::credentials::{CredentialFile, CredentialFiles, SecretStore, atomic_write};
use crate::log::Log;
use crate::wire::RoomHandle;
use curve25519_dalek::scalar::Scalar;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs, io,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;
use uuid::Uuid;

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
fn secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}
/// Six uniform decimal digits, leading zeros kept.
fn pairing_code() -> String {
    loop {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes).expect("the OS random source is available");
        let value = u32::from_le_bytes(bytes);
        // Reject the top of the range so the modulus is unbiased.
        if value < 4_294_000_000 {
            return format!("{:06}", value % 1_000_000);
        }
    }
}

#[derive(Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct RemoteDevice {
    pub id: String,
    pub name: String,
    pub paired_at: i64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Grant {
    device: RemoteDevice,
    token_hash: String,
    /// Where a notification for this phone goes, once it has said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    push: Option<PushTarget>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PushTarget {
    token: String,
    platform: String,
}
/// The phones a notification goes to, read off the saved grants so the room
/// never has to hold the remote.
pub struct PushTargets {
    pub desktop_id: String,
    pub tokens: Vec<String>,
}
pub fn push_targets(root: &Path) -> PushTargets {
    let saved: Saved = fs::read(root.join("remote.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    PushTargets {
        desktop_id: saved.desktop_id,
        tokens: saved
            .grants
            .iter()
            .filter_map(|grant| grant.push.as_ref().map(|push| push.token.clone()))
            .collect(),
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    desktop_id: String,
    host: String,
    #[serde(default)]
    port: u16,
    enabled: bool,
    grants: Vec<Grant>,
}
#[derive(Serialize, Deserialize)]
struct Identity {
    #[serde(default)]
    hosts: Vec<String>,
    certificate: Vec<u8>,
    key: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct PairingInvitation {
    pub kind: String,
    pub version: u8,
    pub desktop_id: String,
    pub name: String,
    pub endpoint: String,
    pub certificate_sha256: String,
    pub invitation_id: String,
    pub secret: String,
    pub expires_at: i64,
}
/// What an operator types into a phone that cannot scan: where the desktop
/// is, and a six-digit code that is the password of a PAKE, never a secret
/// sent on the wire.
#[derive(Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct ManualPairing {
    pub address: String,
    pub port: u16,
    pub code: String,
    pub expires_at: i64,
}
#[derive(Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct RemotePairing {
    pub invitation: PairingInvitation,
    pub qr_svg: String,
    pub manual: ManualPairing,
}
#[derive(Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct RemoteStatus {
    pub enabled: bool,
    pub host: String,
    pub endpoint: Option<String>,
    pub endpoints: Vec<String>,
    pub addresses: Vec<String>,
    pub devices: Vec<RemoteDevice>,
    pub error: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Claim {
    invitation_id: String,
    secret: String,
    claim_id: String,
    name: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManualStart {
    claim_id: String,
    name: String,
    phone_public: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManualFinish {
    claim_id: String,
    confirm: String,
}
struct Invitation {
    value: PairingInvitation,
    claimed: Option<(String, Value)>,
}
/// One phone's half-finished exchange. A session is one guess: a failed
/// confirmation discards it, so a second guess costs a second start.
struct Session {
    secret: Scalar,
    name: String,
    phone_public: String,
    desktop_public: String,
}
struct Manual {
    code: String,
    expires_at: i64,
    failures: u8,
    sessions: HashMap<String, Session>,
    claimed: Option<(String, Value)>,
}
const MANUAL_ATTEMPTS: u8 = 5;
const MANUAL_SESSIONS: usize = 8;
struct Live {
    saved: Saved,
    endpoints: Vec<String>,
    fingerprint: String,
    invitation: Option<Invitation>,
    manual: Option<Manual>,
    cancel: CancellationToken,
    devices: HashMap<String, CancellationToken>,
    error: Option<String>,
    attempts: u16,
    attempts_since: i64,
}
#[derive(Serialize, Deserialize)]
struct Receipt {
    digest: String,
    result: Option<Result<(), String>>,
}

pub struct Remote {
    root: PathBuf,
    identity: CredentialFile,
    state: Mutex<Live>,
    server: AsyncMutex<Option<tokio::task::JoinHandle<()>>>,
    lifecycle: AsyncMutex<()>,
    prompts: AsyncMutex<()>,
    slots: Arc<Semaphore>,
    log: Log,
    room: Arc<dyn RoomHandle>,
}
#[derive(Clone)]
pub(crate) struct Phone {
    remote: Arc<Remote>,
    id: String,
    pub cancel: CancellationToken,
}
impl Phone {
    pub(crate) async fn prompt(
        &self,
        operation_id: &str,
        persona_id: &str,
        text: &str,
        attachment_ids: &[String],
        reply_to: Option<&str>,
    ) -> Result<Value, String> {
        self.remote
            .prompt(
                self,
                operation_id,
                persona_id,
                text,
                attachment_ids,
                reply_to,
            )
            .await
    }

    /// Remembers where to notify this phone. An Expo push token, which is
    /// what the phone's push service issues on every platform it runs on.
    pub(crate) fn register_push(&self, token: String, platform: String) -> Result<Value, String> {
        let token = token.trim().to_string();
        let expo = (token.starts_with("ExponentPushToken[") || token.starts_with("ExpoPushToken["))
            && token.ends_with(']')
            && token.len() <= 200;
        if !expo || !matches!(platform.as_str(), "ios" | "android") {
            return Err("register a push token Expo issued, for ios or android.".into());
        }
        let mut s = self.remote.state.lock().unwrap();
        if self.cancel.is_cancelled() {
            return Err("This phone has been disconnected.".into());
        }
        let mut saved = s.saved.clone();
        let grant = saved
            .grants
            .iter_mut()
            .find(|g| g.device.id == self.id)
            .ok_or_else(|| "This phone is no longer paired.".to_string())?;
        grant.push = Some(PushTarget { token, platform });
        self.remote.save(&saved)?;
        s.saved = saved;
        Ok(Value::Null)
    }

    pub(crate) async fn upload(&self, upload: &MobileAttachmentChunk) -> Result<Value, String> {
        let _held = self.remote.prompts.lock().await;
        if self.cancel.is_cancelled() {
            return Err("This phone has been disconnected.".into());
        }
        attachments::upload(&self.remote.root, &self.id, upload)
    }
}

impl Remote {
    pub fn open(root: &Path, log: Log, room: Arc<dyn RoomHandle>) -> io::Result<Arc<Self>> {
        Self::open_with_store(root, log, room, crate::credentials::default_store())
    }
    pub fn open_with_store(
        root: &Path,
        log: Log,
        room: Arc<dyn RoomHandle>,
        store: Arc<dyn SecretStore>,
    ) -> io::Result<Arc<Self>> {
        fs::create_dir_all(root)?;
        let mut saved: Saved = match fs::read(root.join("remote.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Saved {
                desktop_id: Uuid::new_v4().to_string(),
                host: network::ALL.into(),
                ..Saved::default()
            },
            Err(e) => return Err(e),
        };
        // Older builds required a single address and defaulted to loopback.
        // A Remote setting must never restore that simulator-only choice.
        if saved.host.is_empty()
            || saved
                .host
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified())
        {
            saved.host = network::ALL.into();
        }
        let identity =
            CredentialFiles::new(root.to_path_buf(), store).file(root.join("remote-identity.json"));
        Ok(Arc::new(Self {
            root: root.to_path_buf(),
            identity,
            state: Mutex::new(Live {
                saved,
                endpoints: Vec::new(),
                fingerprint: String::new(),
                invitation: None,
                manual: None,
                cancel: CancellationToken::new(),
                devices: HashMap::new(),
                error: None,
                attempts: 0,
                attempts_since: now(),
            }),
            server: AsyncMutex::new(None),
            lifecycle: AsyncMutex::new(()),
            prompts: AsyncMutex::new(()),
            slots: Arc::new(Semaphore::new(16)),
            log,
            room,
        }))
    }
    fn save(&self, saved: &Saved) -> Result<(), String> {
        atomic_write(
            &self.root.join("remote.json"),
            &serde_json::to_vec(saved).map_err(message)?,
        )
        .map_err(message)
    }
    pub fn addresses() -> Vec<String> {
        network::addresses()
    }
    #[cfg(test)]
    pub(crate) fn status_desktop_id(&self) -> String {
        self.state.lock().unwrap().saved.desktop_id.clone()
    }
    pub fn status(&self) -> RemoteStatus {
        let s = self.state.lock().unwrap();
        RemoteStatus {
            enabled: !s.endpoints.is_empty(),
            host: s.saved.host.clone(),
            endpoint: s.endpoints.first().cloned(),
            endpoints: s.endpoints.clone(),
            addresses: Self::addresses(),
            devices: s.saved.grants.iter().map(|g| g.device.clone()).collect(),
            error: s.error.clone(),
        }
    }
    pub async fn restore(self: &Arc<Self>) {
        let (enabled, host) = {
            let s = self.state.lock().unwrap();
            (s.saved.enabled, s.saved.host.clone())
        };
        if enabled && let Err(e) = self.configure(true, &host).await {
            self.state.lock().unwrap().error = Some(e);
        }
    }
    pub async fn configure(
        self: &Arc<Self>,
        enabled: bool,
        host: &str,
    ) -> Result<RemoteStatus, String> {
        let _held = self.lifecycle.lock().await;
        let addresses = Self::addresses();
        if host != network::ALL && !host.parse::<IpAddr>().is_ok_and(network::reachable) {
            return Err(
                "Choose all host IPs or a network address. Loopback is not available for Remote."
                    .into(),
            );
        }
        if enabled && host != network::ALL && !addresses.iter().any(|a| a == host) {
            return Err("Choose an address on this computer.".into());
        }
        if enabled && addresses.is_empty() {
            return Err("Connect to a network before enabling Remote.".into());
        }
        let hosts = if host == network::ALL {
            addresses.clone()
        } else {
            vec![host.to_owned()]
        };
        {
            let mut s = self.state.lock().unwrap();
            let endpoints: Vec<_> = hosts
                .iter()
                .map(|host| network::endpoint(host, s.saved.port))
                .collect();
            if enabled
                && !s.endpoints.is_empty()
                && s.saved.host == host
                && s.endpoints == endpoints
            {
                drop(s);
                return Ok(self.status());
            }
            s.cancel.cancel();
            s.endpoints.clear();
            s.invitation = None;
            s.manual = None;
            s.devices.clear();
            s.saved.enabled = false;
            s.saved.host = host.into();
            s.error = None;
            self.save(&s.saved)?;
        }
        if let Some(task) = self.server.lock().await.take() {
            let _ = task.await;
        }
        if !enabled {
            return Ok(self.status());
        }
        let saved_port = self.state.lock().unwrap().saved.port;
        let listener = network::bind(host, saved_port, &addresses)
            .await
            .map_err(message)?;
        let port = listener.local_addr().map_err(message)?.port();
        // The identity is a certificate this desk signed itself, so one it
        // cannot read any more — corrupt, gone from the OS store, or written
        // by an earlier edition into a store this build cannot name, which is
        // what a room moved over from Toad holds — is replaced like a missing
        // one, and the phones that pinned it pair again. A locked or
        // unavailable store is reported instead: the identity is most likely
        // still there, and replacing it would orphan every phone for nothing.
        let identity = match self.identity.read() {
            Ok(Some(bytes)) => serde_json::from_slice::<Identity>(&bytes).ok(),
            Ok(None) => None,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::InvalidData | io::ErrorKind::NotFound
                ) =>
            {
                None
            }
            Err(error) => return Err(message(error)),
        };
        let identity = match identity {
            Some(identity) if hosts.iter().all(|host| identity.hosts.contains(host)) => identity,
            _ => {
                let key = rcgen::KeyPair::generate().map_err(message)?;
                let mut params =
                    rcgen::CertificateParams::new(addresses.clone()).map_err(message)?;
                params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
                params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
                let cert = params.self_signed(&key).map_err(message)?;
                let identity = Identity {
                    hosts: addresses,
                    certificate: cert.der().to_vec(),
                    key: key.serialize_der(),
                };
                self.identity
                    .write(&serde_json::to_vec(&identity).map_err(message)?)
                    .map_err(message)?;
                let mut s = self.state.lock().unwrap();
                s.saved.grants.clear();
                self.save(&s.saved)?;
                identity
            }
        };
        let tls = server::tls(&identity)?;
        let cancel = CancellationToken::new();
        {
            let mut s = self.state.lock().unwrap();
            let mut saved = s.saved.clone();
            saved.host = host.into();
            saved.enabled = true;
            saved.port = port;
            self.save(&saved)?;
            s.saved = saved;
            s.endpoints = hosts
                .iter()
                .map(|host| network::endpoint(host, port))
                .collect();
            s.fingerprint = format!("{:x}", Sha256::digest(&identity.certificate));
            s.cancel = cancel.clone();
            for grant in s.saved.grants.clone() {
                s.devices.insert(grant.device.id, cancel.child_token());
            }
        }
        let this = self.clone();
        *self.server.lock().await = Some(tokio::spawn(async move {
            server::run(this, listener, tls, cancel).await;
        }));
        Ok(self.status())
    }
    pub fn pairing(&self) -> Result<RemotePairing, String> {
        let mut s = self.state.lock().unwrap();
        let endpoint = s
            .endpoints
            .first()
            .cloned()
            .ok_or("Enable remote access first.")?;
        let invitation = PairingInvitation {
            kind: "hotline-pairing".into(),
            version: 1,
            desktop_id: s.saved.desktop_id.clone(),
            name: desktop_name(),
            endpoint,
            certificate_sha256: s.fingerprint.clone(),
            invitation_id: Uuid::new_v4().to_string(),
            secret: secret(),
            expires_at: now() + 120_000,
        };
        let code = qrcode::QrCode::new(serde_json::to_vec(&invitation).map_err(message)?)
            .map_err(message)?;
        let qr_svg = code
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(280, 280)
            .build();
        let (address, port) = network::split(&invitation.endpoint);
        let manual = ManualPairing {
            address,
            port,
            code: pairing_code(),
            expires_at: invitation.expires_at,
        };
        s.invitation = Some(Invitation {
            value: invitation.clone(),
            claimed: None,
        });
        s.manual = Some(Manual {
            code: manual.code.clone(),
            expires_at: manual.expires_at,
            failures: 0,
            sessions: HashMap::new(),
            claimed: None,
        });
        Ok(RemotePairing {
            invitation,
            qr_svg,
            manual,
        })
    }
    pub fn revoke(&self, id: &str) -> Result<RemoteStatus, String> {
        let mut s = self.state.lock().unwrap();
        let mut saved = s.saved.clone();
        saved.grants.retain(|g| g.device.id != id);
        self.save(&saved)?;
        s.saved = saved;
        if let Some(cancel) = s.devices.remove(id) {
            cancel.cancel();
        }
        // An idempotent pairing retry must never recover a revoked grant.
        s.invitation = None;
        s.manual = None;
        drop(s);
        Ok(self.status())
    }
    /// Every pairing request, on any path, spends from one budget.
    fn throttle(s: &mut Live) -> Result<(), &'static str> {
        if now() - s.attempts_since > 60_000 {
            s.attempts = 0;
            s.attempts_since = now();
        }
        s.attempts = s.attempts.saturating_add(1);
        if s.attempts > 30 {
            return Err("rate_limited");
        }
        Ok(())
    }
    fn valid_claim(claim_id: &str, name: &str) -> bool {
        Uuid::parse_str(claim_id).is_ok() && !name.trim().is_empty() && name.len() <= 80
    }
    /// Records a new phone and returns what it needs to enter the wire. The
    /// grant is on disk before the token leaves this function.
    fn grant(&self, s: &mut Live, name: &str) -> Result<Value, &'static str> {
        if s.saved.grants.len() >= 16 {
            return Err("device_limit");
        }
        let token = secret();
        let id = Uuid::new_v4().to_string();
        let grant = Grant {
            device: RemoteDevice {
                id: id.clone(),
                name: name.trim().into(),
                paired_at: now(),
            },
            token_hash: hash(&token),
            push: None,
        };
        s.saved.grants.push(grant);
        if self.save(&s.saved).is_err() {
            s.saved.grants.pop();
            return Err("storage_unavailable");
        }
        let cancellation = s.cancel.child_token();
        s.devices.insert(id.clone(), cancellation);
        Ok(
            json!({"desktopId": s.saved.desktop_id, "protocolVersion": 1, "deviceId": id, "token": token}),
        )
    }
    fn claim(&self, claim: Claim) -> Result<Value, &'static str> {
        let mut s = self.state.lock().unwrap();
        Self::throttle(&mut s)?;
        if !Self::valid_claim(&claim.claim_id, &claim.name) {
            return Err("invalid_claim");
        }
        let invitation = s.invitation.as_ref().ok_or("pairing_closed")?;
        if s.endpoints.is_empty()
            || invitation.value.expires_at <= now()
            || invitation.value.invitation_id != claim.invitation_id
            || !crate::wire::same_secret(&claim.secret, &invitation.value.secret)
        {
            return Err("pairing_closed");
        }
        if let Some((id, value)) = &invitation.claimed {
            return if id == &claim.claim_id {
                Ok(value.clone())
            } else {
                Err("pairing_claimed")
            };
        }
        let answer = self.grant(&mut s, &claim.name)?;
        s.invitation.as_mut().unwrap().claimed = Some((claim.claim_id, answer.clone()));
        // One pairing session, two ways in: the first phone through closes both.
        s.manual = None;
        Ok(answer)
    }
    fn manual_start(&self, start: ManualStart) -> Result<Value, &'static str> {
        let mut s = self.state.lock().unwrap();
        Self::throttle(&mut s)?;
        if !Self::valid_claim(&start.claim_id, &start.name)
            || pake::decode_public(&start.phone_public).is_none()
        {
            return Err("invalid_claim");
        }
        if s.endpoints.is_empty() {
            return Err("pairing_closed");
        }
        let desktop_id = s.saved.desktop_id.clone();
        let manual = s.manual.as_mut().ok_or("pairing_closed")?;
        if manual.claimed.is_some() {
            return Err("pairing_closed");
        }
        if manual.expires_at <= now() {
            return Err("expired");
        }
        let secret = pake::random_scalar();
        let desktop_public = pake::public(&manual.code, &start.claim_id, &secret);
        if manual.sessions.len() >= MANUAL_SESSIONS
            && !manual.sessions.contains_key(&start.claim_id)
        {
            // A flood of starts is noise, not guesses; the real phone retries.
            manual.sessions.clear();
        }
        manual.sessions.insert(
            start.claim_id,
            Session {
                secret,
                name: start.name,
                phone_public: start.phone_public,
                desktop_public: desktop_public.clone(),
            },
        );
        Ok(json!({
            "desktopId": desktop_id,
            "name": desktop_name(),
            "desktopPublic": desktop_public,
            "expiresAt": manual.expires_at,
        }))
    }
    fn manual_finish(&self, finish: ManualFinish) -> Result<Value, &'static str> {
        let mut s = self.state.lock().unwrap();
        Self::throttle(&mut s)?;
        if Uuid::parse_str(&finish.claim_id).is_err() || finish.confirm.len() != 64 {
            return Err("invalid_claim");
        }
        if s.endpoints.is_empty() {
            return Err("pairing_closed");
        }
        let desktop_id = s.saved.desktop_id.clone();
        let fingerprint = s.fingerprint.clone();
        let manual = s.manual.as_mut().ok_or("pairing_closed")?;
        if let Some((id, answer)) = &manual.claimed {
            return if id == &finish.claim_id {
                Ok(answer.clone())
            } else {
                Err("pairing_closed")
            };
        }
        if manual.expires_at <= now() {
            return Err("expired");
        }
        let session = manual
            .sessions
            .get(&finish.claim_id)
            .ok_or("pairing_closed")?;
        let tags = pake::decode_public(&session.phone_public)
            .and_then(|phone| {
                pake::confirmations(
                    &desktop_id,
                    &finish.claim_id,
                    &session.secret,
                    &phone,
                    &session.phone_public,
                    &session.desktop_public,
                    &fingerprint,
                )
            })
            .ok_or("pairing_closed")?;
        if !pake::same_tag(&finish.confirm, &tags.phone) {
            manual.sessions.remove(&finish.claim_id);
            manual.failures += 1;
            if manual.failures >= MANUAL_ATTEMPTS {
                s.manual = None;
                return Err("too_many_attempts");
            }
            return Err("bad_code");
        }
        let name = session.name.clone();
        let mut answer = self.grant(&mut s, &name)?;
        answer["certificateSha256"] = json!(fingerprint);
        answer["confirm"] = json!(tags.desktop);
        let manual = s.manual.as_mut().unwrap();
        manual.claimed = Some((finish.claim_id, answer.clone()));
        manual.sessions.clear();
        s.invitation = None;
        Ok(answer)
    }
    fn authenticate(self: &Arc<Self>, token: &str) -> Option<Phone> {
        let s = self.state.lock().unwrap();
        if s.endpoints.is_empty() || token.len() != 64 {
            return None;
        }
        let digest = hash(token);
        let grant = s
            .saved
            .grants
            .iter()
            .find(|g| crate::wire::same_secret(&g.token_hash, &digest))?;
        Some(Phone {
            remote: self.clone(),
            id: grant.device.id.clone(),
            cancel: s.devices.get(&grant.device.id)?.child_token(),
        })
    }
    async fn prompt(
        &self,
        phone: &Phone,
        operation: &str,
        persona: &str,
        text: &str,
        attachment_ids: &[String],
        reply_to: Option<&str>,
    ) -> Result<Value, String> {
        if Uuid::parse_str(operation).is_err()
            || (text.trim().is_empty() && attachment_ids.is_empty())
            || text.len() > 32_768
            || attachment_ids.len() > 4
            || reply_to.is_some_and(|id| id.is_empty() || id.len() > 128)
        {
            return Err("Supply a message and a valid operation id (maximum 32 KiB).".into());
        }
        let _held = self.prompts.lock().await;
        if phone.cancel.is_cancelled() {
            return Err("This phone has been disconnected.".into());
        }
        let dir = self.root.join("remote-receipts").join(&phone.id);
        let path = dir.join(format!(
            "{}.json",
            Uuid::parse_str(operation).map_err(message)?
        ));
        // Keep text-only receipts compatible with phones already paired: a
        // field joins the digest only when the message carries it.
        let digest = hash(
            &match (attachment_ids.is_empty(), reply_to) {
                (true, None) => json!([persona, text]),
                (false, None) => json!([persona, text, attachment_ids]),
                (_, Some(answered)) => json!([persona, text, attachment_ids, answered]),
            }
            .to_string(),
        );
        match fs::read(&path) {
            Ok(bytes) => {
                let receipt: Receipt = serde_json::from_slice(&bytes).map_err(message)?;
                if receipt.digest != digest {
                    return Err("This operation id already belongs to another message.".into());
                }
                return match receipt.result {
                    Some(Ok(())) => Ok(json!({"state":"accepted"})),
                    Some(Err(e)) => Err(e),
                    None => Ok(json!({"state":"unknown"})),
                };
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err("Could not read message acceptance.".into()),
        }
        let attachments = attachments::resolve(&self.root, &phone.id, attachment_ids)?;
        fs::create_dir_all(&dir).map_err(message)?;
        let mut receipt = Receipt {
            digest,
            result: None,
        };
        atomic_write(&path, &serde_json::to_vec(&receipt).map_err(message)?).map_err(message)?;
        // The desktop opens a session when its conversation pane mounts. A
        // phone may be the first client to speak to a teammate after restart.
        let result = async {
            self.room.start(persona).await?;
            if phone.cancel.is_cancelled() {
                return Err("This phone has been disconnected.".into());
            }
            self.room
                .prompt(
                    persona,
                    text,
                    reply_to.map(str::to_owned),
                    Some(attachments),
                )
                .await
        }
        .await;
        receipt.result = Some(result.clone());
        if atomic_write(&path, &serde_json::to_vec(&receipt).map_err(message)?).is_err() {
            // The command may already have reached the core. A refusal would
            // wrongly tell the composer it is safe to issue a new operation.
            return Ok(json!({"state":"unknown"}));
        }
        result.map(|()| json!({"state":"accepted"}))
    }
}

/// What this desk calls itself to a phone: the machine's name, as the person
/// named it, with the local-network suffix taken off. Two desks paired to one
/// phone were both "Hotline desktop" before, which told the person nothing.
pub(crate) fn desktop_name() -> String {
    let host = gethostname::gethostname().to_string_lossy().into_owned();
    let name = host.trim_end_matches(".local").trim();
    if name.is_empty() {
        "Hotline desktop".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests;
