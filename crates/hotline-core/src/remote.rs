//! A paired phone enters the real wire through a separate, opt-in TLS listener.
//! Its identity key uses the same OS credential store as provider credentials.
mod admission;
mod attachments;
pub mod bridge;
mod channel;
pub mod client;
mod link;
mod network;
mod relay;
mod sealed;
mod served;
mod server;
mod v2;
mod viewer_files;
pub use served::ServeOptions;
pub use v2::{PairingPayload, SealedPairing};

use crate::contract::{DeviceRole, MobileAttachmentChunk, ThreadId, ThreadKind};
use crate::credentials::{CredentialFile, CredentialFiles, SecretStore, atomic_write};
use crate::log::Log;
use crate::wire::RoomHandle;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs, io,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};
use tokio::sync::Mutex as AsyncMutex;
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
/// A panic on one connection must not take every later one down with it, nor
/// unwind a destructor before it signals release. Recovering the guard is
/// sound only where the writers under it leave the value whole at every point
/// that can panic; each lock taken through here has been read for that.
fn lock<T>(held: &Mutex<T>) -> MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}
/// Leaves `held` poisoned, as a panic while it was held would.
#[cfg(test)]
fn poison<T>(held: &Mutex<T>) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _held = held.lock();
        panic!("poisoned on purpose");
    }));
    assert!(held.is_poisoned());
}
impl DeviceRole {
    /// The seat this role is given on the wire, if any. A relay grant has
    /// none, so the wire cannot be asked to seat one.
    pub(crate) fn seated(self) -> Option<WireRole> {
        match self {
            Self::Owner => Some(WireRole::Owner),
            Self::Companion => Some(WireRole::Companion),
            Self::Relay => None,
        }
    }
}
/// The roles the wire seats: a `DeviceRole` with the relay taken out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WireRole {
    Owner,
    Companion,
}

#[derive(Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct RemoteDevice {
    pub id: String,
    pub name: String,
    pub paired_at: i64,
    #[serde(default)]
    pub role: DeviceRole,
    // Older bearer grants remain visible for revocation, but only a public
    // key can authorize a sealed connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub public_key: Option<String>,
    /// Set only on the grant a phone's own `remote.devices` came in on, so it
    /// can name itself and warn before revoking itself. Never saved true:
    /// the flag is put on a copy as the list is answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub current: Option<bool>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Grant {
    device: RemoteDevice,
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
/// never has to hold the remote. None while Remote is off: turning it off
/// keeps the pairings for later, and must also stop the notifications.
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
            .filter(|grant| saved.enabled && grant.device.public_key.is_some())
            .filter_map(|grant| grant.push.as_ref().map(|push| push.token.clone()))
            .collect(),
    }
}
/// A pasted public address, as the origin phones dial: https, a host, no
/// path, credentials, query or fragment. Blank is none.
fn public_origin(text: &str) -> Result<Option<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let refused = || {
        "Use the https address of your tunnel or proxy, such as https://desk.example.com, with nothing after the name or port.".to_string()
    };
    let url = url::Url::parse(text).map_err(|_| refused())?;
    if url.scheme() != "https"
        || url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(refused());
    }
    Ok(Some(url.origin().ascii_serialization()))
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
    /// Relay grants, kept out of `grants` because a reader from before the
    /// relay role fails the whole file on a role it does not know. Such a
    /// reader ignores this list, so it never authenticates a relay key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    relay_grants: Vec<Grant>,
    /// The paired desk whose relay this one stands in on, when it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relay: Option<String>,
    /// An https origin outside this computer that reaches its listener: a
    /// tunnel, a reverse proxy, a forwarded port. Optional and only advertised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    public_url: Option<String>,
}
impl Saved {
    /// Every grant, relay grants included. A key is in one list at most.
    fn all_grants(&self) -> impl Iterator<Item = &Grant> {
        self.grants.iter().chain(&self.relay_grants)
    }
}
#[derive(Serialize, Deserialize)]
struct Identity {
    #[serde(default)]
    hosts: Vec<String>,
    certificate: Vec<u8>,
    key: Vec<u8>,
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
    /// The paired desk carrying visitors to this one, when one is chosen.
    pub relay: Option<relay::RemoteRelay>,
    /// The person's own public address for this desk, kept whether or not
    /// Remote is on, and advertised to phones after this computer's own.
    pub public_url: Option<String>,
}
struct Live {
    saved: Saved,
    endpoints: Vec<String>,
    sealed_pairing: Option<v2::Window>,
    cancel: CancellationToken,
    devices: HashMap<String, CancellationToken>,
    error: Option<String>,
    attempts: u16,
    attempts_since: i64,
    relay: relay::Standing,
}
#[derive(Serialize, Deserialize)]
struct Receipt {
    digest: String,
    result: Option<Result<(), String>>,
}

pub struct Remote {
    root: PathBuf,
    identity: CredentialFile,
    noise_identity: CredentialFile,
    served: Option<ServeOptions>,
    admission: Arc<admission::Admission>,
    relay: Arc<relay::Hub>,
    relayed: Arc<tokio::sync::Semaphore>,
    store: Arc<dyn SecretStore>,
    state: Mutex<Live>,
    server: AsyncMutex<Option<tokio::task::JoinHandle<()>>>,
    lifecycle: AsyncMutex<()>,
    prompts: AsyncMutex<()>,
    /// The desk behind this Remote's wire, computer and link: absent on a
    /// server that runs only a relay (ADR 0006), whose room is never opened.
    agents: Option<Agents>,
}
/// The room a Remote opens doors onto when its server runs agents.
pub struct Agents {
    pub log: Log,
    pub room: Arc<dyn RoomHandle>,
}
/// Why a relay-only server answers nothing that needs the room.
pub const RELAY_ONLY: &str = "This server runs only a relay: it has no teammates, links or wire. Serve it with --capabilities agents,relay to use them.";
#[derive(Clone)]
pub(crate) struct Phone {
    remote: Arc<Remote>,
    id: String,
    role: DeviceRole,
    pub cancel: CancellationToken,
}
impl Phone {
    pub(crate) fn endpoints(&self) -> Vec<String> {
        self.remote.status().endpoints
    }
    /// Which address among the endpoints is outside the networks this
    /// computer is on, so any seat can tell it from a VPN or LAN one.
    pub(crate) fn public_endpoint(&self) -> Option<String> {
        self.remote.public_endpoint()
    }
    /// A served desk is a headless server; anything else is a person's desktop.
    pub(crate) fn desk_kind(&self) -> &'static str {
        if self.remote.served.is_some() {
            "server"
        } else {
            "desktop"
        }
    }
    /// What the server runs, for an owner's hello.
    pub(crate) fn serves(&self) -> Vec<&'static str> {
        self.remote.serves()
    }
    /// Narrows this device's own owner grant to a relay grant.
    pub(crate) fn become_relay(&self) -> Result<Value, String> {
        self.remote.become_relay(&self.id)
    }
    /// The paired phones, this one marked.
    pub(crate) fn devices(&self) -> Vec<RemoteDevice> {
        let mut devices = self.remote.devices();
        for device in &mut devices {
            device.current = (device.id == self.id).then_some(true);
        }
        devices
    }
    pub(crate) async fn prompt(
        &self,
        operation_id: &str,
        persona_id: &str,
        text: &str,
        attachment_ids: &[String],
        reply_to: Option<&str>,
        thread: Option<&ThreadId>,
    ) -> Result<Value, String> {
        self.remote
            .prompt(
                self,
                operation_id,
                persona_id,
                text,
                attachment_ids,
                reply_to,
                thread,
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
    pub fn open_with_store(
        root: &Path,
        log: Log,
        room: Arc<dyn RoomHandle>,
        store: Arc<dyn SecretStore>,
    ) -> io::Result<Arc<Self>> {
        Self::open_options(root, Some(Agents { log, room }), store, None)
    }
    fn open_options(
        root: &Path,
        agents: Option<Agents>,
        store: Arc<dyn SecretStore>,
        served: Option<ServeOptions>,
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
        if let Some(options) = &served {
            saved.host = options.listen.to_string();
            saved.port = options.listen.port();
            saved.enabled = false;
        }
        let files = CredentialFiles::new(root.to_path_buf(), store.clone());
        let identity = files.file(root.join("remote-identity.json"));
        let noise_identity = files.file(root.join("remote-noise-identity.json"));
        Ok(Arc::new(Self {
            root: root.to_path_buf(),
            identity,
            noise_identity,
            served,
            admission: Arc::new(admission::Admission::default()),
            relay: Arc::default(),
            relayed: Arc::new(tokio::sync::Semaphore::new(relay::RELAYED_MAX)),
            store,
            state: Mutex::new(Live {
                saved,
                endpoints: Vec::new(),
                sealed_pairing: None,
                cancel: CancellationToken::new(),
                devices: HashMap::new(),
                error: None,
                attempts: 0,
                attempts_since: now(),
                relay: relay::Standing::default(),
            }),
            server: AsyncMutex::new(None),
            lifecycle: AsyncMutex::new(()),
            prompts: AsyncMutex::new(()),
            agents,
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
    /// The room behind the wire, or why there is none.
    fn agents(&self) -> Result<&Agents, String> {
        self.agents.as_ref().ok_or_else(|| RELAY_ONLY.to_string())
    }
    /// Whether this is a served desk that carries a relay. A desktop's own
    /// Remote never does.
    fn relays(&self) -> bool {
        self.served.as_ref().is_some_and(|options| options.relay)
    }
    /// What this server runs, as a device is told (ADR 0006 § Discovery).
    pub(crate) fn serves(&self) -> Vec<&'static str> {
        let mut serves = Vec::new();
        if self.agents.is_some() {
            serves.push("agents");
        }
        if self.relays() {
            serves.push("relay");
        }
        serves
    }

    pub(crate) fn status_desktop_id(&self) -> String {
        self.state.lock().unwrap().saved.desktop_id.clone()
    }
    pub fn status(&self) -> RemoteStatus {
        let s = self.state.lock().unwrap();
        RemoteStatus {
            enabled: !s.endpoints.is_empty(),
            host: s.saved.host.clone(),
            endpoint: s.endpoints.first().cloned(),
            // A paired phone learns these from the hello and tries them all.
            // The public address comes after this computer's own, so the QR
            // keeps pairing on the network the phone is standing in.
            endpoints: s
                .endpoints
                .iter()
                .chain(
                    s.saved
                        .public_url
                        .iter()
                        .filter(|_| !s.endpoints.is_empty()),
                )
                .chain(&s.relay.url)
                .cloned()
                .collect(),
            addresses: Self::addresses(),
            devices: s.saved.all_grants().map(|g| g.device.clone()).collect(),
            error: s.error.clone(),
            relay: self.relay_status(&s),
            public_url: s.saved.public_url.clone(),
        }
    }
    /// A served desk's only address is its public one; a desktop's is the
    /// person's own, advertised while Remote is on.
    fn public_endpoint(&self) -> Option<String> {
        self.public_endpoint_of(&self.state.lock().unwrap())
    }
    fn public_endpoint_of(&self, s: &Live) -> Option<String> {
        if s.endpoints.is_empty() {
            return None;
        }
        match &self.served {
            Some(options) => Some(options.public_url.clone()),
            None => s.saved.public_url.clone(),
        }
    }
    /// Keeps, replaces or clears (`None` or blank) the public address. It is
    /// only advertised, so nothing restarts and no phone is disconnected.
    pub fn set_public_url(&self, url: Option<&str>) -> Result<RemoteStatus, String> {
        if self.served.is_some() {
            return Err("A served desk's public URL is set when it starts.".into());
        }
        let url = public_origin(url.unwrap_or_default())?;
        let mut s = self.state.lock().unwrap();
        let mut saved = s.saved.clone();
        saved.public_url = url;
        self.save(&saved)?;
        s.saved = saved;
        drop(s);
        Ok(self.status())
    }
    pub async fn restore(self: &Arc<Self>) {
        // A desk dials the links it made whether or not its own listener is
        // on: the other desk is the one being reached.
        self.restore_links();
        if self.served.is_some() {
            self.restore_served().await;
            return;
        }
        let (enabled, host) = {
            let s = self.state.lock().unwrap();
            (s.saved.enabled, s.saved.host.clone())
        };
        if enabled && let Err(e) = self.configure(true, &host).await {
            eprintln!("[remote] did not start: {e}");
            self.state.lock().unwrap().error = Some(e);
        }
    }
    pub async fn configure(
        self: &Arc<Self>,
        enabled: bool,
        host: &str,
    ) -> Result<RemoteStatus, String> {
        if self.served.is_some() {
            return self.configure_served(enabled, host).await;
        }
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
            s.sealed_pairing = None;
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
            self.stand();
            return Ok(self.status());
        }
        let saved_port = self.state.lock().unwrap().saved.port;
        let listener = network::bind(host, saved_port, &addresses)
            .await
            .map_err(message)?;
        let port = listener.local_addr().map_err(message)?.port();
        // The identity is a certificate this desk signed itself, so one it
        // cannot read any more — corrupt, gone from the OS store, or written
        // by an earlier edition into a store this build cannot name — is replaced
        // like a missing one. Sealed phones trust the independent Noise key.
        // A locked or unavailable store is reported instead of replaced.
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
                // TLS rotation changes no device authority.
                identity
            }
        };
        self.noise_keys()?;
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
            s.cancel = cancel.clone();
            let keyed: Vec<_> = s
                .saved
                .all_grants()
                .filter(|grant| grant.device.public_key.is_some())
                .map(|grant| grant.device.id.clone())
                .collect();
            for id in keyed {
                s.devices.insert(id, cancel.child_token());
            }
        }
        let this = self.clone();
        *self.server.lock().await = Some(tokio::spawn(async move {
            server::run(this, listener, tls, cancel).await;
        }));
        self.stand();
        Ok(self.status())
    }
    pub fn revoke(&self, id: &str) -> Result<RemoteStatus, String> {
        let mut s = self.state.lock().unwrap();
        let mut saved = s.saved.clone();
        saved.grants.retain(|g| g.device.id != id);
        saved.relay_grants.retain(|g| g.device.id != id);
        // Close the live authority even if persistence fails. A failed revoke
        // must never leave a socket executing under the grant being removed.
        if let Some(cancel) = s.devices.remove(id) {
            cancel.cancel();
        }
        let result = self.save(&saved);
        s.saved = saved;
        if let Err(error) = result {
            s.cancel.cancel();
            s.endpoints.clear();
            s.error = Some(error.clone());
            return Err(error);
        }
        // An idempotent pairing retry must never recover a revoked grant.
        s.sealed_pairing = None;
        drop(s);
        Ok(self.status())
    }
    /// `remote.become_relay`: the grant `id` holds moves from `grants` to
    /// `relayGrants` (ADR 0006 § Migration), keeping its id, key, name and
    /// pairing time, and its sessions end as a revocation's would. `id` is
    /// only ever the caller's own, from its handshake. An owner grant alone
    /// may narrow, and nothing widens one.
    fn become_relay(&self, id: &str) -> Result<Value, String> {
        if !self.relays() {
            return Err("This server does not run a relay.".into());
        }
        let mut s = self.state.lock().unwrap();
        let mut saved = s.saved.clone();
        let at = saved
            .grants
            .iter()
            .position(|g| g.device.id == id && g.device.role == DeviceRole::Owner)
            .ok_or("Only an owner's pairing can become a relay pairing.")?;
        let mut grant = saved.grants.remove(at);
        grant.device.role = DeviceRole::Relay;
        // A relay is never notified: push targets are a phone's.
        grant.push = None;
        saved.relay_grants.push(grant);
        self.save(&saved)?;
        s.saved = saved;
        if let Some(cancel) = s.devices.remove(id) {
            cancel.cancel();
        }
        let fresh = s.cancel.child_token();
        s.devices.insert(id.to_owned(), fresh);
        drop(s);
        Ok(json!({
            "role": DeviceRole::Relay,
            "deskName": desktop_name(),
            "deskId": self.status_desktop_id(),
            "serves": self.serves(),
        }))
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
    // The operation, who it is to, what is said and where: one message, field by field.
    #[allow(clippy::too_many_arguments)]
    async fn prompt(
        &self,
        phone: &Phone,
        operation: &str,
        persona: &str,
        text: &str,
        attachment_ids: &[String],
        reply_to: Option<&str>,
        thread: Option<&ThreadId>,
    ) -> Result<Value, String> {
        // The main conversation, named or not, or one of the teammate's work threads.
        let side = match thread {
            None => None,
            Some(id) if id.kind == ThreadKind::Dm && id.key == persona => None,
            Some(id) if id.kind == ThreadKind::Side && !id.key.is_empty() => Some(id.key.as_str()),
            Some(_) => return Err("A phone speaks in the conversation or a work thread.".into()),
        };
        if Uuid::parse_str(operation).is_err()
            || (text.trim().is_empty() && attachment_ids.is_empty())
            || text.len() > 32_768
            || attachment_ids.len() > 4
            || reply_to.is_some_and(|id| id.is_empty() || id.len() > 128)
        {
            return Err("Supply a message and a valid operation id (maximum 32 KiB).".into());
        }
        let room = &self.agents()?.room;
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
            &match (attachment_ids.is_empty(), reply_to, side) {
                (true, None, None) => json!([persona, text]),
                (false, None, None) => json!([persona, text, attachment_ids]),
                (_, Some(answered), None) => json!([persona, text, attachment_ids, answered]),
                (_, answered, Some(side)) => {
                    json!([persona, text, attachment_ids, answered, side])
                }
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
            if let Some(side) = side {
                // A work thread brings its own agent up; the main session is not started for it.
                return room
                    .side_prompt(side, text, reply_to.map(str::to_owned), Some(attachments))
                    .await;
            }
            room.start(persona).await?;
            if phone.cancel.is_cancelled() {
                return Err("This phone has been disconnected.".into());
            }
            room.prompt(
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
