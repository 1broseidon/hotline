//! Teammates: their pictures, reach, tool policies and ledger, web search, and the
//! draft the new-teammate form fills in.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use super::*;

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
    /// Folders besides the workspace this teammate may read, and change where
    /// a grant is `writable`. Absent means none, including on every record
    /// from before the field: no older reach or mount is translated into a
    /// grant. Set only by the person through `persona.update` or
    /// `persona.create`, which check each path (see [`FolderGrant`]).
    /// Under workspace reach Hotline Agent's file tools and confined shell
    /// enforce it; Hotline's ACP file callbacks honour it for a harness; a
    /// computer mounts it; whole-machine reach makes it moot but keeps it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folders: Option<Vec<FolderGrant>>,
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
    /// On a CLI agent an agent account, on Hotline Agent a credential of its
    /// model's provider, and absent is the default; an id that belongs to
    /// another backend or provider stays stored and is ignored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
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
    /// The agent account the session ran under. Absent means This computer,
    /// so a checkpoint from before accounts still belongs to a teammate that
    /// has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub account_id: Option<String>,
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

/// One folder a teammate may reach besides its workspace.
///
/// The path is stored as it was checked when granted: absolute, an existing
/// directory, canonical (so no symlink in it points somewhere else), not the
/// workspace or inside it, not inside another granted folder, and not `/`,
/// the home directory or anything holding it, or Hotline's data directory
/// or anything holding it or inside it, another teammate's workspace there
/// included.
/// A folder that later stops being that same directory is left out when a
/// session opens it rather than followed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts")]
pub struct FolderGrant {
    pub path: String,
    /// Whether the teammate may create, change and delete files in it.
    /// Absent is false: a grant is read-only unless the person said more.
    #[serde(default)]
    pub writable: bool,
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
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[ts(export, export_to = "contract.ts")]
pub struct WebSearchPolicy {
    pub mode: PolicyMode,
    pub providers: Vec<WebSearchProvider>,
}

impl<'de> Deserialize<'de> for WebSearchPolicy {
    /// Reads the providers it knows. One this build does not have is left out
    /// rather than taking the whole teammate with it.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            mode: PolicyMode,
            providers: Vec<Value>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            mode: raw.mode,
            providers: raw
                .providers
                .into_iter()
                .filter_map(|value| serde_json::from_value(value).ok())
                .collect(),
        })
    }
}

/// The four providers web search has. The chain tries them Parallel, Exa,
/// Keenable, Firecrawl before any key promotes one. A record may name one
/// this build does not have; it is left out on read rather than taking the
/// whole teammate with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum WebSearchProvider {
    Parallel,
    Exa,
    Firecrawl,
    Keenable,
}

/// One provider as the Tools pane shows it: whether the desk has it on, and
/// whether a key is saved for it. Never the key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct WebSearchProviderStatus {
    pub provider: WebSearchProvider,
    pub name: String,
    pub enabled: bool,
    pub has_key: bool,
}

/// What `websearch.status` answers: the providers in the order a search with
/// no keys tries them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct WebSearchStatus {
    pub providers: Vec<WebSearchProviderStatus>,
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
    /// Extra folders from the start, checked as `persona.update` checks
    /// them. Absent or empty is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folders: Option<Vec<FolderGrant>>,
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
    /// On a CLI agent an agent account, on Hotline Agent a credential of its
    /// model's provider, and absent is the default; an id that belongs to
    /// another backend or provider stays stored and is ignored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}
