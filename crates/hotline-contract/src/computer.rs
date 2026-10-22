//! A teammate's computer: its runtime and status, the host's browsers and
//! cookies, shared secrets, passkeys, and the image releases.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A teammate's computer settings.
///
/// Only what the user decides lives here. Everything Hotline derives — the bearer
/// token, container state, last activity — is process state, not config.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct PersonaComputer {
    pub enabled: bool,
    /// Image override. Defaults to the app's version-pinned image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Memory limit for the container as the runtime spells it — `"2g"`,
    /// `"8g"`, `"512m"`. Absent is 4g. Larger builds can request more.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    /// CPU limit in cores. Absent leaves the runtime unlimited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpus: Option<f64>,
    /// Process limit for the container; threads count. Absent is 1024, zero
    /// is unlimited. A parallel build needs more than the default allows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pids: Option<u32>,
    /// Host folders bound into the desktop besides the workspace, so a
    /// teammate can read a checkout it has to test without cloning it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mounts: Option<Vec<ComputerMount>>,
    /// The stored secrets this computer's shell gets as environment
    /// variables, by name (see [`SharedSecret`]). The value never leaves the
    /// vault except on its way to this machine, and the computer redacts it
    /// from what it answers the agent. Absent means none: a record from
    /// before this field, or a teammate nobody granted anything, is handed
    /// nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<String>>,
}

/// One host folder bound into a teammate's computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ComputerMount {
    /// Absolute host path; `~` expands to the home directory.
    pub host: String,
    /// Absolute path inside the container. `/home/agent/workspace`,
    /// `/home/agent/src` and `/nix` are Hotline's and cannot be taken.
    pub path: String,
    /// Bound read-only. The default: a teammate tests a checkout, it does
    /// not edit it in place.
    #[serde(default)]
    pub readonly: bool,
}

/// The container CLI Hotline will shell out to. `"container"` is Apple's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ComputerRuntime {
    Docker,
    Podman,
    Container,
}

/// The resources available to a teammate's computer. Runtime limits take
/// precedence over host resources; the source makes fallback explicit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct ComputerCapacity {
    pub runtime: Option<ComputerRuntime>,
    pub cpus: u32,
    #[ts(type = "number")]
    pub memory_bytes: u64,
    pub source: ComputerCapacitySource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ComputerCapacitySource {
    Runtime,
    Host,
    Default,
}

/// What probing one runtime found, for the window's settings: a state the
/// window can name in two words, and, when the runtime said something on
/// its way to that state, its words kept apart for whoever wants them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct RuntimeReport {
    pub runtime: ComputerRuntime,
    pub state: RuntimeState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub rootless: bool,
}

/// How a probe of a container runtime ended. `Ready` is the only state a
/// computer can start on; the rest are the two words the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum RuntimeState {
    Ready,
    NotInstalled,
    NotRunning,
    NotResponding,
    Failed,
    Unsupported,
}

impl RuntimeState {
    pub fn ready(self) -> bool {
        self == Self::Ready
    }
}

/// Whether a teammate's computer container is up, stopped, or gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ComputerState {
    Running,
    Stopped,
    Absent,
}

/// A peek at one teammate's computer. `url` and `viewer` are only present
/// while it is running — a drawer that asks must not be what spins it up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ComputerStatus {
    pub state: ComputerState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The web desktop, `http://127.0.0.1:<host port for 8787>/#<token>`:
    /// the viewer page the computer serves, with its bearer in the fragment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewer: Option<String>,
    /// The release the running computer reported with its guide. Absent
    /// until it has, and for an image too old to say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    /// The release this teammate's computer would be created on now, when
    /// that differs from the one running: the pane's offer to update.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available: Option<String>,
    /// Why the update the person last pressed did not land, until they try
    /// again. Said in the pane that offered it, never in the conversation.
    #[serde(
        rename = "updateFailed",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub update_failed: Option<String>,
    /// The running computer was made with other memory or CPU limits than
    /// the teammate has now, which only a restart makes it take.
    #[serde(
        rename = "limitsChanged",
        default,
        skip_serializing_if = "std::ops::Not::not"
    )]
    #[ts(as = "Option<bool>", optional)]
    pub limits_changed: bool,
}

/// A browser found on the host, offered to the operator as a source of
/// cookies for a teammate's computer. Discovery reads only what names a
/// profile; no cookie store is opened until the operator asks for a preview,
/// and no value ever crosses the wire — only these names and, later, the
/// per-site counts in [`CookieSite`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct HostBrowser {
    /// The stable id the operator's choice names back to the desk, e.g.
    /// `chrome`, `brave`, `chromium-snap`, `firefox`.
    pub id: String,
    pub name: String,
    /// `chromium` or `firefox`: which family, so the desk knows to read it
    /// over the debugging protocol or straight from its cookie file. The UI
    /// does not branch on it; it is there for the preview copy.
    pub family: String,
    pub profiles: Vec<BrowserProfile>,
    /// The browser is installed and has a profile folder, but the host
    /// refused to let the desk read it (macOS keeps other apps' data from an
    /// app without Full Disk Access). It is listed with no profiles so the
    /// window can say what to allow, rather than leaving it out as if absent.
    #[serde(default)]
    pub blocked: bool,
    /// The desk's explanation and remedy, naming the app whose access must
    /// change (including the bare binary when running a development build).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

/// One profile within a host browser. Only profiles that have a cookie store
/// on disk are listed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct BrowserProfile {
    /// The on-disk directory (Chromium) or the `profiles.ini` path (Firefox);
    /// the id the operator's choice names back.
    pub id: String,
    /// What the browser shows the profile as, e.g. "Personal", "default".
    pub name: String,
}

/// One site the operator can choose to bring over, and how many cookies it
/// holds. A preview is domains and counts only, never a value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct CookieSite {
    pub domain: String,
    pub cookies: u32,
}

/// One browser profile's cookies brought over to a teammate's computer, as
/// the pane lists it afterwards: which browser and profile they came from,
/// when, and the sites, so the person can see what the computer's browser
/// is signed in to and take any of it back. Domains and counts only; no
/// cookie value is ever kept on the desk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct CookieImport {
    pub browser_id: String,
    pub browser_name: String,
    pub profile_id: String,
    pub profile_name: String,
    /// Unix milliseconds of the latest import from this browser and profile.
    pub imported_at: i64,
    pub sites: Vec<CookieSite>,
}

/// What a stored secret is, which says where a granted computer puts it: a
/// variable into the environment of every job; a login typed into a sign-in
/// form on one of its own sites; a passkey into the browser's authenticator,
/// where it signs in by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SharedSecretKind {
    Variable,
    Login,
    Passkey,
}

/// Whether a login or passkey may be used without the person's answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SecretUse {
    #[default]
    Auto,
    Ask,
}

impl From<bool> for SecretUse {
    fn from(ask: bool) -> Self {
        if ask { Self::Ask } else { Self::Auto }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SecretUseAnswer {
    Approve,
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SecretUseStatus {
    Pending,
    Approved,
    Allowed,
    Denied,
    Expired,
}

/// A secret the operator stored for teammates to use, kept in the OS
/// credential store under a name spelled like an environment variable. This
/// is all the window ever sees of one — the value is written once and never
/// answered back; what is listed beside the name is identity, not a secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SharedSecret {
    /// `[A-Z][A-Z0-9_]*`, not Hotline's own `HOTLINE_*` and not the shell's.
    pub name: String,
    /// When the value was last stored or replaced, ms since the epoch.
    pub updated_at: i64,
    pub kind: SharedSecretKind,
    /// Missing on older records means automatic.
    #[serde(default)]
    pub r#use: SecretUse,
    /// A login's sites: the origins its fields are typed on, and no other.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sites: Option<Vec<String>>,
    /// A login's username.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Whether a login carries a TOTP seed, so `NAME.code` is a code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub totp: Option<bool>,
    /// A passkey's site, as a relying party id: `github.com`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rp_id: Option<String>,
    /// The account name the site gave a passkey, when it gave one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
}

/// Where the making of a passkey stands. `armed` is the ten minutes in
/// which the teammate's computer may make one for the site; `asked` is the
/// site's request, parked in the browser until the person answers the card
/// on the teammate's tape; `approved` is the moment between that answer and
/// the browser making it; `stored` is answered once, when the computer made
/// it and the desk has stored it and ticked it for that teammate; `idle` is
/// no arming, or one that ran out, or one a denial ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum PasskeyRegistrationState {
    Idle,
    Armed,
    Asked,
    Approved,
    Stored,
}

/// What a site asked for when it called for a passkey under an arming: the
/// site, the origin the page is on, and the account the passkey would be
/// for, as the computer read them off the request. What the card shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct PasskeyAsk {
    /// The computer's id for the request; the answer names it.
    pub id: String,
    pub rp_id: String,
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rp_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_display_name: Option<String>,
    /// When the computer first saw it, ms since the epoch.
    #[serde(default)]
    pub asked_at: i64,
}

/// The answer to `secrets.passkey.register`, `.registration`, `.answer`
/// and `.cancel`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct PasskeyRegistration {
    pub state: PasskeyRegistrationState,
    /// The name the passkey is, or will be, stored under.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rp_id: Option<String>,
    /// When the arming ends, ms since the epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// The site's request, when `asked` or `approved`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask: Option<PasskeyAsk>,
    /// The record stored, when `stored`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<SharedSecret>,
}

/// Which Hotline Computer release a new computer is created on: the newest
/// published one the desk has heard of, else the floor it was built
/// against. A pinned image, the teammate's or the room's, overrides both.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ComputerReleases {
    pub floor: String,
    /// The repository every release's image is under; `<repository>:<tag>`
    /// is the pin the Release picker writes.
    pub repository: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub newest: Option<String>,
    /// Every published release from the floor up, newest first: what a room
    /// may be set to. Empty until a lookup has answered.
    pub releases: Vec<String>,
    /// When the desk last asked, ms since the epoch; absent before it has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
    /// Why the last lookup failed, until one succeeds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Offers default to on for all sites, including rooms saved before this
/// setting existed. This permits discovery, never lending without Use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export, export_to = "contract.ts")]
pub struct CookieOfferSettings {
    pub enabled: bool,
    /// Absent means all sites; an empty list means none.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sites: Option<Vec<String>>,
}

impl Default for CookieOfferSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            sites: None,
        }
    }
}

impl CookieOfferSettings {
    pub fn allows(&self, site: &str) -> bool {
        self.enabled
            && self
                .sites
                .as_ref()
                .is_none_or(|sites| sites.iter().any(|s| s == site))
    }
}

/// Site names only, read when the person opens the settings page.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct CookieOfferSites {
    pub sites: Vec<CookieOfferSite>,
    /// A refused or unreadable browser is not an empty inventory.
    pub unavailable: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct CookieOfferSite {
    pub site: String,
    pub sources: Vec<CookieOfferSource>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct CookieOfferSource {
    pub browser_name: String,
    pub profile_name: String,
}

/// A host-cookie offer carries metadata only, including after settlement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct CookieOffer {
    pub id: String,
    pub ts: i64,
    pub teammate: String,
    pub site: String,
    pub browser_name: String,
    pub profile_name: String,
    pub login: String,
    pub status: CookieOfferStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum CookieOfferStatus {
    Pending,
    /// A delivery began; retain Remove even if the desk stopped mid-transfer.
    Unconfirmed,
    Lent,
    Declined,
    Never,
    Expired,
    Removed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum CookieOfferAnswer {
    Use,
    NotNow,
    Never,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct CookieOfferBlock {
    pub teammate: String,
    pub name: String,
    pub site: String,
}
