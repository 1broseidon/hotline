//! The room's contract types: what the window is handed when it asks about a
//! teammate, and what the room pushes at it unasked.
//!
//! These are defined once, here, and the TypeScript is generated from them:
//! `cargo test -p hotline-core` runs the tests `#[ts(export)]` writes and leaves
//! `ui/src/generated/contract.ts` behind, which the window
//! re-exports rather than spelling the same shapes a second time. The
//! workspace's `.cargo/config.toml` is what points ts-rs at that directory
//! and tells it a 64-bit integer is a JavaScript `number`; run `cargo test`
//! and check the regenerated file in with the change that moved it.
//!
//! Every attribute here serves one invariant: the JSON serde emits must be
//! what the Bun main emits for the same value, field for field. That is why
//! an optional field carries `skip_serializing_if` — absent means ABSENT, and
//! a `null` where Bun wrote nothing is a different value to the window — and
//! why the tests at the bottom feed the store's own output through these
//! types and back out again.

use crate::thread::{ThreadId, ThreadKind};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

pub use crate::imagegen::ImageSettings;
pub use crate::spending::{SpendingSettings, SpendingSummary};

// ---------------------------------------------------------------------------
// Avatars
// ---------------------------------------------------------------------------

/// A teammate's picture: a square PNG the desk keeps under
/// `avatars/<teammate>/<hash>.png`, named by the SHA-256 of its bytes, so a
/// picture is never rewritten and a window can cache it by hash for good.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct Avatar {
    /// Lowercase hex SHA-256 of the kept PNG.
    pub hash: String,
    pub by: AvatarBy,
    /// When the picture was set, as an ISO timestamp.
    pub updated_at: String,
}

/// Who chose the picture. A teammate does not replace one the person chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum AvatarBy {
    #[serde(rename = "self")]
    Own,
    Person,
}

// ---------------------------------------------------------------------------
// Teammates
// ---------------------------------------------------------------------------

/// A Hotline teammate. Four axes make up an identity:
///   - Identity   : `goal`, materialised as AGENTS.md inside `cwd`
///   - Workspace  : `cwd`, passed to session/new
///   - Capability : `mcpPolicy`, resolved against the app's MCP servers
///   - Disposition: `modelId` / `modeId`, switchable mid-session
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct Persona {
    /// Set on a teammate who lives on a linked desktop: which one. Their id is
    /// node-qualified (`nodeId/personaId`) and every call about them rides the
    /// fleet wire to that desktop. Absent for teammates of this machine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<PersonaNode>,
    pub id: String,
    pub name: String,
    pub goal: String,
    /// The teammate's picture. Absent means the initial on its hashed colour.
    /// A record written before pictures existed may still carry a `face`,
    /// which is read past and dropped the next time the record is written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<Avatar>,
    /// The team this teammate sits on — a label, not an entity. Teams are not
    /// agents and never speak: addressing one round-robins to the next
    /// available member, who routes it onward. Distinct labels ARE the teams;
    /// an empty label is the unteamed default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    pub backend_id: String,
    pub cwd: String,
    /// How far Hotline Agent's tools reach. Absent means the working directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reach: Option<Reach>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode_id: Option<String>,
    /// The effort a Hotline Agent teammate runs at, when its model offers one.
    /// Absent means the model's default. An ACP teammate does not store this:
    /// the harness owns its config ids.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort_id: Option<String>,
    /// A configured fallback harness for a desk that cannot run this
    /// teammate's current one — the matching ladder's middle rung, between
    /// "exactly what it runs now" and the room's default. Absent means no
    /// preference beyond those.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness_override: Option<HarnessChoice>,
    /// A hop landed this teammate here and it has not been told yet. Machine-
    /// bound and consumed once: the first message after the move carries this
    /// ahead of the user's words, so the agent knows it changed machines and
    /// must verify its workspace instead of assuming old filesystem state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hop_notice: Option<String>,
    /// Which of the app's MCP servers this teammate is given.
    pub mcp_policy: McpPolicy,
    /// Which of the offered skills this teammate is given: the gateway's,
    /// and the person's own that are switched on. Built-ins are always on
    /// and not part of this. Absent means none, including for older records.
    #[serde(default)]
    pub skill_policy: SkillPolicy,
    /// Whether this teammate may create and receive its own persistent
    /// schedules. Operator-created jobs carry their own provenance and do not
    /// depend on this grant. Absent means off, including for older records.
    #[serde(default)]
    pub background_work: bool,
    /// Stable ids of teammates this teammate accepts work requests from.
    /// Missing on older records means no permanent collaboration grants.
    #[serde(default)]
    pub allowed_senders: Vec<String>,
    /// Absent means inherit the desk's web search entirely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search_policy: Option<WebSearchPolicy>,
    /// This teammate's computer: a containerized desktop it drives through MCP
    /// tools. Deliberately not part of `mcpPolicy` — the computer is a
    /// per-teammate capability Hotline manages, not one of the app's
    /// user-configured servers. Absent means off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computer: Option<PersonaComputer>,
    /// The voice this teammate speaks in on a direct call, as picked from the
    /// desk's speaking model. It applies only while the desk still speaks
    /// with that provider and model; otherwise the desk's own voice is used.
    /// Absent means the desk's voice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<PersonaVoice>,
    /// The last durable ACP session for each backend this teammate has used.
    ///
    /// ACP session ids are opaque to the agent that issued them. Keeping one
    /// per backend lets a teammate switch harnesses and later return to either
    /// one without sending Cursor's id to Claude (or vice versa).
    pub session_checkpoints: Vec<SessionCheckpoint>,
    /// Legacy v1 field. Read once into `sessionCheckpoints`; no longer
    /// written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_session_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The linked desktop a teammate lives on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts")]
pub struct PersonaNode {
    pub id: String,
    pub name: String,
}

/// One backend's durable session id for one teammate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SessionCheckpoint {
    pub backend_id: String,
    pub session_id: String,
}

/// How far a teammate's tools reach. The one policy a teammate has, and it is
/// binary: the working directory is a wall, or the whole machine is open.
/// Neither asks; an agent is there to go and do things.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum Reach {
    #[default]
    Workspace,
    Machine,
}

/// Inherit everything, inherit nothing, or name what is inherited. `Some` is
/// read only in the `some` mode; the list is kept in the other two so that
/// toggling does not lose it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum PolicyMode {
    All,
    None,
    Some,
}

/// Which servers from the global MCP gateway a teammate gets.
///
/// New teammates get `none`: configuring a server does not grant its powers
/// to the roster. The operator grants selected servers with `some`, or every
/// current and future server with `all`. These grants are independent of reach;
/// a granted server retains its own permissions outside the shell sandbox.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct McpPolicy {
    pub mode: PolicyMode,
    pub server_ids: Vec<String>,
}

/// Which kind of agent produced a tool ledger.
///
/// `hotline` is Hotline Agent's stored backend id, not a second name: that agent
/// builds its own tool array, so a verified row is a fact. An ACP backend is
/// handed descriptors and does not report what it loaded, so its honest
/// state is declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum AgentKind {
    Hotline,
    Acp,
}

/// Where one of a teammate's tools came from.
///
/// Coarse on purpose: this names the mechanism that supplies the tool,
/// because that is what decides how an absence is fixed. `origin` beside it
/// names the particular supplier — an MCP server's id, `hotline`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ToolSourceKind {
    Builtin,
    Mcp,
}

/// How sure Hotline is about one tool.
///
/// `verified` — Hotline watched the agent take it: it built the tool array
/// itself. `declared` — Hotline handed it over and cannot see what happened
/// next. `absent` — it is not there, and `reason` says why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ToolState {
    Verified,
    Declared,
    Absent,
}

/// One line of a teammate's tool ledger.
///
/// `reason` is required in every state, and that is the whole design. Tools
/// vanishing silently is the worst failure this project has shipped, and
/// every one of those bugs was an absence with an optional explanation
/// nobody filled in. A field that cannot be omitted cannot be forgotten.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts")]
pub struct ToolLedgerRow {
    pub name: String,
    pub source: ToolSourceKind,
    /// The particular supplier, named: a server id, or `hotline`.
    pub origin: String,
    pub state: ToolState,
    /// Why this tool is in this state. Never empty.
    pub reason: String,
    /// When Hotline last observed it.
    pub at: i64,
}

/// Everything Hotline knows about one teammate's tools, and how it knows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct TeammateToolLedger {
    pub persona_id: String,
    pub agent_kind: AgentKind,
    pub backend_id: String,
    /// When the session that produced this ledger started.
    pub at: i64,
    pub rows: Vec<ToolLedgerRow>,
}

/// Which of the desk's web search a teammate gets — the same
/// inherit/override question `McpPolicy` answers for servers. Absent on the
/// teammate means `all`: inherit whatever the app's Tools pane has on. `some`
/// intersects with the app's choices — a provider the desk switched off stays
/// off for everyone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts")]
pub struct WebSearchPolicy {
    pub mode: PolicyMode,
    pub providers: Vec<WebSearchProvider>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum WebSearchProvider {
    Parallel,
    Exa,
    Firecrawl,
    Keenable,
}

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

/// One harness, optionally pinned to a model — how the matching ladder names
/// what runs a teammate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct HarnessChoice {
    pub backend_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

/// What the new-teammate form fills in. Everything but the name has a default
/// the store supplies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct PersonaDraft {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Initial roster section. Empty and omitted both mean the default team.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reach: Option<Reach>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computer: Option<PersonaComputer>,
    /// Whether the teammate may keep its own schedules from the start. Absent
    /// is off, as on the pane.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_work: Option<bool>,
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// A provider credential, as the room knows one: everything except the secret.
///
/// The secret is never an event. It lives in the vault — a `0600` file in a
/// `0700` directory on this machine — and the room stream carries only this,
/// so a stream can be read, copied or shipped without a key going with it.
/// A `credential` event is these fields under that `kind`; a deletion is the
/// same id with `deleted: true` and nothing else.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct Credential {
    pub id: String,
    pub provider_id: String,
    /// The event's own `kind` names the event, so a credential's kind is
    /// spelled differently here: two fields called `kind` would be one field.
    pub credential_kind: CredentialKind,
    /// The chosen Ollama or custom server. Other providers use their fixed endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Custom connections keep their protocol and model ids with their identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomProvider>,
    /// What the user called it, so a list of keys is a list they recognise.
    pub label: String,
    /// Revoked. Set once and never unset — revocation is a fact, not a toggle.
    pub revoked: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A provider Hotline Agent can hold a credential for, as the key form offers
/// them. Which ones there are is `models::WIRING`; the name and the doc link
/// come from the model catalogue. A provider can offer several ways to connect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct Provider {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub credential_kinds: Vec<CredentialKind>,
    /// Whether Rig can discover models using this provider connection.
    pub model_discovery: bool,
}

/// How this connection was established: a pasted key, a provider sign-in,
/// or a keyless server URL. OAuth does not imply subscription billing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum CredentialKind {
    ApiKey,
    Oauth,
    Local,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum OpenAiApi {
    Responses,
    ChatCompletions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct CustomProvider {
    pub api: OpenAiApi,
    pub models: Vec<String>,
}

/// Input only. The key is never copied into credential metadata or a stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct CustomProviderDraft {
    pub name: String,
    pub base_url: String,
    pub api: OpenAiApi,
    pub models: Vec<String>,
    /// Omitted keeps an existing key; an empty string removes it.
    pub secret: Option<String>,
}

/// Instructions for provider sign-in. Browser callback flows leave `user_code`
/// empty; device flows supply the code to enter at `verification_uri`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct LoginPrompt {
    pub login_id: String,
    pub user_code: String,
    pub verification_uri: String,
}

/// How far a provider login has got. A finished one stays queryable until
/// the process exits, so a window that missed the moment can still read it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct LoginStatus {
    pub state: LoginState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<Credential>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum LoginState {
    Pending,
    Done,
    Failed,
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// Capabilities and options a live session reported, used to drive the UI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SessionInfo {
    pub persona_id: String,
    pub state: SessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
    /// Whether Hotline's transcript is showing history the agent no longer
    /// remembers.
    pub context_restored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restore_note: Option<String>,
    pub models: Vec<ConfigChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_label: Option<String>,
    pub modes: Vec<ConfigChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_mode_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode_label: Option<String>,
    /// Select config options that are not the model or mode picker (e.g.
    /// Claude effort).
    pub configs: Vec<SessionConfig>,
    pub slash_commands: Vec<SlashCommand>,
    pub capabilities: SessionCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SessionState {
    Idle,
    Starting,
    Ready,
    Thinking,
    Error,
    Stopped,
}

/// One picker the agent offers beyond the model and the mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SessionConfigCategory {
    /// A model's reasoning or thought level.
    Effort,
}

/// The idle effort picker's reply for one catalogue model: the levels it
/// offers and the one a teammate with none stored runs at, so the strip
/// shows before a session what the session will do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct EffortChoices {
    pub choices: Vec<ConfigChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_id: Option<String>,
}

/// One picker the agent offers beyond the model and the mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SessionConfig {
    pub id: String,
    pub name: String,
    /// The small set of generic selectors Hotline may place in the conversation
    /// header. Unknown ACP selectors stay out of the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<SessionConfigCategory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_id: Option<String>,
    pub options: Vec<ConfigChoice>,
}

/// What the agent behind a session can be asked to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SessionCapabilities {
    /// The driver admits operator input during its active conversation.
    #[serde(default)]
    pub active_input: bool,
    pub load_session: bool,
    pub resume: bool,
    pub fork: bool,
    pub mcp_http: bool,
    pub image: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ConfigChoice {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Picker section header — the provider serving this choice, with its
    /// billing flavor ("Anthropic — subscription", "OpenRouter — API key").
    /// The same model name can be served two ways at very different prices;
    /// the section is what tells them apart before the choice is made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// One model in a provider's catalogue, as the filter panel lists them.
///
/// `id` is the catalogue key (bare, so OpenRouter keeps its own slash).
/// `enabled` is the room's `enabledModels` filter: every model is enabled
/// when that provider is absent from the setting. A subscription with an
/// account list omits models the account cannot run, so they never appear
/// here to be flagged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    pub release_date: String,
    pub enabled: bool,
    pub manual: bool,
    /// An exact provider/model match exists in the bundled metadata.
    pub metadata_known: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub efforts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<ModelCost>,
}

/// Dollars per million tokens, when exact catalogue metadata has a price.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SlashCommand {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

// ---------------------------------------------------------------------------
// The transcript
// ---------------------------------------------------------------------------

/// One durable line in a teammate's transcript. Appended as JSONL and replayed
/// at startup. Note this is Hotline's own record: replaying it is not the same as
/// the agent remembering the conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub enum TranscriptEvent {
    User {
        id: String,
        ts: i64,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<Attachment>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reactions: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
        /// Nobody typed this: a schedule fired. The text stays the prompt in
        /// full; this is what lets the transcript draw one line and keep the
        /// prompt behind an expander.
        #[serde(skip_serializing_if = "Option::is_none")]
        scheduled: Option<ScheduledRun>,
        /// An emphasis on this bubble.
        #[serde(skip_serializing_if = "Option::is_none")]
        ring: Option<RingIntent>,
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<Receipt>,
        /// Which of the person's apps this was written in. Never drawn: the
        /// teammate hears it beside the time, so "my browser" means the
        /// right one.
        #[serde(skip_serializing_if = "Option::is_none")]
        client: Option<Client>,
    },
    Agent {
        id: String,
        ts: i64,
        /// With a file, the caption, which may be empty.
        text: String,
        /// A file the teammate sent the person with `send_file`: one per
        /// message, read back with `file.read`.
        #[serde(skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<Attachment>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reactions: Option<Vec<String>>,
        /// An emphasis on this bubble.
        #[serde(skip_serializing_if = "Option::is_none")]
        ring: Option<RingIntent>,
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<Receipt>,
    },
    Thought {
        id: String,
        ts: i64,
        text: String,
    },
    Tool {
        id: String,
        ts: i64,
        tool_call_id: String,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_kind: Option<String>,
        status: ToolStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        locations: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<Vec<ToolOutput>>,
    },
    Permission {
        id: String,
        ts: i64,
        request_id: String,
        title: String,
        options: Vec<PermissionOption>,
        #[serde(skip_serializing_if = "Option::is_none")]
        decision: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        decided_option_name: Option<String>,
    },
    Plan {
        id: String,
        ts: i64,
        entries: Vec<PlanEntry>,
    },
    Notice {
        id: String,
        ts: i64,
        level: NoticeLevel,
        text: String,
    },
    /// The computer image being pulled for this teammate, as the runtime
    /// reports its layers: one line per pull, rewritten in place as layers
    /// land, so a minute of download is a bar and not a silence. `done` and
    /// `failed` are the line's afterlife. A runtime whose output the desk
    /// cannot count leaves `layersTotal` at zero, and the bar is indeterminate.
    ComputerPull {
        id: String,
        ts: i64,
        image: String,
        layers_done: u32,
        layers_total: u32,
        status: PullStatus,
        /// How long the pull took, once it is done.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<i64>,
    },
    /// A frame of the teammate's computer screen, taken as its capture tool
    /// ran. The chat is where the work actually happens, so what the agent saw
    /// belongs in it — a thumbnail, with the live screen a click away.
    ComputerFrame {
        id: String,
        ts: i64,
        data_url: String,
    },
    /// The agent asked the human to take an action it cannot — credentials, a
    /// 2FA tap, a CAPTCHA — usually on its computer. Pending renders a card
    /// with the way in; any other status is the card's afterlife.
    HumanAction {
        id: String,
        ts: i64,
        action_id: String,
        reason: String,
        status: HumanActionStatus,
        /// What the person said with their answer, when they said anything.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        /// The asking turn did not wait: the answer comes back to the
        /// teammate as a [`TranscriptEvent::Delivery`], whenever it comes.
        /// Such a card is not orphaned by a restart or a stop, because
        /// nothing was parked on it; only the person, or a day going by,
        /// settles it. Absent on a card a tool is waiting on.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivers: Option<bool>,
        /// The work thread the card was raised in, when it was raised in one:
        /// the card is on that thread's stream and not on the tape, and the
        /// answer goes back to that thread's agent. Absent on the tape's own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<String>,
    },
    /// A site asked the teammate's browser to make a passkey, under an
    /// arming the person started for that site: the request waits in the
    /// browser until the person answers here. Pending renders the card
    /// with Approve and Deny; any other status is the card's afterlife.
    PasskeyAsk {
        id: String,
        ts: i64,
        ask_id: String,
        /// The name the passkey is stored under once made.
        name: String,
        rp_id: String,
        origin: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rp_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_display_name: Option<String>,
        status: PasskeyAskStatus,
    },
    Peer {
        id: String,
        ts: i64,
        thread_key: String,
        with_persona_id: String,
        with_name: String,
        role: PeerRole,
        exchanges: i64,
        status: PeerStatus,
        /// What kind of citizen the other side is. Absent means a teammate —
        /// every marker written before client seats existed, and every one
        /// written for a teammate since. `client` means an outside MCP agent
        /// holding a seat in this room; `withName` already carries the desk
        /// it connected through, as in "Claude Code @ beastie".
        ///
        /// The field exists because the name alone cannot carry it: "Claude
        /// Code @ beastie" and "Boris @ beastie" read identically, and a
        /// message from outside the room must never look like one from a
        /// teammate.
        #[serde(skip_serializing_if = "Option::is_none")]
        seat: Option<PeerSeat>,
    },
    /// Something that came back to this teammate after it had moved on: a
    /// colleague's answer to a message it sent. Nobody typed it and it is not
    /// a bubble; the tape keeps it so it reaches the agent exactly once, in
    /// order, across a restart, and so the turn it starts can say why it
    /// started. `text` is what came back; the agent hears it framed.
    Delivery {
        id: String,
        ts: i64,
        cause: DeliveryCause,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<Receipt>,
        /// The thread it came from and the request it answers. Absent from a
        /// delivery written before this was kept, which is told apart by the
        /// id it was written under.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<DeliveryFrom>,
    },
    /// A pair reached the automatic message cap. Its queue waits for the
    /// person to Keep going or Stop exchange, across intents and restarts.
    ExchangePaused {
        id: String,
        ts: i64,
        with_persona_id: String,
        with_name: String,
        exchanges: i64,
        status: ExchangePauseStatus,
    },
    Turn {
        id: String,
        ts: i64,
        stop_reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<TokenUsage>,
    },
    /// A subagent this teammate started: one line on the teammate's tape,
    /// written again under the same id as the run goes, that opens the run's
    /// own transcript. The run itself never writes to this tape — what it did
    /// is on its own stream, `runs/<runId>`, and what it reported came back
    /// to the teammate as a job result.
    ///
    /// Stored as a thread's link (`thread::Link`) and sent as this, so a
    /// client that draws markers needs nothing else; lines written before
    /// links are this already.
    Subagent {
        id: String,
        /// When the run started. The line keeps its place as it is rewritten.
        ts: i64,
        run_id: String,
        /// The short label the teammate gave the task.
        title: String,
        status: SubagentStatus,
        /// How long it ran, once it has stopped.
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<i64>,
    },
    /// A side thread this teammate has going: a second conversation with the
    /// same teammate about another task, running in parallel with this one.
    /// One line on the main tape, written again under the same id as the
    /// thread goes, that opens the thread's own transcript at `sides/<sideId>`.
    /// Nothing the thread says is on this tape. While `status` is `live` the
    /// line reads "Started a side thread"; `parked` is the same thread with
    /// its agent let go of, still open; once `archived` it is a one-line
    /// `result` with an Open, and `note` holds the closing handoff note.
    ///
    /// Stored as a thread's link (`thread::Link`) and sent as this.
    Side {
        id: String,
        /// When the thread started. The line keeps its place as it is rewritten.
        ts: i64,
        side_id: String,
        persona_id: String,
        /// The task, shortened to a label.
        title: String,
        status: SideStatus,
        /// Once archived: what the thread came to, in a line. Absent when
        /// nothing was said worth keeping.
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        /// Once archived: who ended it.
        #[serde(skip_serializing_if = "Option::is_none")]
        archived_by: Option<SideEnd>,
        #[serde(skip_serializing_if = "Option::is_none")]
        archived_at: Option<i64>,
        /// Once archived: the closing handoff note (goal, what got done,
        /// what is still open, key files), when a model could write one.
        /// What `search_thread` finds the thread by.
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        /// The agent's own id for the thread's conversation, written after
        /// its first turn, and the harness that issued it. What lets a parked
        /// or archived thread be reopened with recall.
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        backend_id: Option<String>,
        /// The teammate that opened the thread, when one did: a handoff is a
        /// thread on the teammate it was handed to, and the line is on both
        /// teammates' tapes. Absent when the person opened it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opened_by: Option<SideOpener>,
    },
    /// A voice call with this teammate: one quiet line on the tape, written
    /// again under the same id as the call goes, that stands for the call
    /// kept in `calls/<callId>`. What was said on it is not on this tape
    /// unless the teammate was handed it. While `status` is `live` the line
    /// reads as a call in progress; once `ended` it says how long it lasted
    /// and what it came to.
    ///
    /// Stored as a thread's link (`thread::Link`) and sent as this. Clients
    /// that do not know the kind skip it.
    Call {
        id: String,
        /// When the call started. The line keeps its place as it is rewritten.
        ts: i64,
        call_id: String,
        /// "Call", or what the closing note called it.
        title: String,
        status: CallStatus,
        /// How long it lasted, once it has ended.
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<i64>,
        /// Once ended: what it came to, in a line, or how it ended.
        #[serde(skip_serializing_if = "Option::is_none")]
        outcome: Option<String>,
    },
    /// A thread that hangs off this one: the line a side thread, a run or a
    /// call leaves on its parent, written again under the same id as the
    /// thread goes. It is the stored model of what `Side`, `Subagent` and
    /// `Call` are drawn from, and a client that declared `threads2` is sent
    /// this for every kind in their place; one that did not is sent those.
    ///
    /// `threadKind` and not `kind`, because the event's own `kind` is `link`.
    Link {
        id: String,
        /// When the thread started. The line keeps its place as it is rewritten.
        ts: i64,
        /// The thread's key: with `threadKind`, its `ThreadId`.
        thread: String,
        thread_kind: ThreadKind,
        /// Whose thread it is. Absent from a run's marker written before links.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        persona_id: Option<String>,
        title: String,
        state: LinkState,
        /// Once closed: how it ended.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<crate::thread::End>,
        /// Once closed: what the thread came to, in a line.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<String>,
        /// When it closed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<i64>,
        /// The closing note.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        /// The agent's own id for the thread's conversation and the harness
        /// that issued it, once a turn has completed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backend_id: Option<String>,
        /// How long a run ran, once it has stopped.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<i64>,
        /// The teammate that opened the thread, when one did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opener_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opener_name: Option<String>,
    },
    /// A chapter boundary: one working context of the agent, marked in the
    /// tape it belongs to. Written once when the chapter opens and superseded
    /// by id when it closes, carrying what the next chapter needs to know.
    /// `sessionId` is the agent's own memory of this stretch, so a chapter
    /// can be reopened rather than merely summarised.
    Chapter {
        id: String,
        ts: i64,
        backend_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ended_at: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// The handoff note: goal, outcome, open loops, decisions, files.
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<ChapterStatus>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tags: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        closed_by: Option<ChapterClose>,
        /// Set on a chapter that reopened an earlier one's context.
        #[serde(skip_serializing_if = "Option::is_none")]
        resumed_from: Option<String>,
    },
}

/// A file that rides with a message, either way.
///
/// Handed to a teammate, everything is a path, pasted images included: a
/// pasted screenshot is written into the persona's `attachments` directory
/// before it is ever attached. That keeps one shape on the wire, keeps base64
/// out of the transcript on disk, and means an attachment can still be opened
/// months later from the record of the conversation that mentioned it.
/// For phone readback the desk separately retains supported user images at
/// send time; `file.read` selects that copy by message id and attachment index,
/// never by `path`. Older messages without a retained copy cannot be read back.
///
/// Sent by a teammate with `send_file`, the file is the desk's own copy,
/// under `files/` in the data directory, and `path` is where that copy is. A
/// phone never sees that path mean anything: it reads the file with
/// `file.read`, by the message's id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct Attachment {
    /// Images can be inlined for an agent that takes them, and are drawn in
    /// the conversation when a teammate sends one; files are linked.
    pub kind: AttachmentKind,
    /// Basename, which is all the composer and the bubble ever show.
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Bytes on disk, for the size a chip shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
    /// An image's size in pixels, so its place can be held before it loads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Where a teammate's file came from, in words a person reads: a path in
    /// its workspace or on its computer, or the part of the computer's screen
    /// a screenshot shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum AttachmentKind {
    Image,
    File,
}

/// A ring: an agent's mark on one of its own messages, saying "this is the
/// one". The set is closed and the theme owns the colours — an agent naming a
/// hex would be an agent doing visual design.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum RingIntent {
    Attention,
    Warning,
    Problem,
}

/// How far one message in a teammate-to-teammate thread has got.
///
/// `sent` is written when the message enters the thread — it has been accepted
/// for delivery and nothing more. `read` means the recipient's *agent* has
/// been handed it. Delivery to a machine is not the interesting fact and is
/// deliberately not a rung.
/// Which of the person's apps a message was written in: the desktop app on
/// this desk's own machine, or their phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum Client {
    Desktop,
    Phone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum Receipt {
    Sent,
    Read,
}

/// Where a paused exchange's card has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ExchangePauseStatus {
    Pending,
    Resumed,
    Stopped,
}

/// What a [`TranscriptEvent::Delivery`] answers, so a teammate juggling
/// several can tell them apart and a seat can say why a turn began.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts")]
pub enum DeliveryCause {
    /// A colleague answered a message this teammate sent it, or could not.
    /// `status` is `done` for an answer and `failed` for none; the words of
    /// the message and the answer are in the thread named by `threadKey`.
    Peer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        request_id: Option<String>,
        persona_id: String,
        name: String,
        thread_key: String,
        status: PeerStatus,
        /// The start of the message this answers, clipped to a line.
        about: String,
    },
    /// Work explicitly handed into this teammate's main conversation.
    /// The result returns automatically to requestId on the sender's tape.
    Handoff {
        request_id: String,
        persona_id: String,
        name: String,
        thread_key: String,
        /// The start of the message, clipped to a line.
        about: String,
    },
    /// The person answered a `request_human` card the teammate did not wait
    /// on, or a day went by without an answer (`expired`). `text` is the
    /// note they typed with it, or empty.
    Answer {
        action_id: String,
        status: HumanActionStatus,
        /// The start of the card's reason, clipped to a line.
        about: String,
    },
}

/// Where a delivery came from: the thread that produced it, and the request it
/// answers when it answers one. It is a field of the delivery, so the turn that
/// takes it never has to read an id to find out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct DeliveryFrom {
    /// The thread's key: see [`ThreadId`].
    pub thread: String,
    pub kind: ThreadKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
}

impl DeliveryFrom {
    pub fn new(thread: &ThreadId, request: Option<String>) -> Self {
        Self {
            thread: thread.key.clone(),
            kind: thread.kind,
            request,
        }
    }

    pub fn thread(&self) -> ThreadId {
        ThreadId::new(self.kind, self.thread.clone())
    }
}

/// What a firing stamps on the user event it writes.
///
/// The event's text is still the whole prompt, because debugging a schedule
/// means reading what it actually said. This is what lets the transcript show
/// one line instead — and what tells the quiet gate which job is speaking.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ScheduledRun {
    pub job_id: String,
    pub kind: ScheduleKind,
    pub name: String,
    /// The immutable provenance captured when the scheduler queued this run.
    /// A one-shot may be tombstoned before its queued turn reaches the driver,
    /// so dispatch cannot recover this fact from the room stream.
    #[serde(default)]
    pub operator_created: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet: Option<bool>,
}

/// `schedule` is once. `loop` is every interval until cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ScheduleKind {
    Schedule,
    Loop,
}

/// Work a teammate has asked Hotline to wake it for later.
///
/// `schedule` is once. `loop` is every `every` milliseconds until cancelled.
/// `nextAt` is the next fire, so the window can say when without doing the
/// math. Jobs live on the room stream as events of kind `schedule`; that
/// event kind is the stream's, not this `kind`, and a loop is recovered from
/// `every` being present. A delete is a tombstone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ScheduledJob {
    pub id: String,
    pub persona_id: String,
    pub kind: ScheduleKind,
    /// The original fire time of a one-shot, milliseconds since epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<i64>,
    /// The interval of a loop, in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub every: Option<i64>,
    pub prompt: String,
    /// The user asked this job for nothing in the chat. Absent means it
    /// speaks. Stored only when true, so "not quiet" has one representation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet: Option<bool>,
    /// True only when the authenticated desk created this job. Jobs created by
    /// a teammate, and jobs written by older versions without provenance,
    /// require that teammate's current background-work grant at every wake.
    #[serde(default)]
    pub operator_created: bool,
    pub next_at: i64,
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum ToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts")]
pub enum ToolOutput {
    Text {
        text: String,
    },
    Diff {
        path: String,
        /// Written as `null` rather than omitted when there was no old text,
        /// because that is what the ACP session writes and the record is
        /// whatever it wrote.
        #[ts(optional = nullable)]
        old_text: Option<String>,
        new_text: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct PlanEntry {
    pub content: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

/// What one turn's requests cost in tokens. On Hotline Agent the input counts
/// every input token, cached or not, and the two cache counts are part of it;
/// an ACP agent's numbers are the agent's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct TokenUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<i64>,
    /// Input the provider read back from its prompt cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<i64>,
    /// Input the provider wrote to its prompt cache for the next request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum NoticeLevel {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum PullStatus {
    Pulling,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum HumanActionStatus {
    Pending,
    Done,
    Dismissed,
    Expired,
}

/// Where a passkey card stands: waiting for the person, answered either
/// way, or gone — the request left with its page, the arming ran out or
/// was cancelled, or the computer stopped — before anyone answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum PasskeyAskStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

/// What the person can say to a waiting `request_human` card.
///
/// The tape still writes `done` or `dismissed` — `declined` is this
/// command's word for the afterlife the previous edition called dismissed, so
/// an imported tape still reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum HumanAnswer {
    Done,
    Declined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum PeerRole {
    Caller,
    Target,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum PeerStatus {
    Open,
    Done,
    Waiting,
    Failed,
}

/// Where a subagent's run has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SubagentStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

/// Whether a call is still going.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum CallStatus {
    Live,
    Ended,
}

/// Where a thread stands, in every kind's words: an agent is running it, it is
/// open with none, or it is over (and `end` says how).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum LinkState {
    Live,
    Parked,
    Closed,
}

/// Whether a side thread is still going.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SideStatus {
    /// An agent is running it.
    Live,
    /// Still open, with its agent let go of: nobody spoke in it for a few
    /// hours, or the desk restarted. Saying something in it brings it back.
    Parked,
    /// Ended on purpose. Read-only until it is continued.
    Archived,
}

/// Who or what ended a side thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SideEnd {
    /// The teammate said it was done (`archive_thread`).
    Agent,
    /// The person pressed Archive.
    Person,
    /// Nobody spoke in it for a few hours. Threads written before parking
    /// existed carry it; now such a thread is parked, not archived.
    Idle,
    /// The teammate was stopped, its policy changed, it was removed, or the
    /// desk restarted, so the thread's agent is gone.
    Stopped,
}

/// One side thread, as a list of them is read: live, parked and archived.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SideThreadSummary {
    pub side_id: String,
    pub persona_id: String,
    pub title: String,
    pub status: SideStatus,
    pub started_at: i64,
    /// The newest line in the thread.
    pub last_at: i64,
    /// A turn of the thread is running right now. Always false once archived.
    pub working: bool,
    /// A permission card in the thread is waiting for an answer.
    pub waiting: bool,
    /// The newest thing said in the thread, in a line: the teammate's last
    /// words, or what the person asked when it has said none yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_by: Option<SideEnd>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<i64>,
    /// The teammate that opened it, when one did: "from Mack" on the row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opened_by: Option<SideOpener>,
}

/// A teammate that opened a work thread on another by handing it work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SideOpener {
    pub persona_id: String,
    pub name: String,
}

/// An outside MCP agent holding a seat in this room, rather than a teammate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum PeerSeat {
    Client,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "contract.ts")]
pub enum ChapterStatus {
    InProgress,
    Done,
}

/// What ended a chapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ChapterClose {
    Idle,
    User,
    Agent,
    Resume,
}

// ---------------------------------------------------------------------------
// Views of the transcript
// ---------------------------------------------------------------------------

/// A chapter as the drawer lists it: the marker plus how much was said in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ChapterSummary {
    pub id: String,
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ChapterStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_by: Option<ChapterClose>,
    pub messages: i64,
}

/// The last thing either side said, shown under a name in the roster.
///
/// A roster of names alone says who is there; a roster with these says what is
/// going on. Only messages count — a teammate's last tool call is Hotline's
/// business, not a line of conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts")]
pub struct Preview {
    pub from: Side,
    pub text: String,
    pub at: i64,
}

/// Which side of a conversation with the user a line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum Side {
    Me,
    Them,
}

/// One hit from a thread search: a chapter by its note, or a message by its
/// text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub enum ThreadSearchHit {
    Chapter {
        chapter_id: String,
        ts: i64,
        title: String,
        excerpt: String,
        /// The status column as the index holds it, which is a word the
        /// indexer copied rather than one this build named.
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    Message {
        event_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        chapter_id: Option<String>,
        ts: i64,
        from: Side,
        excerpt: String,
    },
}

/// The last thing said in a peer thread, and which of the two said it.
///
/// Not [`Preview`]'s `me`/`them`: both sides are teammates and neither of them
/// is the person reading, so the name is spelled out rather than implied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct PeerPreview {
    pub from_name: String,
    pub text: String,
    pub at: i64,
}

/// One thread between two teammates, as a list of them is read.
///
/// `lastAt` is the newest thing in the thread, which is what the window counts
/// unread against — it remembers the latest it has shown, exactly as it does
/// for a tape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct PeerThreadSummary {
    pub thread_key: String,
    /// The other side, from the point of view of the teammate that asked.
    pub with_persona_id: String,
    pub with_name: String,
    /// How many turns have been answered in this thread.
    pub exchanges: i64,
    pub last_at: i64,
    /// A permission card in this thread is still waiting for an answer.
    pub waiting: bool,
    /// Who is mid-reply in *this* thread right now, or absent for nobody.
    ///
    /// Not "is that teammate busy": a teammate deep in its own conversation
    /// with the user is not working on this, and a line saying it is would be
    /// a lie the reader can check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_persona_id: Option<String>,
    /// Written as `null` rather than omitted, because a thread with nothing
    /// said in it is a row the window still draws.
    #[ts(optional = nullable)]
    pub preview: Option<PeerPreview>,
}

/// A search hit that names whose conversation it came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub enum GlobalSearchHit {
    Chapter {
        persona_id: String,
        chapter_id: String,
        ts: i64,
        title: String,
        excerpt: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    Message {
        persona_id: String,
        event_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        chapter_id: Option<String>,
        ts: i64,
        from: Side,
        excerpt: String,
    },
}

// ---------------------------------------------------------------------------
// What the room pushes
// ---------------------------------------------------------------------------

/// `transcriptAppended` and `transcriptUpdated`: one event, and whose tape it
/// belongs to. The two pushes differ in what the window does with the event,
/// not in what is on the wire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct TranscriptPush {
    pub persona_id: String,
    pub event: TranscriptEvent,
}

/// `peerThreadAppended` and `peerThreadUpdated`. A peer thread has no "me" in
/// it, so the line is addressed by thread rather than by teammate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct PeerThreadPush {
    pub thread_key: String,
    pub event: TranscriptEvent,
}

/// `streamDelta`: text arriving as the agent writes it. Never persisted — the
/// durable line is the `TranscriptEvent` that lands when the message is whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts")]
pub enum StreamDelta {
    AgentDelta {
        persona_id: String,
        message_id: String,
        text: String,
    },
    ThoughtDelta {
        persona_id: String,
        message_id: String,
        text: String,
    },
    /// Text arriving in a side thread. Addressed by side id, because a side
    /// thread is a second conversation with the same teammate and its words
    /// must never land in the main one. Subscribe to `{"side": "<sideId>"}`.
    SideAgentDelta {
        side_id: String,
        message_id: String,
        text: String,
    },
    SideThoughtDelta {
        side_id: String,
        message_id: String,
        text: String,
    },
    /// Text arriving in any thread, addressed by its `ThreadId`: what a client
    /// that declared `threads2` is sent in place of `AgentDelta`,
    /// `ThoughtDelta`, `SideAgentDelta` and `SideThoughtDelta`. Subscribe to
    /// `{"threadId": {"kind", "key"}}`.
    ThreadDelta {
        thread: ThreadId,
        message_id: String,
        kind: DeltaKind,
        text: String,
    },
    /// How far the teammate's computer image has downloaded. Drawn as a ring
    /// where the computer's button sits, so the conversation carries on
    /// around it; never written to the tape.
    ComputerPull {
        persona_id: String,
        layers_done: u32,
        layers_total: u32,
        status: PullStatus,
    },
}

/// What a `ThreadDelta` carries: words the agent is saying, or thinking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum DeltaKind {
    Text,
    Thought,
}

// ---------------------------------------------------------------------------
// Backends
// ---------------------------------------------------------------------------

/// One agent harness as the new-teammate sheet offers it: Hotline Agent, or an
/// ACP harness the registry knows. `unavailable` is absent when this machine
/// can start it and a sentence naming what is missing when it cannot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct BackendChoice {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

/// Where a fresh room stands on its way to a first turn, as the welcome
/// pane reads it. Derived from what the room already knows — its credentials,
/// the harnesses this machine can start, its default backend and its roster —
/// never from a stored "seen" flag: the pane is on screen exactly as long as
/// there is nothing else to show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct Welcome {
    /// The providers with a live credential, by name. A revoked one is not
    /// a way to run anything.
    pub providers: Vec<String>,
    /// The ACP harnesses this machine can start right now, Hotline Agent aside.
    pub harnesses: Vec<BackendChoice>,
    /// The room's default backend, which the first teammate form lands on.
    pub default_backend_id: String,
    /// Step one is done: a teammate could run, on a provider or on a harness
    /// that is the room's default.
    pub can_run: bool,
    /// How many teammates the room has. Past zero the pane is gone.
    pub teammates: usize,
}

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

/// Where a skill came from. `builtin` is bundled with Hotline and always on;
/// `gateway` is the operator's folder in the data directory, granted per
/// teammate; `home` is the person's own folder, `~/.agents/skills` unless
/// the room's `skillsHome` says otherwise, the standard place other agents
/// read too, whose entries are offered to teammates by switch and read from
/// where they are; `workspace` is the teammate's own `.agents/skills`,
/// written by the person or the teammate; `computer` is the guide the
/// running computer serves, at its release.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SkillSource {
    Builtin,
    Gateway,
    Home,
    Workspace,
    Computer,
}

/// Which of the offered skills — the gateway's, and the person's own that
/// are switched on — a teammate gets, the way [`McpPolicy`] says which
/// servers: `none` for a new teammate, `some` by name, or `all` including
/// skills offered later. Built-in skills are not governed here; they are
/// always in the workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SkillPolicy {
    pub mode: PolicyMode,
    pub names: Vec<String>,
}

impl Default for SkillPolicy {
    fn default() -> Self {
        SkillPolicy {
            mode: PolicyMode::None,
            names: Vec::new(),
        }
    }
}

/// One skill as the catalog lists it. `invalid` is absent when the folder is
/// a skill and a sentence saying what is wrong when it is not; an invalid
/// entry is listed so the person can fix it, never silently skipped. `path`
/// is where the folder is: inside the workspace for what a teammate sees,
/// on disk for the gateway and the person's own folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SkillEntry {
    pub source: SkillSource,
    pub name: String,
    pub description: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid: Option<String>,
    /// Whether a `home` entry is offered to teammates. Only a `home` entry
    /// has it: the gateway offers everything valid in it, and the rest are
    /// not the person's to offer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offered: Option<bool>,
    /// The release a computer's guide came from. Only a `computer` entry has
    /// one: the desk's own skills are the desk's version, a gateway folder is
    /// whatever the person put there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

// ---------------------------------------------------------------------------
// The wire
// ---------------------------------------------------------------------------

/// A bounded, repeatable upload chunk. The phone never supplies a desktop path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "contract.ts")]
pub struct MobileAttachmentChunk {
    pub id: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub size: u32,
    pub offset: u32,
    pub data: String,
}

/// Part of a kept file, as `file.read` answers it. `data` is
/// base64 and at most 512 KiB of the file; `next` is where the next part
/// starts, and is absent once `data` reaches the end.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct FileChunk {
    pub name: String,
    pub mime_type: String,
    /// The whole file's size in bytes.
    pub size: i64,
    pub offset: i64,
    pub data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<i64>,
}

/// Selected laptop cookies, transported inside the sealed owner channel.
/// `source_id` is a stable laptop ID, separating it from server browser imports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "contract.ts")]
pub struct CookieTransfer {
    pub source_id: String,
    pub browser_id: String,
    pub profile_id: String,
    pub domains: Vec<String>,
    #[ts(type = "Array<Record<string, unknown>>")]
    pub cookies: Vec<serde_json::Value>,
}

/// Exactly one upload destination: a server path or a filename staged by the desk.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged, deny_unknown_fields)]
#[ts(export, export_to = "contract.ts")]
pub enum UploadDestination {
    Name { name: String },
    Path { path: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ImagesStatus {
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spending: Option<SpendingSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spending_unavailable: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct VoiceModel {
    pub provider_id: String,
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct VoiceBudget {
    pub day_usd: f64,
    pub month_usd: f64,
    pub spent_day_usd: f64,
    pub spent_month_usd: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct VoiceStatus {
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub available: bool,
    /// Direct teammate calls need speech and budget, but no desk dispatcher.
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub direct_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stt: Option<VoiceModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tts: Option<VoiceModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_tts: Option<VoiceModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatcher: Option<VoiceModel>,
    pub budget: VoiceBudget,
}

/// A teammate's own voice: one of a speaking model's voices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct PersonaVoice {
    pub provider_id: String,
    pub model_id: String,
    pub voice: String,
}

/// One model a connected provider can be asked to do a job with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct CapabilityModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The voices that speak with this model, the provider's default first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voices: Option<Vec<String>>,
    /// The thinking levels this chat model takes, as a teammate's Effort
    /// picker lists them; absent when it takes none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efforts: Option<Vec<String>>,
}

/// A connected provider and the models it can do one job with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct CapabilityProvider {
    pub provider_id: String,
    pub provider_name: String,
    pub models: Vec<CapabilityModel>,
}

/// A provider, and the model and voice on it when they are known.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct CapabilityPick {
    pub provider_id: String,
    pub provider_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    /// The thinking level picked for the model; absent means the model's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// One job the owner can pick a provider for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct CapabilityJob {
    /// What the owner chose; absent means automatic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<CapabilityPick>,
    /// What automatic resolves to right now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic: Option<CapabilityPick>,
    /// Why nothing can do this job yet, as a sentence for a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
    /// Connected providers only, in the order they were connected.
    pub options: Vec<CapabilityProvider>,
}

/// The shared caps and what has been spent against them so far.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct CapabilitySpending {
    pub day_usd: f64,
    pub month_usd: f64,
    pub spent_day_usd: f64,
    pub spent_month_usd: f64,
    /// Set when a tally could not be read, so the spent figures are not whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct CapabilityOptions {
    pub images: CapabilityJob,
    pub stt: CapabilityJob,
    pub tts: CapabilityJob,
    pub dispatcher: CapabilityJob,
    pub spending: CapabilitySpending,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct VoiceCall {
    pub call_id: String,
    pub input: Vec<String>,
    pub output: String,
    #[serde(default)]
    pub input_mode: VoiceInputMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub persona_id: Option<String>,
}

/// Omission preserves remote audio transcription for existing callers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum VoiceInputMode {
    #[default]
    Audio,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum VoiceState {
    Listening,
    Thinking,
    Speaking,
    Held,
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum VoiceEndReason {
    Client,
    Goodbye,
    Budget,
    Replaced,
    Error,
    Idle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub enum VoiceEvent {
    State {
        state: VoiceState,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<VoiceEndReason>,
    },
    Heard {
        seq: u32,
        text: String,
    },
    Said {
        id: String,
        text: String,
    },
    Clip {
        id: String,
        index: u32,
        r#final: bool,
        mime_type: String,
        data: String,
    },
    Delivery {
        persona_id: String,
        event_id: String,
        text: String,
    },
    Card {
        persona_id: String,
        request_id: String,
        kind: String,
    },
}

/// Everything a client may ask the room to do or to answer.
///
/// One enum, so the window's whole API is generated from it and a command the
/// core does not know is a parse failure rather than a silent no-op. The names
/// are `noun.verb` and the frame is `{id, cmd, params}` — the tag and the
/// content of this enum, with the id beside them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "cmd",
    content = "params",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub enum Command {
    #[serde(rename = "voice.status")]
    VoiceStatus {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_mode: Option<VoiceInputMode>,
    },
    #[serde(rename = "voice.call_start")]
    VoiceCallStart {
        call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        persona_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stream_audio: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_mode: Option<VoiceInputMode>,
    },
    /// One finalized on-device transcript; partial recognition is never submitted.
    #[serde(rename = "voice.text")]
    VoiceText {
        call_id: String,
        seq: u32,
        text: String,
    },
    /// Negotiated PCM16 little-endian mono, 16 kHz. Empty final commits.
    #[serde(rename = "voice.audio")]
    VoiceAudio {
        call_id: String,
        seq: u32,
        index: u32,
        data: String,
        r#final: bool,
    },
    #[serde(rename = "voice.utterance")]
    VoiceUtterance {
        call_id: String,
        seq: u32,
        mime_type: String,
        data: String,
        duration_ms: u32,
    },
    #[serde(rename = "voice.interrupt")]
    VoiceInterrupt { call_id: String },
    #[serde(rename = "voice.hold")]
    VoiceHold { call_id: String, hold: bool },
    #[serde(rename = "voice.call_end")]
    VoiceCallEnd { call_id: String },
    /// Listener and pairing controls belong to the local desk, never a remote owner.
    #[serde(rename = "remote.status")]
    RemoteStatus {},
    #[serde(rename = "remote.configure")]
    RemoteConfigure { enabled: bool, host: String },
    #[serde(rename = "remote.devices")]
    RemoteDevices {},
    #[serde(rename = "remote.revoke")]
    RemoteRevoke { device_id: String },
    /// Stand in on a paired desk's relay so visitors reach this desk through
    /// it, or stop with no desk named. Kept, and used whenever Remote is on.
    #[serde(rename = "remote.relay")]
    RemoteRelay {
        #[serde(default)]
        desk_id: Option<String>,
    },
    /// Start a v2 invitation, poll/cancel its id, or explicitly request desktop legacy pairing.
    #[serde(rename = "remote.pairing")]
    RemotePairing {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<crate::remote::DeviceRole>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default)]
        cancel: bool,
        #[serde(default)]
        legacy: bool,
    },
    /// A paired phone supplies a stable operation id; retries never run twice.
    /// `replyTo` is the id of the message this one answers, as on
    /// `session.prompt`. `thread` says it in one of the teammate's work
    /// threads instead of the main conversation, with the same files and
    /// replies; absent, it is the main conversation.
    #[serde(rename = "mobile.prompt")]
    MobilePrompt {
        operation_id: String,
        persona_id: String,
        text: String,
        #[serde(default)]
        attachment_ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<ThreadId>,
    },
    #[serde(rename = "mobile.attachment")]
    MobileAttachment { upload: MobileAttachmentChunk },
    /// Operator-only server paths for remote pickers and file transfer.
    #[serde(rename = "files.browse")]
    FilesBrowse { path: String },
    #[serde(rename = "files.mkdir")]
    FilesMkdir { path: String },
    #[serde(rename = "files.download")]
    FilesDownload { path: String, offset: u64 },
    #[serde(rename = "files.upload_start")]
    FilesUploadStart(UploadDestination),
    #[serde(rename = "files.upload_chunk")]
    FilesUploadChunk {
        upload_id: String,
        offset: u64,
        data: String,
    },
    #[serde(rename = "files.upload_finish")]
    FilesUploadFinish { upload_id: String },
    #[serde(rename = "files.upload_cancel")]
    FilesUploadCancel { upload_id: String },
    /// A teammate file or a retained user image on that teammate's tape.
    /// `index` selects the original attachment position, defaulting to zero;
    /// teammate files accept only zero. The answer is a [`FileChunk`].
    #[serde(rename = "file.read")]
    FileRead {
        persona_id: String,
        event_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<u32>,
        #[serde(default)]
        offset: i64,
    },
    /// A teammate's kept picture, named by the hash on its record. Any seat
    /// may read one. The answer is a [`FileChunk`], a part at a time from
    /// `offset`.
    #[serde(rename = "avatar.read")]
    AvatarRead {
        persona_id: String,
        hash: String,
        #[serde(default)]
        offset: i64,
    },
    /// Draws a picture for a teammate from its name and goal through the
    /// room's image providers, within the spending cap, and puts it on the
    /// roster: what setup offers when images are available. Answers `null`
    /// once the drawing has started, since a socket's commands are answered
    /// in order and a picture takes up to a minute; the roster carries it
    /// when it lands. Refused at once when no provider can draw or the
    /// person chose the current picture.
    #[serde(rename = "avatar.generate")]
    AvatarGenerate { persona_id: String },
    /// Where to notify this phone: the token its push service issued, and
    /// which platform it is for. Sent by the phone after it connects.
    #[serde(rename = "mobile.push_register")]
    MobilePushRegister { token: String, platform: String },
    /// A narrow create for the phone seat. Core builds the whole draft and
    /// fills in everything posture-related with its safest defaults —
    /// workspace reach, a workspace this desk makes, no computer, no
    /// background work — because the phone has no way to ask for more.
    /// `requestId` is a phone-made uuid and becomes the teammate's id, so a
    /// retry after a lost acknowledgement returns the teammate already made
    /// instead of a second one. `backendId` must be a harness `backends.list`
    /// reports as ready; anything else is refused.
    #[serde(rename = "mobile.persona_create")]
    MobilePersonaCreate {
        request_id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        goal: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backend_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort_id: Option<String>,
    },
    /// A narrow edit for the phone seat: a new name, a new goal, or both,
    /// and nothing else a patch could carry. An absent field is left as it
    /// is; an empty goal clears it; a blank name is refused. Everything
    /// that is a standing grant — reach, tools, servers, computer,
    /// background work — stays behind `persona.update` at the desk.
    #[serde(rename = "mobile.persona_update")]
    MobilePersonaUpdate {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        goal: Option<String>,
    },
    /// The owner phone's access controls for a teammate: how far a Hotline
    /// Agent teammate reaches, which mode a harness teammate runs in (for
    /// some harnesses the mode is the permission posture), and whether it
    /// may keep its own background work. Owner seat only; a companion phone
    /// is refused. An absent field is left as it is. Reach and background
    /// work restart a live session the way `persona.update` does; a mode
    /// switches live when the teammate is running and is kept for its next
    /// start either way.
    #[serde(rename = "mobile.persona_access")]
    MobilePersonaAccess {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reach: Option<Reach>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        background_work: Option<bool>,
    },
    /// The owner phone's narrow computer controls. Omitted fields are kept;
    /// explicit null CPUs clear the limit. Limits apply on the next creation.
    #[serde(rename = "mobile.persona_computer")]
    MobilePersonaComputer {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memory: Option<String>,
        #[serde(
            default,
            with = "present_option",
            skip_serializing_if = "Option::is_none"
        )]
        #[ts(as = "Option<f64>", optional = nullable)]
        cpus: Option<Option<f64>>,
    },
    #[serde(rename = "persona.create")]
    PersonaCreate { draft: PersonaDraft },
    /// The patch is folded over the teammate's record and the whole record is
    /// written again, because a stream folds by id and a partial line would
    /// leave the fold holding half a teammate.
    #[serde(rename = "persona.update")]
    PersonaUpdate {
        id: String,
        #[ts(type = "Partial<Persona>")]
        patch: Value,
    },
    #[serde(rename = "persona.delete")]
    PersonaDelete { id: String },
    /// Pins a teammate to the top of the desk at `slot` (0-based), or unpins
    /// it when `slot` is absent. Pinning shifts the pins at and after the slot
    /// along, moving an already pinned teammate reorders, and a fourth pin is
    /// refused. Owners only: a companion phone sees the pins on the roster.
    #[serde(rename = "persona.pin")]
    PersonaPin {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slot: Option<u8>,
    },
    /// One key at a time, `null` clearing a key back to its default.
    #[serde(rename = "settings.update")]
    SettingsUpdate {
        #[ts(type = "Record<string, unknown>")]
        patch: Map<String, Value>,
    },
    #[serde(rename = "images.status")]
    ImagesStatus {},
    #[serde(rename = "capabilities.options")]
    CapabilitiesOptions {},
    #[serde(rename = "credential.create")]
    CredentialCreate {
        provider_id: String,
        label: String,
        secret: String,
    },
    /// Starts a device-code login. The command runs sequentially per socket,
    /// which is why this is start-then-poll rather than one blocking call:
    /// a waiting authorize would hold every later command on that socket
    /// until the person signed in.
    #[serde(rename = "credential.login")]
    CredentialLogin { provider_id: String },
    /// Stops a pending login, including its callback listener or device polling.
    #[serde(rename = "credential.login_cancel")]
    CredentialLoginCancel { login_id: String },
    /// Connects an Ollama server and discovers the models installed there.
    #[serde(rename = "credential.connect_local")]
    CredentialConnectLocal { base_url: String },
    #[serde(rename = "credential.custom_save")]
    CredentialCustomSave {
        id: Option<String>,
        draft: CustomProviderDraft,
    },
    /// Discovery can run before saving; a failed fetch leaves the connection alone.
    #[serde(rename = "credential.custom_models")]
    CredentialCustomModels {
        id: Option<String>,
        base_url: String,
        secret: Option<String>,
    },
    #[serde(rename = "credential.login_status")]
    CredentialLoginStatus { login_id: String },
    /// Discovers models through the connection's native Rig client and answers with
    /// that provider's catalogue as `models.catalog` would. An absent
    /// connection or a failed fetch is an error; the previous list survives.
    #[serde(rename = "credential.refresh_models")]
    CredentialRefreshModels { provider_id: String },
    #[serde(rename = "credential.revoke")]
    CredentialRevoke { id: String },
    #[serde(rename = "credential.delete")]
    CredentialDelete { id: String },
    /// Every agent harness this machine can start, and the ones it knows of
    /// but cannot, with the reason.
    #[serde(rename = "backends.list")]
    BackendsList {},
    /// The skills catalog: the built-ins, the gateway folder's entries valid
    /// or not, and — given a teammate — the skills in its own workspace.
    #[serde(rename = "skills.list")]
    SkillsList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        persona_id: Option<String>,
    },
    /// Copies a skill folder the person picked into the gateway, under its
    /// own name. Refused when the folder is not a skill or the gateway already
    /// has one of that name; the answer is the entry as `skills.list` lists it.
    #[serde(rename = "skills.add")]
    SkillsAdd { path: String },
    /// Removes a gateway skill by name. Teammates granted it lose it at
    /// their next start.
    #[serde(rename = "skills.remove")]
    SkillsRemove { name: String },
    /// Offers one of the person's own skills — a `home` entry — to
    /// teammates, or withdraws it. Nothing is copied: the skill is read from
    /// the person's folder at every start. Refused when the folder has no
    /// valid skill of that name; the answer is the entry as `skills.list`
    /// lists it. Teammates granted a withdrawn one lose it at their next
    /// start.
    #[serde(rename = "skills.offer")]
    SkillsOffer { name: String, offered: bool },
    /// Every credential the room knows of, never a secret.
    #[serde(rename = "credential.list")]
    CredentialList {},
    /// Starts OAuth discovery and a native callback for one HTTP MCP server.
    /// The answer carries only the authorization URL, login id and callback
    /// address; client secrets and tokens stay in the protected vault.
    #[serde(rename = "mcp.auth_start")]
    McpAuthStart { server_id: String },
    #[serde(rename = "mcp.auth_callback")]
    McpAuthCallback {
        login_id: String,
        callback_url: String,
    },
    #[serde(rename = "mcp.auth_status")]
    McpAuthStatus { server_id: String },
    #[serde(rename = "mcp.auth_reconnect")]
    McpAuthReconnect { server_id: String },
    #[serde(rename = "mcp.auth_sign_out")]
    McpAuthSignOut { server_id: String },
    /// Saves the token an HTTP MCP server in bearer or header mode sends,
    /// in the protected vault, bound to the server's URL. The settings entry
    /// never carries it; `mcp.auth_sign_out` forgets it.
    #[serde(rename = "mcp.secret_set")]
    McpSecretSet {
        server_id: String,
        url: String,
        secret: String,
    },
    /// Every provider Hotline Agent can hold a key for, whether or not this
    /// desk holds one.
    #[serde(rename = "providers.list")]
    ProvidersList {},
    /// Every model the desk's keys can reach, grouped by provider. An empty
    /// struct rather than a unit so `"params": {}` — what a client that
    /// always sends params spells it — reads the same as no params at all.
    #[serde(rename = "models.list")]
    ModelsList {},
    /// Every model the catalogue lists for one provider, flagged by the
    /// room's `enabledModels` filter. A subscription whose account list is
    /// on disk lists only those models. An unwired provider is an error.
    #[serde(rename = "models.catalog")]
    ModelsCatalog { provider_id: String },
    /// Replaces manual ids for the active connection, without changing its
    /// endpoint, credentials, selected models, or enabled-model filter.
    #[serde(rename = "models.manual_set")]
    ModelsManualSet {
        provider_id: String,
        model_ids: Vec<String>,
    },
    /// The effort levels a catalogue model offers, as picker choices. Empty
    /// when the id is unknown or the model has no `effort` option.
    #[serde(rename = "models.efforts")]
    ModelsEfforts { model_id: String },
    /// Operator-only harness sign-in. Executable configuration never crosses this wire.
    #[serde(rename = "agent.auth.start")]
    AgentAuthStart {
        persona_id: String,
        method_id: String,
    },
    #[serde(rename = "agent.auth.poll")]
    AgentAuthPoll { persona_id: String, id: String },
    #[serde(rename = "agent.auth.input")]
    AgentAuthInput {
        persona_id: String,
        id: String,
        input: String,
    },
    #[serde(rename = "agent.auth.cancel")]
    AgentAuthCancel { persona_id: String, id: String },
    #[serde(rename = "session.start")]
    SessionStart { persona_id: String },
    #[serde(rename = "session.stop")]
    SessionStop { persona_id: String },
    /// `replyTo` is the id of the message this one answers, and the
    /// attachments are files handed to the teammate alongside the words.
    #[serde(rename = "session.prompt")]
    SessionPrompt {
        persona_id: String,
        text: String,
        reply_to: Option<String>,
        attachments: Option<Vec<Attachment>>,
    },
    #[serde(rename = "session.cancel")]
    SessionCancel { persona_id: String },
    #[serde(rename = "session.set_model")]
    SessionSetModel {
        persona_id: String,
        model_id: String,
    },
    /// Switches the agent's mode. Only an agent that offers modes has one; the
    /// pickers a session reports are what says whether it does.
    #[serde(rename = "session.set_mode")]
    SessionSetMode { persona_id: String, mode_id: String },
    /// Sets a config the session offers beyond the model and the mode —
    /// Hotline Agent's effort, or an ACP harness's own option. For Hotline Agent
    /// the persona is written first, the same as `session.set_model`.
    #[serde(rename = "session.set_config")]
    SessionSetConfig {
        persona_id: String,
        config_id: String,
        value: String,
    },
    /// Answers a permission card the agent is waiting behind. Refused when
    /// nothing is waiting any more — the turn ended, the session stopped, or
    /// somebody else answered first — so a stale card cannot silently let an
    /// agent through.
    #[serde(rename = "session.answer_permission")]
    SessionAnswerPermission {
        persona_id: String,
        request_id: String,
        option_id: String,
    },
    /// Resets a paused pair's message count and releases its durable queue.
    #[serde(rename = "teammates.exchange_resume")]
    TeammatesExchangeResume { a: String, b: String },
    /// Stops a pair's queued automatic exchange without changing its grants.
    #[serde(rename = "teammates.exchange_stop")]
    TeammatesExchangeStop { a: String, b: String },
    /// Answers a card the agent posted with `request_human`. Refused when
    /// nothing is waiting any more — the deadline passed, the session
    /// stopped, the room restarted, or somebody else answered first. The
    /// note, when there is one, reaches the agent word for word.
    #[serde(rename = "human.answer")]
    HumanAnswer {
        persona_id: String,
        action_id: String,
        status: HumanAnswer,
        #[serde(default)]
        note: Option<String>,
    },
    #[serde(rename = "search.thread")]
    SearchThread {
        persona_id: String,
        query: String,
        limit: Option<i64>,
    },
    #[serde(rename = "search.all")]
    SearchAll { query: String, limit: Option<i64> },
    /// Older lines of a teammate's tape than the window its subscription
    /// opened with: the `limit` lines before `before`, or, with `through`,
    /// every line from a little before `through` up to `before`, so a search
    /// hit or a quoted reply can be scrolled to. Answers `{ events, more }`.
    #[serde(rename = "tape.page")]
    TapePage {
        persona_id: String,
        before: String,
        limit: Option<i64>,
        through: Option<String>,
    },
    #[serde(rename = "chapter.list")]
    ChapterList { persona_id: String },
    /// Copies an existing Hotline data directory into this room. The source is
    /// never written.
    #[serde(rename = "room.import")]
    RoomImport { from: String },
    /// Closes the teammate's open chapter now, and answers with the chapter it
    /// closed — its title and its note, which the summariser has written by
    /// the time this returns.
    #[serde(rename = "chapter.start_fresh")]
    ChapterStartFresh { persona_id: String },
    /// Reopens the previous chapter's full context in place of the current
    /// one. Only the chapter immediately before is offered.
    #[serde(rename = "chapter.resume")]
    ChapterResume { persona_id: String },
    /// What tools this teammate was given, where they came from, and — for
    /// anything absent — why. Null when it has never started under a Hotline
    /// that keeps a ledger.
    #[serde(rename = "teammate.tools")]
    TeammateTools { persona_id: String },
    /// Times are milliseconds. `when` is a one-shot's fire, milliseconds
    /// since epoch; `every` is a loop's interval. The parsers that take a
    /// string live with the scheduler, for the tool that will speak them.
    #[serde(rename = "schedule.create")]
    ScheduleCreate {
        persona_id: String,
        kind: ScheduleKind,
        when: Option<i64>,
        every: Option<i64>,
        prompt: String,
        quiet: Option<bool>,
    },
    #[serde(rename = "schedule.list")]
    ScheduleList {},
    #[serde(rename = "schedule.cancel")]
    ScheduleCancel { id: String },
    #[serde(rename = "schedule.set_quiet")]
    ScheduleSetQuiet { id: String, quiet: bool },
    /// Starts a side thread with this teammate: a second conversation, in its
    /// own context and in parallel with the main one, about the task in
    /// `text`. Answers the new thread's `SideThreadSummary`. With two already
    /// live, the one least recently used is parked to make room; refused only
    /// when both are mid-turn.
    #[serde(rename = "side.start")]
    SideStart { persona_id: String, text: String },
    /// Says something in a side thread. A parked thread is brought back first.
    /// Returns at once; the teammate's answer arrives on the thread's
    /// `{"side": sideId}` subscription.
    #[serde(rename = "side.prompt")]
    SidePrompt {
        side_id: String,
        text: String,
        #[serde(default)]
        attachments: Option<Vec<Attachment>>,
    },
    /// Stops the turn in flight in a side thread and drops what waited behind
    /// it. The thread stays live.
    #[serde(rename = "side.cancel")]
    SideCancel { side_id: String },
    /// Archives a side thread: its agent is stopped, its chip goes, and the
    /// main conversation's marker becomes a one-line result with Open.
    #[serde(rename = "side.archive")]
    SideArchive { side_id: String },
    /// Brings an archived or parked side thread back: a new agent, resuming
    /// the saved session when the harness can and reading the thread's own
    /// transcript when it cannot. Answers the thread's `SideThreadSummary`.
    #[serde(rename = "side.continue")]
    SideContinue { side_id: String },
    /// Every side thread this teammate has had: live first, then parked, then
    /// archived, each newest first.
    #[serde(rename = "side.list")]
    SideList { persona_id: String },
    /// Answers a permission card raised inside a side thread.
    #[serde(rename = "side.answer_permission")]
    SideAnswerPermission {
        side_id: String,
        request_id: String,
        option_id: String,
    },
    /// Every thread this teammate has with another teammate, newest first.
    /// The events of one of them are a `{"thread": "<key>"}` subscription,
    /// which is a stream like any other.
    #[serde(rename = "peers.list")]
    PeersList { persona_id: String },
    /// Says that these messages in a peer thread have been read, and answers
    /// how many of them that actually moved. A message that is already read,
    /// or an id naming nothing, moves nothing.
    #[serde(rename = "peers.mark_read")]
    PeersMarkRead { key: String, event_ids: Vec<String> },
    /// Answers a permission card raised in a peer thread, while the teammate
    /// who raised it is still answering there. Owner seat only.
    #[serde(rename = "peers.answer_permission")]
    PeersAnswerPermission {
        key: String,
        request_id: String,
        option_id: String,
    },
    /// Every runtime this machine knows how to drive, rootless-available first.
    #[serde(rename = "computer.runtimes")]
    ComputerRuntimes {},
    /// Read-only capacity for computer resource controls, available to every seat.
    #[serde(rename = "computer.capacity")]
    ComputerCapacity {},
    /// The release a new computer is created on, as the desk knows it now.
    #[serde(rename = "computer.releases")]
    ComputerReleases {},
    /// Asks the releases endpoint now instead of on the six-hour clock —
    /// the Settings button — and answers the same as `computer.releases`.
    #[serde(rename = "computer.releases.check")]
    ComputerReleasesCheck {},
    /// Where a fresh room stands on its way to a first turn.
    #[serde(rename = "welcome")]
    Welcome {},
    /// The window has the person, focused and used lately (`true`, said
    /// again while they keep at it), or has lost them: blurred, hidden or
    /// closed (`false`). While it has them, the phone is not notified of
    /// what the window already shows.
    #[serde(rename = "desk.looking")]
    DeskLooking { looking: bool },
    /// A peek: never wakes the container.
    #[serde(rename = "computer.status")]
    ComputerStatus { persona_id: String },
    #[serde(rename = "computer.stop")]
    ComputerStop { persona_id: String },
    #[serde(rename = "computer.remove")]
    ComputerRemove { persona_id: String },
    /// Recreates the computer on the release it would be created on now,
    /// and starts the teammate again if it was running. The volumes survive.
    #[serde(rename = "computer.update")]
    ComputerUpdate { persona_id: String },
    /// The browsers on the host the operator could bring cookies from, with
    /// their profiles. Names only; no cookie store is opened. Owner or local desk only —
    /// this reads the desk host, never a teammate's computer, and no agent
    /// tool can reach it.
    #[serde(rename = "computer.browsers.list")]
    ComputerBrowsersList {},
    /// The sites in one host browser profile and how many cookies each has,
    /// for the operator's picker. Domains and counts only; a value is never
    /// read out. Owner or local desk only.
    #[serde(rename = "computer.cookies.preview")]
    ComputerCookiesPreview {
        browser_id: String,
        profile_id: String,
    },
    /// Copies the cookies for the ticked `domains` from a host browser into
    /// the teammate's computer, so its browser starts signed in to them. The
    /// operator chooses the sites; the values pass host → desk → container and
    /// never touch the tape, the model, or a log. Owner or local desk only, and there is
    /// no agent tool that does this — the agent can never pull cookies itself.
    #[serde(rename = "computer.cookies.import")]
    ComputerCookiesImport {
        persona_id: String,
        browser_id: String,
        profile_id: String,
        domains: Vec<String>,
    },
    /// Selected laptop cookies, available to an owner or the local desk.
    #[serde(rename = "computer.cookies.push")]
    ComputerCookiesPush {
        persona_id: String,
        transfer: CookieTransfer,
    },
    /// What has been brought over to this teammate's computer, by browser
    /// and profile, with the sites: the record the pane lists. Owner or local desk only.
    #[serde(rename = "computer.cookies.list")]
    ComputerCookiesList { persona_id: String },
    /// Takes brought-over cookies back out of the teammate's computer: one
    /// site of an import when `domain` is given, the whole import when not.
    /// The computer's browser drops them at once, and the record answers as
    /// it stands afterwards. Starts the computer if it is stopped, the same
    /// as the import did. Owner or local desk only.
    #[serde(rename = "computer.cookies.forget")]
    ComputerCookiesForget {
        persona_id: String,
        browser_id: String,
        profile_id: String,
        #[serde(default)]
        domain: Option<String>,
    },
    /// The secrets the operator has stored for teammates: names and when
    /// each changed, never a value. Owner or local desk only.
    #[serde(rename = "secrets.list")]
    SecretsList {},
    /// Stores a secret under `name`, or replaces the one there, and hands the
    /// new value to every running computer granted that name. The value goes
    /// to the OS credential store and is never answered back — not by this
    /// command, not by a list, not by a subscription. Owner or local desk only.
    #[serde(rename = "secrets.set")]
    SecretsSet { name: String, value: String },
    /// Takes a stored secret away, and out of every running computer it was
    /// granted to. Owner or local desk only.
    #[serde(rename = "secrets.delete")]
    SecretsDelete { name: String },
    /// Stores a login under `name`, or replaces the record there: the sites
    /// its fields may be typed on, a username, a password, and a TOTP seed
    /// when the sign-in asks for a code. Answered like `secrets.set`. Desk
    /// seat only.
    #[serde(rename = "secrets.login.set")]
    SecretsLoginSet {
        name: String,
        sites: Vec<String>,
        username: String,
        password: String,
        #[serde(default)]
        totp: Option<String>,
    },
    /// Arms `persona_id`'s computer, starting it if need be, to make one
    /// passkey for `rp_id` in the next ten minutes, to be stored under
    /// `name` and ticked for that teammate. The person then adds the passkey
    /// in the site's own settings through the computer's screen, or asks the
    /// teammate to. Owner or local desk only.
    #[serde(rename = "secrets.passkey.register")]
    SecretsPasskeyRegister {
        name: String,
        persona_id: String,
        rp_id: String,
    },
    /// Where that stands: polled by the window while armed. `asked` carries
    /// the site's request, which waits for the card on the teammate's tape
    /// to be answered; the look that finds the passkey made stores it,
    /// ticks it, hands the computer its set, ends the arming and answers
    /// `stored`. Owner or local desk only.
    #[serde(rename = "secrets.passkey.registration")]
    SecretsPasskeyRegistration { persona_id: String },
    /// Answers the passkey card on a teammate's tape. Approved, the browser
    /// makes the passkey and the room stores it and ticks it; denied, the
    /// site hears no and the arming ends. One answer to one request, which
    /// the phone may give too; refused when no such request is waiting.
    #[serde(rename = "secrets.passkey.answer")]
    SecretsPasskeyAnswer {
        persona_id: String,
        ask_id: String,
        approved: bool,
    },
    /// Ends an arming without a passkey. Owner or local desk only.
    #[serde(rename = "secrets.passkey.cancel")]
    SecretsPasskeyCancel { persona_id: String },
    /// Says which of the wire's newer shapes this socket understands, so the
    /// core sends it those and not the older ones: `threads2` is `link` events
    /// and `ThreadDelta`s in place of the per-kind markers and deltas. Answers
    /// `{capabilities}`, what the core can do for this seat, and takes effect
    /// on every frame sent after it, including on subscriptions already open.
    /// A name the core does not know is ignored.
    #[serde(rename = "client.hello")]
    ClientHello { capabilities: Vec<String> },
    /// The threads of one teammate, or of the whole room when `personaId` is
    /// absent: each kind's own rows as one `ThreadSummary`, live first, then
    /// parked, then closed, each newest first. A phone is not shown pairs.
    #[serde(rename = "thread.list")]
    ThreadList { persona_id: Option<String> },
    /// Opens a work thread with this teammate about `text`, already running
    /// its first turn. Empty `text` opens it untitled and idle, named by the
    /// first line said in it. Answers the new thread's `ThreadSummary`.
    #[serde(rename = "thread.open")]
    ThreadOpen { persona_id: String, text: String },
    /// Says something in a thread, and returns at once: the answer arrives on
    /// the thread's `{"threadId": …}` subscription. A parked work thread is
    /// brought back first. The person speaks in the main conversation and in
    /// a work thread; a pair, a run and a call are not spoken in.
    #[serde(rename = "thread.prompt")]
    ThreadPrompt {
        thread: ThreadId,
        text: String,
        reply_to: Option<String>,
        attachments: Option<Vec<Attachment>>,
    },
    /// Stops the turn in flight and drops what waited behind it; the thread
    /// stays as it was. On a pair it stops the automatic exchange.
    #[serde(rename = "thread.cancel")]
    ThreadCancel { thread: ThreadId },
    /// Lets go of a work thread's agent and keeps the thread open: saying
    /// something in it brings one back.
    #[serde(rename = "thread.park")]
    ThreadPark { thread: ThreadId },
    /// Ends a work thread: its agent is stopped and its transcript is kept,
    /// read-only until it is continued.
    #[serde(rename = "thread.close")]
    ThreadClose { thread: ThreadId },
    /// Brings a parked or closed work thread back, as a `ThreadSummary`. On a
    /// pair it lets a paused exchange go on.
    #[serde(rename = "thread.continue")]
    ThreadContinue { thread: ThreadId },
    /// Answers a card raised in a thread: a permission, or a request the
    /// teammate made of the person. Routed by the kind's policy; a thread
    /// whose cards nobody answers (a run, a call) refuses.
    #[serde(rename = "thread.answer")]
    ThreadAnswer {
        thread: ThreadId,
        answer: ThreadAnswer,
    },
    /// Older lines of a thread than its subscription opened with, as
    /// `tape.page` reads a tape's: the `limit` lines before `before`, or with
    /// `through` every line from a little before it. Answers `{events, more}`.
    #[serde(rename = "thread.page")]
    ThreadPage {
        thread: ThreadId,
        before: String,
        limit: Option<i64>,
        through: Option<String>,
    },
}

/// The answer to a card in a thread, by the kind of card it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub enum ThreadAnswer {
    /// An option on a permission card.
    Permission {
        request_id: String,
        option_id: String,
    },
    /// A `request_human` card: done, or declined with a note the teammate
    /// hears word for word.
    Human {
        action_id: String,
        status: HumanAnswer,
        #[serde(default)]
        note: Option<String>,
    },
}

/// One thread as a list of every kind reads: what a person scanning them
/// needs, whichever kind each is. The kind is `thread.kind`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ThreadSummary {
    pub thread: ThreadId,
    /// The teammate whose thread it is. A pair is listed with the first of its
    /// two, and `with` names the other.
    pub persona_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_persona_id: Option<String>,
    /// What the thread is called: a work thread's or run's task, a call's
    /// title. A DM and a pair have none, and are named by who is in them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub state: LinkState,
    /// How a closed thread ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<crate::thread::End>,
    /// The teammate that opened it, when one did: "from Mack" on the row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opener: Option<SideOpener>,
    pub started_at: i64,
    /// The newest thing in the thread: what the window counts unread against.
    pub updated_at: i64,
    /// A turn of the thread is running right now.
    pub working: bool,
    /// A card in the thread is waiting for an answer.
    pub waiting: bool,
    /// The newest thing said in the thread, in a line: the teammate's last
    /// words, or what was asked when it has said none yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// What a closed thread came to, in a line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// What a subscription is a subscription to: a stream, or a view the core
/// maintains and nobody logs.
///
/// Externally tagged, so a stream reads as the word or the pair naming it —
/// `"room"`, `{"tape": "<personaId>"}`, `{"thread": "<key>"}`, `{"run":
/// "<runId>"}`, `{"view": "roster"}`, `{"schedules": "<personaId>"}` — which
/// is the shape the window would have written by hand. `{"thread": "<key>"}`
/// is a pair's, and stays so; any thread of any kind is `{"threadId": {"kind",
/// "key"}}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum Target {
    Call(String),
    Room,
    Tape(String),
    /// A thread between two teammates, by its pair key. The name predates
    /// threads of every kind and means this for clients that know no other.
    Thread(String),
    /// Any thread, by kind and key: its own transcript and live words. For a
    /// DM it is the teammate's tape, and for a work thread, a run and a call
    /// the stream of that name.
    #[serde(rename = "threadId")]
    ThreadId(ThreadId),
    /// One subagent run's own transcript, by run id.
    Run(String),
    /// One side thread's own transcript and live words, by side id.
    Side(String),
    View(ViewName),
    /// One teammate's scheduled jobs and loops, by persona id, as
    /// `ScheduleEntry`s: the whole list on opening and the whole list again
    /// whenever it changes. The way a phone reads them, since the room stream
    /// that holds them is the desk's alone.
    Schedules(String),
}

/// One of a teammate's scheduled jobs as the schedules view shows it: what a
/// person reading the list needs, and nothing that decides what the job may
/// do.
///
/// `kind` is `schedule` for once and `loop` for every `every` milliseconds.
/// `when` is a one-shot's original time, milliseconds since the epoch.
/// `nextAt` is when the desk next means to wake the teammate for it, also
/// milliseconds since the epoch, and a plan rather than a promise: a desk
/// that is closed or asleep then fires it once when it can, a fire that
/// could not start tries again a minute later, and a loop counts its next
/// interval from when a run ends. There is no paused or failed job — a job
/// is listed until it has fired for the last time or is cancelled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ScheduleEntry {
    pub id: String,
    pub persona_id: String,
    pub kind: ScheduleKind,
    /// What the teammate is asked when the job fires, which is also the only
    /// name a job has.
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub every: Option<i64>,
    pub next_at: i64,
    /// Present and true when the job was asked to say nothing in the chat
    /// unless it finds something worth saying.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet: Option<bool>,
}

impl From<ScheduledJob> for ScheduleEntry {
    fn from(job: ScheduledJob) -> Self {
        Self {
            id: job.id,
            persona_id: job.persona_id,
            kind: job.kind,
            prompt: job.prompt,
            when: job.when,
            every: job.every,
            next_at: job.next_at,
            quiet: job.quiet,
        }
    }
}

/// The views the core maintains. One so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ViewName {
    Roster,
}

/// One row of the roster view: who the teammate is, the last thing either
/// side said, and what its session is doing.
///
/// Three sources — the room stream, the tape's tail, the live session — that
/// the window would otherwise have to join for itself on every change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct RosterEntry {
    pub persona: Persona,
    /// Absent for a teammate that has never spoken.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<Preview>,
    /// The preview's `at`, kept beside it so the window can count unread
    /// without opening every tape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<i64>,
    /// The title of the tool still running on this tape, only while the
    /// session is thinking. Absent, not null, when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// A card on this tape is still waiting on the person: a permission with
    /// no decision, or a `request_human` or passkey card still pending.
    ///
    /// On the row so a phone can say "Needs you" for every teammate on
    /// every desk from one roster, without opening each tape. Always
    /// written, so a phone can tell `false` from a desk that predates it.
    pub waiting: bool,
    /// Its picture is being drawn: the face shows it is on its way rather
    /// than leaving the initial looking final.
    #[serde(default)]
    pub drawing: bool,
    /// Where this teammate is pinned on the desk, 0-based, absent when it is
    /// not. On the row so a companion phone, which cannot read settings, sees
    /// the same pins as the desk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<u8>,
    /// Subagents this teammate has running, oldest first, so the
    /// conversation can show them without scrolling back to their lines.
    /// Absent when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<RunningSubagent>>", optional)]
    pub subagents: Vec<RunningSubagent>,
    /// Side threads this teammate has live, oldest first. Absent when there
    /// are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<RunningSide>>", optional)]
    pub sides: Vec<RunningSide>,
    pub session: SessionInfo,
}

/// A subagent still running, as its teammate's roster row lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct RunningSubagent {
    pub run_id: String,
    /// The short label the teammate gave the task.
    pub title: String,
    pub started_at: i64,
}

/// A side thread still live, as its teammate's roster row lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct RunningSide {
    pub side_id: String,
    pub title: String,
    pub started_at: i64,
    /// A turn of the thread is running right now.
    pub working: bool,
}

/// With `default`, absence is None; a present null is Some(None).
mod present_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Some)
    }

    pub fn serialize<S, T>(value: &Option<Option<T>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        value.serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    //! What the readers already answer, run through these types and back out.
    //!
    //! The tape, chapter, preview and search readers build their JSON key by
    //! key, and they stay that way until a later task hands them these
    //! structs. So the way to prove the structs are the same contract is to
    //! take what those readers emit, parse it, write it again, and find
    //! nothing changed. A field spelled differently here, or dropped, or
    //! written as `null` where the reader wrote nothing at all, fails here.
    //! The persona's proof is `room::roster`'s, because the room stream is
    //! where a teammate is written down.

    use super::*;
    use crate::log::{Log, StreamId};
    use crate::paths::transcript_segments_dir;
    use crate::store::search::fixture::{chapter, index, message};
    use crate::store::{chapters, previews, search};
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};

    #[test]
    fn voice_input_mode_is_additive_and_legacy_calls_keep_audio() {
        for value in [
            json!({"cmd":"voice.status","params":{}}),
            json!({"cmd":"voice.call_start","params":{"callId":"fixture"}}),
        ] {
            let command = serde_json::from_value::<Command>(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(command).unwrap(), value);
        }
        let old_reply: VoiceCall = serde_json::from_value(json!({
            "callId":"fixture", "input":["audio/wav"], "output":"audio/wav"
        }))
        .unwrap();
        assert_eq!(old_reply.input_mode, VoiceInputMode::Audio);
        let text_start = serde_json::from_value::<Command>(json!({
            "cmd":"voice.call_start", "params":{"callId":"fixture","inputMode":"text"}
        }))
        .unwrap();
        assert!(matches!(
            text_start,
            Command::VoiceCallStart {
                input_mode: Some(VoiceInputMode::Text),
                ..
            }
        ));
        assert!(
            serde_json::from_value::<Command>(json!({
                "cmd":"voice.call_start", "params":{"callId":"fixture","inputMode":"partial"}
            }))
            .is_err()
        );
    }

    #[test]
    fn direct_voice_readiness_is_an_additive_optional_field() {
        let old: VoiceStatus = serde_json::from_value(json!({
            "capabilities":["voiceDirectCalls"], "available":true,
            "budget":{"dayUsd":2,"monthUsd":20,"spentDayUsd":0,"spentMonthUsd":0}
        }))
        .unwrap();
        assert!(!old.direct_available);
        assert!(VoiceStatus::decl(&ts_rs::Config::default()).contains("directAvailable?: boolean"));
    }

    #[test]
    fn images_settings_and_status_have_additive_wire_shapes() {
        let config = ts_rs::Config::default();
        assert_eq!(
            serde_json::to_value(ImageSettings::default()).unwrap(),
            json!({})
        );
        assert!(ImageSettings::decl(&config).contains("provider?: string"));
        assert!(ImageSettings::decl(&config).contains("model?: string"));
        assert!(SpendingSettings::decl(&config).contains("dayUsd: number"));
        assert!(SpendingSettings::decl(&config).contains("monthUsd: number"));
        assert!(ImagesStatus::decl(&config).contains("unavailable?: string"));
        assert!(ImagesStatus::decl(&config).contains("provider?: string"));
        assert!(ImagesStatus::decl(&config).contains("model?: string"));
        assert!(ImagesStatus::decl(&config).contains("spending?: SpendingSummary"));
        assert!(ImagesStatus::decl(&config).contains("spendingUnavailable?: string"));
        let command = Command::ImagesStatus {};
        let value = json!({"cmd": "images.status", "params": {}});
        assert_eq!(serde_json::to_value(&command).unwrap(), value);
        assert_eq!(serde_json::from_value::<Command>(value).unwrap(), command);
    }

    fn scratch(name: &str) -> (PathBuf, Log) {
        let dir = std::env::temp_dir().join(format!(
            "hotline-core-contract-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("transcripts")).unwrap();
        let log = Log::open(&dir);
        (dir, log)
    }

    /// Parse `value` as `T` and write it back out. Anything but the value that
    /// went in is a contract that has drifted from the reader.
    fn round_trip<T>(value: &Value) -> Value
    where
        T: Serialize + serde::de::DeserializeOwned,
    {
        let parsed: T = serde_json::from_value(value.clone())
            .unwrap_or_else(|error| panic!("{value} does not read as this type: {error}"));
        serde_json::to_value(parsed).unwrap()
    }

    fn write_tape(root: &Path, persona_id: &str, events: &[Value]) {
        let dir = transcript_segments_dir(root, persona_id);
        std::fs::create_dir_all(&dir).unwrap();
        let lines: Vec<String> = events.iter().map(ToString::to_string).collect();
        std::fs::write(dir.join("1.jsonl"), lines.join("\n")).unwrap();
    }

    /// A tape holding one line of every kind, each carrying every optional
    /// field it can — and, alongside them, the barest legal spelling of the
    /// kinds whose optional fields are worth seeing omitted.
    fn every_kind() -> Vec<Value> {
        vec![
            json!({
                "kind": "user", "id": "u1", "ts": 1, "text": "hi",
                "attachments": [{ "kind": "image", "name": "shot.png", "path": "/tmp/shot.png", "mimeType": "image/png", "size": 12 }],
                "reactions": ["🐸"], "replyTo": "a0",
                "scheduled": { "jobId": "j1", "kind": "loop", "name": "daily", "operatorCreated": false, "quiet": true },
                "ring": "attention", "receipt": "read",
            }),
            json!({ "kind": "user", "id": "u2", "ts": 2, "text": "bare" }),
            json!({
                "kind": "agent", "id": "a1", "ts": 3, "text": "hello",
                "reactions": ["👍"], "ring": "problem", "receipt": "sent",
            }),
            json!({ "kind": "thought", "id": "th1", "ts": 4, "text": "still thinking" }),
            json!({
                "kind": "tool", "id": "t1", "ts": 5, "toolCallId": "call-1", "title": "Edit",
                "toolKind": "edit", "status": "in_progress", "locations": ["/tmp/a"],
                "output": [
                    { "type": "text", "text": "done" },
                    { "type": "diff", "path": "/tmp/a", "oldText": null, "newText": "next" },
                    { "type": "diff", "path": "/tmp/b", "oldText": "was", "newText": "next" },
                ],
            }),
            json!({ "kind": "tool", "id": "t2", "ts": 6, "toolCallId": "call-2", "title": "Read", "status": "completed" }),
            json!({
                "kind": "permission", "id": "p1", "ts": 7, "requestId": "r1", "title": "Write?",
                "options": [{ "optionId": "yes", "name": "Allow", "kind": "allow_once" }],
                "decision": "yes", "decidedOptionName": "Allow",
            }),
            json!({ "kind": "plan", "id": "pl1", "ts": 8, "entries": [{ "content": "ship it", "status": "pending", "priority": "high" }] }),
            json!({ "kind": "notice", "id": "n1", "ts": 9, "level": "warn", "text": "careful" }),
            json!({ "kind": "computer_frame", "id": "cf1", "ts": 10, "dataUrl": "data:image/png;base64,AA" }),
            json!({ "kind": "human_action", "id": "ha1", "ts": 11, "actionId": "act-1", "reason": "log in", "status": "pending" }),
            json!({
                "kind": "peer", "id": "pe1", "ts": 12, "threadKey": "ada|bob", "withPersonaId": "bob",
                "withName": "Boris", "role": "caller", "exchanges": 2, "status": "waiting", "seat": "client",
            }),
            json!({ "kind": "turn", "id": "tu1", "ts": 13, "stopReason": "end_turn", "usage": { "inputTokens": 1, "outputTokens": 2, "totalTokens": 3 } }),
            json!({
                "kind": "chapter", "id": "c1", "ts": 14, "backendId": "hotline", "sessionId": "s1",
                "endedAt": 20, "title": "First", "note": "did the thing", "status": "done",
                "tags": ["harbour"], "closedBy": "idle", "resumedFrom": "c0",
            }),
            json!({ "kind": "chapter", "id": "c2", "ts": 21, "backendId": "hotline" }),
        ]
    }

    #[test]
    fn every_line_the_tape_holds_reads_back_as_the_transcript_event_it_was() {
        let (root, log) = scratch("contract-transcript");
        write_tape(&root, "ada", &every_kind());

        let loaded = log.load(&StreamId::Tape("ada".into()));
        assert_eq!(loaded.len(), every_kind().len());
        for event in &loaded {
            assert_eq!(&round_trip::<TranscriptEvent>(event), event);
        }
    }

    #[test]
    fn the_drawers_chapters_and_the_rosters_preview_read_back_unchanged() {
        let (root, log) = scratch("contract-views");
        write_tape(&root, "ada", &every_kind());

        let listed = chapters::list(&log, "ada");
        assert_eq!(listed.len(), 2);
        for summary in &listed {
            assert_eq!(&round_trip::<ChapterSummary>(summary), summary);
        }

        let preview = previews::preview(&root, "ada").unwrap();
        assert_eq!(round_trip::<Preview>(&preview), preview);
    }

    #[test]
    fn both_search_hits_read_back_as_the_hit_the_index_answered() {
        let (root, database) = index("contract-hits");
        message(&database, "ada", "m1", "user", "the harbour crane");
        message(&database, "bob", "m2", "agent", "the harbour again");
        chapter(
            &database,
            "bob",
            "c1",
            "Harbour week",
            "we fixed the harbour",
        );

        let thread = search::search(&root, "ada", "harbour", None).unwrap();
        let thread_hits = thread["hits"].as_array().unwrap();
        assert_eq!(thread_hits.len(), 1);
        for hit in thread_hits {
            assert_eq!(&round_trip::<ThreadSearchHit>(hit), hit);
        }

        let everyone = search::search_all(&root, "harbour", None).unwrap();
        let global_hits = everyone["hits"].as_array().unwrap();
        assert_eq!(global_hits.len(), 3);
        for hit in global_hits {
            assert_eq!(&round_trip::<GlobalSearchHit>(hit), hit);
        }
    }
}
