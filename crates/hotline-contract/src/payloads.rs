//! Spending and images, linked desks, paired devices, and the payloads the
//! wire carries: mobile chunks, voice and capabilities.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ---------------------------------------------------------------------------
// Spending and images
// ---------------------------------------------------------------------------

/// One budget's limits. An absent limit is no limit; zero turns that
/// budget's paid use off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct BudgetLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub month_usd: Option<f64>,
}

impl BudgetLimits {
    pub fn new(day_usd: Option<f64>, month_usd: Option<f64>) -> Self {
        Self { day_usd, month_usd }
    }

    pub fn validate(&self) -> Result<(), String> {
        if [self.day_usd, self.month_usd]
            .into_iter()
            .flatten()
            .all(valid_amount)
        {
            Ok(())
        } else {
            Err("Spending limits must be finite, nonnegative dollar amounts.".into())
        }
    }

    /// Whether either limit is zero, which turns the budget's paid use off.
    pub fn off(&self) -> bool {
        self.day_usd == Some(0.0) || self.month_usd == Some(0.0)
    }
}

impl SpendingSettings {
    pub fn validate(&self) -> Result<(), String> {
        self.chat.validate()?;
        self.voice.validate()?;
        self.images.validate()
    }
}

/// Whether a dollar amount can be a limit or a spend: finite and not negative.
pub fn valid_amount(amount: f64) -> bool {
    amount.is_finite() && amount >= 0.0
}

/// The room's `spending` setting: a budget each for chat (teammates and the
/// call assistant), voice (transcription and speech) and images, each with
/// optional daily and monthly limits. A room that set none has no limits.
///
/// ```json
/// {"chat": {"dayUsd": 5, "monthUsd": 50}, "voice": {"dayUsd": 10}, "images": {}}
/// ```
///
/// The shared limits earlier versions wrote, `{"dayUsd": 10, "monthUsd": 20}`,
/// still read: as the voice and the images limits, with chat unlimited,
/// since those were the two they covered. Writes are always the new shape.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SpendingSettings {
    pub chat: BudgetLimits,
    pub voice: BudgetLimits,
    pub images: BudgetLimits,
}

impl<'de> Deserialize<'de> for SpendingSettings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Stored {
            #[serde(default)]
            chat: Option<BudgetLimits>,
            #[serde(default)]
            voice: Option<BudgetLimits>,
            #[serde(default)]
            images: Option<BudgetLimits>,
            #[serde(default)]
            day_usd: Option<f64>,
            #[serde(default)]
            month_usd: Option<f64>,
        }
        let stored = Stored::deserialize(deserializer)?;
        let budgets = stored.chat.is_some() || stored.voice.is_some() || stored.images.is_some();
        let shared = stored.day_usd.is_some() || stored.month_usd.is_some();
        if budgets && shared {
            return Err(serde::de::Error::custom(
                "spending mixes shared limits with per-budget ones",
            ));
        }
        if shared {
            let limits = BudgetLimits::new(stored.day_usd, stored.month_usd);
            return Ok(Self {
                chat: BudgetLimits::default(),
                voice: limits,
                images: limits,
            });
        }
        Ok(Self {
            chat: stored.chat.unwrap_or_default(),
            voice: stored.voice.unwrap_or_default(),
            images: stored.images.unwrap_or_default(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SpendingSummary {
    pub day_usd: f64,
    pub month_usd: f64,
}

/// The owner's choice, from `settings.images`. Both keys are optional: none
/// means the first connected provider that can draw, on its default model.
/// The provider `off` (see [`OFF`]) means no images at all.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ImageSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl ImageSettings {
    /// The `settings.images` provider that turns image generation off. Stored
    /// as a provider so a desk that never heard of it still reads a string
    /// there, and so the picker's one choice is the setting's one key.
    pub const OFF: &str = "off";

    pub fn is_off(&self) -> bool {
        self.provider.as_deref() == Some(Self::OFF)
    }
}

// ---------------------------------------------------------------------------
// Linked desks
// ---------------------------------------------------------------------------

/// What a linked desk publishes about one of its teammates: exactly what
/// `list_teammates` shows of a local one, and nothing more. No workspace
/// path, tool list, server address, message or secret has a field here to
/// travel in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct LinkedTeammate {
    pub persona_id: String,
    pub name: String,
    /// idle, working, waiting or stopped; offline while its desk is away.
    pub state: String,
    /// The tool or task it is doing right now, while it is working.
    #[serde(default)]
    pub activity: Option<String>,
    pub working_on: WorkingOn,
    #[serde(flatten)]
    pub profile: Profile,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct WorkingOn {
    pub title: String,
    #[serde(default)]
    pub last_turn_at: Option<String>,
}

/// Who a teammate is and what it can do here, as its colleagues see it:
/// the goal the person gave it, the agent and model it runs, and its
/// capabilities as words and switches. Every field defaults, because a desk
/// from before these were published sends none of them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct Profile {
    #[serde(default)]
    pub goal: String,
    /// The agent behind it, by the name a person knows: Hotline Agent,
    /// Claude Code, Codex.
    #[serde(default)]
    pub agent: String,
    /// The model id, when the teammate or its session names one.
    #[serde(default)]
    pub model: Option<String>,
    /// The model's effort, when one is set.
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub capabilities: Capabilities,
}

/// What a teammate can do here, never how: no path, address or tool list.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct Capabilities {
    /// none, attached, or in use (its session is running with it now).
    #[serde(default)]
    pub computer: String,
    #[serde(default)]
    pub web_search: bool,
    #[serde(default)]
    pub image_generation: bool,
    /// Whether it may schedule its own wake-ups.
    #[serde(default)]
    pub background_work: bool,
    /// How it messages a colleague: directly, or after the person's
    /// approval on first contact.
    #[serde(default)]
    pub message_teammates: String,
    /// workspace or whole machine.
    #[serde(default)]
    pub reach: String,
    /// The names of the MCP servers it is granted.
    #[serde(default)]
    pub mcp_servers: Vec<String>,
}

/// What crosses a link for the exchange queue: a teammate's message for one
/// of the other desk's, the answer to one, or a report from the work thread
/// one opened.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts")]
pub enum LinkedDelivery {
    /// `fromPersonaId` is the sender on its own desk, which has published it
    /// on its roster; `to` is the recipient's id on the desk receiving this.
    /// The request id is the sender desk's, and the same id sent again is the
    /// same request.
    Request {
        request_id: String,
        from_persona_id: String,
        to: String,
        message: String,
        /// ask or handoff.
        intent: String,
        /// For a handoff into the work thread an earlier one opened, or an
        /// ask answered in it: the request id of that earlier handoff. An
        /// older desk opens a new thread for a handoff instead, and answers
        /// an ask without the thread or refuses it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        continues: Option<String>,
        /// For an ask from a work thread that a handoff of the receiving
        /// desk's teammate opened: that handoff's request id, which the
        /// receiving desk looks up in its own record.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        about: Option<String>,
    },
    /// The answer to a request this desk sent, by its id.
    Result {
        request_id: String,
        reply: String,
        failed: bool,
        /// The person on the desk that worked it stopped it, or interrupted
        /// its answer, and `reply` is what its sender is told of that: heard
        /// as their decision, without a wake, never handed back as a failure
        /// to retry. An older desk ignores it and delivers the failure.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[ts(as = "Option<bool>", optional)]
        stopped_by_person: bool,
    },
    /// A turn report from the work thread a request of this desk's opened
    /// there: the teammate's own words, never the person's.
    Progress {
        request_id: String,
        text: String,
        wakes: bool,
    },
}

/// How a link is doing, as the Desks section shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum DeskLinkState {
    /// The channel is open; messages cross now.
    Connected,
    /// Not open: the other desk is away, or this one is still reaching it.
    /// Messages wait in the exchange queue meanwhile.
    Offline,
}

/// One linked desk, for `desks.links`.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct DeskLink {
    pub desk_id: String,
    pub name: String,
    /// Whether this desk made the link and dials the other.
    pub dials: bool,
    pub state: DeskLinkState,
    /// Why the channel is not open, when this desk knows.
    #[ts(optional)]
    pub error: Option<String>,
    /// The other desk's teammates, live while connected and as last heard
    /// of while not.
    pub teammates: Vec<LinkedTeammate>,
}

// ---------------------------------------------------------------------------
// Paired devices
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum DeviceRole {
    /// Keyed grants with missing roles retain the full owner command set.
    #[default]
    Owner,
    Companion,
    /// May stand in on this server's relay and carry visits, and nothing
    /// else: no seat on the wire, the computer or the link (ADR 0006).
    Relay,
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

/// A link's card: the page's title, a second line saying where it is or
/// what it is about, and its picture when it has one over https.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct LinkPreview {
    pub url: String,
    pub title: String,
    pub site: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
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
    /// The Voice budget's daily limit; absent when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub day_usd: Option<f64>,
    /// The Voice budget's monthly limit; absent when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub month_usd: Option<f64>,
    /// What transcription and speech have spent today.
    pub spent_day_usd: f64,
    pub spent_month_usd: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct VoiceStatus {
    /// The selected model is live, even when its login or budget is unavailable.
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub live_selected: bool,
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
    /// How teammates' replies on calls to them were said, one entry per
    /// model that wrote any, by model. Absent before the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replies: Option<Vec<VoiceReplies>>,
    /// The voice front calls run on, when the desk can open one. A
    /// teammate's own voice is picked from its voices. Absent on a desk with
    /// no front, or one older than the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front: Option<VoiceFrontModel>,
}

/// A voice front as the desk would open it: which one (`grok`, `openai`,
/// `gpt-live`), its model, the voice calls speak in when a teammate has none
/// of its own, and every voice it speaks in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct VoiceFrontModel {
    pub provider_id: String,
    pub model_id: String,
    pub voice: String,
    pub voices: Vec<String>,
}

/// How one model's replies on calls to teammates were said: each reply is
/// counted once, by the way it was written. Read-only, kept by the desk
/// across restarts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct VoiceReplies {
    /// The teammate's agent and model: `hotline/<provider>/<model>` for
    /// Hotline Agent, `acp/<adapter>` for an ACP agent, followed by
    /// `/<model>` when its session names one.
    pub model: String,
    /// A spoken version and a written one.
    #[serde(default)]
    pub both: u32,
    /// A spoken version and no written one.
    #[serde(default)]
    pub spoken_only: u32,
    /// A spoken version never closed with `</spoken>`.
    #[serde(default)]
    pub unclosed: u32,
    /// No spoken version, and the call said the reply's opening: there was
    /// no call assistant to rewrite it, or it failed or took too long.
    #[serde(default)]
    pub untagged: u32,
    /// No spoken version, so the call assistant rewrote the reply to be
    /// heard, and that was said.
    #[serde(default)]
    pub rewritten: u32,
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

/// The spending budgets and what has been spent against each so far.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct CapabilitySpending {
    /// Always chat, voice and images, in that order.
    pub budgets: Vec<SpendingBudget>,
    /// Set when a tally could not be read, so the spent figures are not whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

/// One budget: its limits, what it has spent, and what that went on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SpendingBudget {
    pub kind: BudgetKind,
    /// `null` is no limit. Zero turns this budget's paid use off.
    pub day_usd: Option<f64>,
    /// `null` is no limit. Zero turns this budget's paid use off.
    pub month_usd: Option<f64>,
    pub spent_day_usd: f64,
    pub spent_month_usd: f64,
    /// Chat: teammates, then the call assistant. Voice: transcription, then
    /// speech. Images: none.
    pub lines: Vec<SpendingLine>,
}

/// What paid use is budgeted under: teammates and the call assistant are
/// chat, hearing and speaking on calls are voice, and drawing is images.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub enum BudgetKind {
    Chat,
    Voice,
    Images,
}

impl BudgetKind {
    /// The budget's name as Settings › Budgets shows it.
    pub fn name(self) -> &'static str {
        match self {
            BudgetKind::Chat => "Chat",
            BudgetKind::Voice => "Voice",
            BudgetKind::Images => "Images",
        }
    }
}

/// One kind of paid use within a budget and what it has cost today and this
/// month.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SpendingLine {
    pub kind: SpendingKind,
    pub day_usd: f64,
    pub month_usd: f64,
}

/// Paid use as Budgets names it: teammates' turns and the call assistant
/// (chat), transcription and speech (voice).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub enum SpendingKind {
    Teammates,
    CallAssistant,
    Transcription,
    Speech,
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

/// Where one of the desk's own speech models stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SpeechModelState {
    Available,
    Downloading,
    Unpacking,
    Installed,
}

/// A model the desk can turn speech into text with on its own machine, which
/// the owner downloads once. Nothing is installed until they ask.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SpeechModel {
    pub id: String,
    pub name: String,
    /// A word or two shown after the name to choose by ("fast", "English only").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// What it hears and how it trades accuracy for speed, in a few words.
    pub detail: String,
    pub download_bytes: u64,
    /// What it takes on disk once installed.
    pub disk_bytes: u64,
    /// The model's maker and licence, as the licence asks to be credited.
    pub credit: String,
    pub licence_url: String,
    pub state: SpeechModelState,
    /// How much has arrived, while it downloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received_bytes: Option<u64>,
    /// Why the last download failed, until the next one starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The id of a model in this list that hears more languages, which
    /// Settings suggests to someone whose window is not in English while
    /// this one is the only one installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub more_languages: Option<String>,
}

/// What the desk's own model heard in one clip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct VoiceTranscript {
    pub text: String,
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
    /// Present only on a front call: the desk's answer to `mode: "front"`.
    /// Absent means the call runs on the transcribe, answer, speak pipeline,
    /// as every desk before the front did, so a client falls back on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub front: Option<VoiceFront>,
}

/// How a front call carries sound. Both directions are mono PCM16
/// little-endian in `voice.front_audio` and `frontAudio` events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct VoiceFront {
    /// The rate the desk takes microphone audio at: the `inputRate` asked for
    /// when it is one the desk resamples, else the front's own rate.
    pub input_rate: u32,
    /// The rate of the audio the desk sends, which the speaker plays at.
    pub output_rate: u32,
}

/// What answers the person on a call. Omission follows Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum VoiceCallMode {
    #[default]
    Selected,
    /// An explicit choice to use Legacy voice for this call.
    Pipeline,
    Front,
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
        /// A live call failed; a new Legacy call requires the person to choose it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        live_error: Option<String>,
        /// True only while recovering a dropped transport or live provider.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[ts(as = "Option<bool>", optional)]
        reconnecting: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<VoiceEndReason>,
        /// On `thinking`: the teammate's turn is still working and the call
        /// takes what the person says, which steers into that turn. A client
        /// keeps its microphone open, as it does on `listening`, unless it is
        /// playing speech. Absent, `thinking` means the desk is still taking
        /// the last thing said and has the floor, as from desks before it.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[ts(as = "Option<bool>", optional)]
        listening: bool,
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
        /// Remove a card settled at the desk or on the call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved: Option<bool>,
    },
    /// Front call: a piece of the front's speech, at most 100 ms of PCM16 at
    /// the call's output rate. `seq` rises by one per frame; a gap means the
    /// desk dropped audio the client was too slow to take.
    #[serde(rename = "frontAudio")]
    FrontAudio {
        seq: u32,
        data: String,
    },
    /// Front call: the person has the floor. Stop playing what is queued and
    /// answer with `voice.front_barge`, the milliseconds actually played.
    #[serde(rename = "frontStop")]
    FrontStop {},
    /// Older desks used this to switch to Legacy. Clients must stop instead.
    #[serde(rename = "frontEnded")]
    FrontEnded {},
}
