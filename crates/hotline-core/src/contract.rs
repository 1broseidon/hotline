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

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

// ---------------------------------------------------------------------------
// Faces
// ---------------------------------------------------------------------------

/// A teammate's face: the activity mark, wearing something it chose.
///
/// The parts are closed vocabularies rather than free drawing, so that every
/// pick renders clean; the geometry and the veto list over the combinations
/// the eye rejects live in `src/shared/face.ts`, which owns the rendering and
/// the judgement and takes the vocabulary from here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts")]
pub struct Face {
    /// The vocabulary's version, and so far its only one.
    #[ts(type = "1")]
    pub v: u8,
    /// OKLCH hue of the disc; lightness and chroma are fixed app-wide.
    ///
    /// Kept as the number it was written as rather than as a float, because
    /// re-spelling a stored `70` as `70.0` would be a teammate whose record
    /// changed on the way through a process that only meant to read it.
    #[ts(type = "number")]
    pub hue: serde_json::Number,
    pub body: FaceBody,
    pub eyes: FaceEyes,
    pub mouth: FaceMouth,
    pub hat: FaceHat,
    pub marks: FaceMarks,
    pub pattern: FacePattern,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum FaceBody {
    Round,
    Wide,
    Tall,
    Squat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum FaceEyes {
    Round,
    Half,
    Wide,
    Narrow,
    Asym,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum FaceMouth {
    None,
    Flat,
    Smile,
    Smirk,
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum FaceHat {
    None,
    Crown,
    Beanie,
    Beret,
    Halo,
    Antenna,
    Sprout,
}

/// `Spots` and `Stripe` are retired from what an agent may choose and stay in
/// the vocabulary anyway: a stored face may still wear them, and curation
/// reads them as their nearest living kin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum FaceMarks {
    None,
    Spots,
    Stripe,
    Freckles,
    Monocle,
}

/// `Spotted` is retired the same way — a stored spotted disc reads as a
/// lilypad now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum FacePattern {
    Solid,
    Spotted,
    Waterline,
    Ripples,
    Lilypad,
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
    /// The icon the agent chose for itself at creation. Absent on teammates
    /// made before faces existed, who keep the hashed-colour initial.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub face: Option<Face>,
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
    },
    Agent {
        id: String,
        ts: i64,
        text: String,
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

/// Something handed to a teammate alongside a message.
///
/// Everything is a path, pasted images included: a pasted screenshot is
/// written into the persona's `attachments` directory before it is ever
/// attached. That keeps one shape on the wire, keeps base64 out of the
/// transcript on disk, and means an attachment can still be opened months
/// later from the record of the conversation that mentioned it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct Attachment {
    /// Images can be inlined for an agent that takes them; files are linked.
    pub kind: AttachmentKind,
    /// Basename, which is all the composer and the bubble ever show.
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Bytes on disk, for the size a chip shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum Receipt {
    Sent,
    Read,
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
    /// A paired phone supplies a stable operation id; retries never run twice.
    /// `replyTo` is the id of the message this one answers, as on
    /// `session.prompt`.
    #[serde(rename = "mobile.prompt")]
    MobilePrompt {
        operation_id: String,
        persona_id: String,
        text: String,
        #[serde(default)]
        attachment_ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
    },
    #[serde(rename = "mobile.attachment")]
    MobileAttachment { upload: MobileAttachmentChunk },
    /// Where to notify this phone: the token its push service issued, and
    /// which platform it is for. Sent by the phone after it connects.
    #[serde(rename = "mobile.push_register")]
    MobilePushRegister { token: String, platform: String },
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
    /// One key at a time, `null` clearing a key back to its default.
    #[serde(rename = "settings.update")]
    SettingsUpdate {
        #[ts(type = "Record<string, unknown>")]
        patch: Map<String, Value>,
    },
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
    /// Every runtime this machine knows how to drive, rootless-available first.
    #[serde(rename = "computer.runtimes")]
    ComputerRuntimes {},
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
    /// their profiles. Names only; no cookie store is opened. Desk seat only —
    /// this reads the person's own machine, never the agent's, and no agent
    /// tool can reach it.
    #[serde(rename = "computer.browsers.list")]
    ComputerBrowsersList {},
    /// The sites in one host browser profile and how many cookies each has,
    /// for the operator's picker. Domains and counts only; a value is never
    /// read out. Desk seat only.
    #[serde(rename = "computer.cookies.preview")]
    ComputerCookiesPreview {
        browser_id: String,
        profile_id: String,
    },
    /// Copies the cookies for the ticked `domains` from a host browser into
    /// the teammate's computer, so its browser starts signed in to them. The
    /// operator chooses the sites; the values pass host → desk → container and
    /// never touch the tape, the model, or a log. Desk seat only, and there is
    /// no agent tool that does this — the agent can never pull cookies itself.
    #[serde(rename = "computer.cookies.import")]
    ComputerCookiesImport {
        persona_id: String,
        browser_id: String,
        profile_id: String,
        domains: Vec<String>,
    },
    /// What has been brought over to this teammate's computer, by browser
    /// and profile, with the sites: the record the pane lists. Desk seat
    /// only.
    #[serde(rename = "computer.cookies.list")]
    ComputerCookiesList { persona_id: String },
    /// Takes brought-over cookies back out of the teammate's computer: one
    /// site of an import when `domain` is given, the whole import when not.
    /// The computer's browser drops them at once, and the record answers as
    /// it stands afterwards. Starts the computer if it is stopped, the same
    /// as the import did. Desk seat only.
    #[serde(rename = "computer.cookies.forget")]
    ComputerCookiesForget {
        persona_id: String,
        browser_id: String,
        profile_id: String,
        #[serde(default)]
        domain: Option<String>,
    },
    /// The secrets the operator has stored for teammates: names and when
    /// each changed, never a value. Desk seat only.
    #[serde(rename = "secrets.list")]
    SecretsList {},
    /// Stores a secret under `name`, or replaces the one there, and hands the
    /// new value to every running computer granted that name. The value goes
    /// to the OS credential store and is never answered back — not by this
    /// command, not by a list, not by a subscription. Desk seat only.
    #[serde(rename = "secrets.set")]
    SecretsSet { name: String, value: String },
    /// Takes a stored secret away, and out of every running computer it was
    /// granted to. Desk seat only.
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
    /// teammate to. Desk seat only.
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
    /// `stored`. Desk seat only.
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
    /// Ends an arming without a passkey. Desk seat only.
    #[serde(rename = "secrets.passkey.cancel")]
    SecretsPasskeyCancel { persona_id: String },
}

/// What a subscription is a subscription to: a stream, or a view the core
/// maintains and nobody logs.
///
/// Externally tagged, so a stream reads as the word or the pair naming it —
/// `"room"`, `{"tape": "<personaId>"}`, `{"thread": "<key>"}`, `{"run":
/// "<runId>"}`, `{"view": "roster"}`, `{"schedules": "<personaId>"}` — which
/// is the shape the window would have written by hand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum Target {
    Room,
    Tape(String),
    Thread(String),
    /// One subagent run's own transcript, by run id.
    Run(String),
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
    pub session: SessionInfo,
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

        let thread = search::search(&root, "ada", "harbour", None);
        let thread_hits = thread["hits"].as_array().unwrap();
        assert_eq!(thread_hits.len(), 1);
        for hit in thread_hits {
            assert_eq!(&round_trip::<ThreadSearchHit>(hit), hit);
        }

        let everyone = search::search_all(&root, "harbour", None);
        let global_hits = everyone["hits"].as_array().unwrap();
        assert_eq!(global_hits.len(), 3);
        for hit in global_hits {
            assert_eq!(&round_trip::<GlobalSearchHit>(hit), hit);
        }
    }
}
