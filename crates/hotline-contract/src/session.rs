//! Sessions, their configuration and the model catalog; the backends a
//! teammate can run on; skills.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::*;

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
    /// The turn in flight is open only for subagents it started: the agent
    /// has said its reply and is doing nothing else, so the turn reads as
    /// done while they work. Only ever set while `state` is `thinking`.
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub awaiting_subagents: bool,
    /// Commands the teammate left running past the turns that started them:
    /// a server, a watcher. They do not change `state`: a teammate with a
    /// server up and nothing to do reads idle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<SessionProcess>>", optional)]
    pub processes: Vec<SessionProcess>,
    /// Which account the session runs under and which it will move to.
    /// Absent for a teammate on the default with no switch pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<SessionAccount>,
}

/// One command a session holds running, as the desk's chip shows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SessionProcess {
    /// The job id the teammate knows it by, and `process.stop` takes.
    pub id: String,
    /// The command's first line, clipped.
    pub title: String,
    /// Milliseconds since the epoch.
    pub started_at: i64,
    pub state: ProcessState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ProcessState {
    Running,
    /// Asked to stop, and not yet gone.
    Stopping,
}

/// What `process.output` answers: the whole command, and the latest of what
/// it has written, stdout and stderr together.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct ProcessOutput {
    pub command: String,
    pub tail: String,
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
    /// The harness takes ACP's `additionalDirectories`, so a teammate's extra
    /// folders are handed to it when a session opens. False means the
    /// harness was not told of them; its own reach is still its own.
    #[serde(default)]
    pub additional_directories: bool,
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
/// the harnesses this machine can start, its default backend and its roster,
/// tombstones included — never from a stored "seen" flag: the pane is on
/// screen exactly while no teammate has ever been made here.
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
    /// A teammate has been made in this room, whether or not one is left.
    /// A room that is set up and empty opens on New teammate, not on the
    /// welcome: it has been through setup once already.
    pub set_up: bool,
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
