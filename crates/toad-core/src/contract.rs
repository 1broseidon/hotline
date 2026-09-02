//! The room's contract types: what the window is handed when it asks about a
//! teammate, and what the room pushes at it unasked.
//!
//! These are defined once, here, and the TypeScript is generated from them:
//! `cargo test -p toad-core` runs the tests `#[ts(export)]` writes and leaves
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

/// A Toad teammate. Four axes make up an identity:
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
    /// How far Toad Agent's tools reach. Absent means the working directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reach: Option<Reach>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode_id: Option<String>,
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
    /// Absent means inherit the desk's web search entirely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search_policy: Option<WebSearchPolicy>,
    /// This teammate's computer: a containerized desktop it drives through MCP
    /// tools. Deliberately not part of `mcpPolicy` — the computer is a
    /// per-teammate capability Toad manages, not one of the app's
    /// user-configured servers. Absent means off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computer: Option<PersonaComputer>,
    /// Subagents this teammate may send work to. Scoped here, not app-wide:
    /// one teammate's reviewer is not another's. Absent means the built-in
    /// task runner only, with no extras and no model pin.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagents: Option<PersonaSubagents>,
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

/// Which of the global MCP servers a teammate gets.
///
/// A capability is a property of the teammate, not of the app: the one that
/// files tickets should not also be able to deploy just because both servers
/// are configured. `all` is the default because the common case is a roster
/// that shares its tools, and `some` exists for when it should not.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct McpPolicy {
    pub mode: PolicyMode,
    pub server_ids: Vec<String>,
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
/// Only what the user decides lives here. Everything Toad derives — the bearer
/// token, container state, last activity — is process state, not config.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct PersonaComputer {
    pub enabled: bool,
    /// Image override. Defaults to the app's version-pinned image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// Operator-configured extras plus an optional pin on the built-in task
/// runner.
///
/// `generic` is reserved: it is always present, cannot be deleted, and is what
/// `subagent` runs when `kind` is omitted. Extras are additional kinds the
/// parent may choose, each with its own brief and optional model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct PersonaSubagents {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defaults: Option<SubagentDefaults>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extras: Option<Vec<SubagentSpec>>,
}

/// Overrides for the built-in task runner (`kind: generic`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SubagentDefaults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Extra briefing appended to the silent-runner prompt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Optional model as provider/id. Absent means inherit the teammate's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

/// An extra subagent the parent can pass as `kind`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SubagentSpec {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
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
    pub computer: Option<PersonaComputer>,
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
    /// What the user called it, so a list of keys is a list they recognise.
    pub label: String,
    /// Revoked. Set once and never unset — revocation is a fact, not a toggle.
    pub revoked: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What a credential's secret is. One word today, because every provider Toad
/// talks to takes an API key; the subscription logins that do not are a later
/// phase, and they arrive as a second word here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum CredentialKind {
    ApiKey,
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
    /// Whether Toad's transcript is showing history the agent no longer
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SessionConfig {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_id: Option<String>,
    pub options: Vec<ConfigChoice>,
}

/// What the agent behind a session can be asked to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SessionCapabilities {
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
/// at startup. Note this is Toad's own record: replaying it is not the same as
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
pub enum HumanActionStatus {
    Pending,
    Done,
    Dismissed,
    Expired,
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
/// going on. Only messages count — a teammate's last tool call is Toad's
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
}

// ---------------------------------------------------------------------------
// Backends
// ---------------------------------------------------------------------------

/// One agent harness as the new-teammate sheet offers it: Toad Agent, or an
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

// ---------------------------------------------------------------------------
// The wire
// ---------------------------------------------------------------------------

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
    #[serde(rename = "credential.revoke")]
    CredentialRevoke { id: String },
    #[serde(rename = "credential.delete")]
    CredentialDelete { id: String },
    /// Every agent harness this machine can start, and the ones it knows of
    /// but cannot, with the reason.
    #[serde(rename = "backends.list")]
    BackendsList {},
    /// Every credential the room knows of, never a secret.
    #[serde(rename = "credential.list")]
    CredentialList {},
    /// Every model the desk's keys can reach, grouped by provider. An empty
    /// struct rather than a unit so `"params": {}` — what a client that
    /// always sends params spells it — reads the same as no params at all.
    #[serde(rename = "models.list")]
    ModelsList {},
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
    /// Copies an existing Toad data directory into this room. The source is
    /// never written.
    #[serde(rename = "room.import")]
    RoomImport { from: String },
    /// Closes the teammate's open chapter now, and answers with the chapter it
    /// closed — its title and its note, which the summariser has written by
    /// the time this returns.
    #[serde(rename = "chapter.start_fresh")]
    ChapterStartFresh { persona_id: String },
}

/// What a subscription is a subscription to: a stream, or a view the core
/// maintains and nobody logs.
///
/// Externally tagged, so a stream reads as the word or the pair naming it —
/// `"room"`, `{"tape": "<personaId>"}`, `{"thread": "<key>"}`, `{"view":
/// "roster"}` — which is the shape the window would have written by hand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum Target {
    Room,
    Tape(String),
    Thread(String),
    View(ViewName),
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
        let dir =
            std::env::temp_dir().join(format!("toad-core-contract-{name}-{}", std::process::id()));
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
                "scheduled": { "jobId": "j1", "kind": "loop", "name": "daily", "quiet": true },
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
                "kind": "chapter", "id": "c1", "ts": 14, "backendId": "pi", "sessionId": "s1",
                "endedAt": 20, "title": "First", "note": "did the thing", "status": "done",
                "tags": ["harbour"], "closedBy": "idle", "resumedFrom": "c0",
            }),
            json!({ "kind": "chapter", "id": "c2", "ts": 21, "backendId": "pi" }),
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
