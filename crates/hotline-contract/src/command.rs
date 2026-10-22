//! `Command`: everything a client may ask the room.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

use super::*;

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
        thread: Option<ThreadId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        persona_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stream_audio: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_mode: Option<VoiceInputMode>,
        /// Omitted: use Settings. `front` requires live; `pipeline` is the
        /// person's explicit Legacy choice. A failed live call never changes mode.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<VoiceCallMode>,
        /// The rate of the microphone audio a front call will send:
        /// 16000 or 24000. Omitted, the front's own rate.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_rate: Option<u32>,
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
    /// Front call: microphone PCM16 mono at the negotiated input rate, at most
    /// 32 KiB, sequence increasing. A desk that is behind refuses the frame;
    /// drop it rather than queue it.
    #[serde(rename = "voice.front_audio")]
    VoiceFrontAudio {
        call_id: String,
        seq: u32,
        data: String,
    },
    /// Front call: the person spoke over the front. `playedMs` is how much of
    /// the front's current reply the speaker had played, which is all the
    /// front keeps of it.
    #[serde(rename = "voice.front_barge")]
    VoiceFrontBarge { call_id: String, played_ms: u32 },
    #[serde(rename = "voice.utterance")]
    VoiceUtterance {
        call_id: String,
        seq: u32,
        mime_type: String,
        data: String,
        duration_ms: u32,
    },
    /// Local Legacy capture owns the floor before a finalized clip or text arrives.
    #[serde(rename = "voice.input_state")]
    VoiceInputState { call_id: String, speaking: bool },
    #[serde(rename = "voice.interrupt")]
    VoiceInterrupt { call_id: String },
    #[serde(rename = "voice.hold")]
    VoiceHold { call_id: String, hold: bool },
    #[serde(rename = "voice.call_end")]
    VoiceCallEnd { call_id: String },
    /// The desk's own speech models and where each stands. Owner only, like
    /// the rest of `voice.*`; a download in progress is polled here.
    #[serde(rename = "voice.models")]
    VoiceModels {},
    /// Starts downloading a model. The download is checked against the hash
    /// Hotline pins for it before anything of it is unpacked.
    #[serde(rename = "voice.model_install")]
    VoiceModelInstall { model_id: String },
    #[serde(rename = "voice.model_cancel")]
    VoiceModelCancel { model_id: String },
    #[serde(rename = "voice.model_remove")]
    VoiceModelRemove { model_id: String },
    /// One clip heard by the desk's own model, outside any call: a 16 kHz
    /// mono PCM16 WAV or AAC in MP4, at most two minutes.
    #[serde(rename = "voice.transcribe")]
    VoiceTranscribe { mime_type: String, data: String },
    /// Listener and pairing controls require an owner or the local desk.
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
    /// The servers this computer is paired with only as a relay, each with
    /// what it last said it runs. They are chosen with `remote.relay`.
    #[serde(rename = "remote.relays")]
    RemoteRelays {},
    /// Sent by a paired device to a served desk: move the caller's own owner
    /// grant to a relay grant (ADR 0006 § Migration). It names no device;
    /// the grant is the one the caller's handshake authenticated.
    #[serde(rename = "remote.become_relay")]
    RemoteBecomeRelay {},
    /// Keep the https address a tunnel or proxy gives this desk, for phones
    /// away from home, or clear it with none or a blank one.
    #[serde(rename = "remote.public_url")]
    RemotePublicUrl {
        #[serde(default)]
        url: Option<String>,
    },
    /// Start a sealed invitation, or poll/cancel its id.
    #[serde(rename = "remote.pairing")]
    RemotePairing {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<DeviceRole>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default)]
        cancel: bool,
    },
    /// The desks this one is linked with (ADR 0003): how each link is doing
    /// and that desk's teammates. Owner or local desk only.
    #[serde(rename = "desks.links")]
    DesksLinks {},
    /// Links this desk with a desk it is paired with, so their teammates can
    /// see and message each other. The other desk records the link over this
    /// computer's owner seat there, and this one dials it from then on.
    #[serde(rename = "desks.link")]
    DesksLink { desk_id: String },
    /// The other half of `desks.link`, asked of the desk being linked with
    /// over the linking computer's owner seat: trust `key`, that desk's own
    /// identity, for `/v2/link`. Answers this desk's id and name.
    #[serde(rename = "desks.link_accept")]
    DesksLinkAccept {
        desk_id: String,
        name: String,
        key: String,
    },
    /// Ends a link on both desks: now here, and on the other desk as soon as
    /// it hears. Messages waiting for it fail with a sentence.
    #[serde(rename = "desks.unlink")]
    DesksUnlink { desk_id: String },
    /// Federation: a linked desk's teammates, as it publishes them. Only a
    /// linked desk's seat sends the `federation.*` commands, and they are
    /// all it may send.
    #[serde(rename = "federation.roster")]
    FederationRoster { teammates: Vec<LinkedTeammate> },
    /// Federation: a message for one of this desk's teammates, or the answer
    /// to one of theirs, for the exchange queue.
    #[serde(rename = "federation.deliver")]
    FederationDeliver { delivery: LinkedDelivery },
    /// Federation: the other desk ended the link.
    #[serde(rename = "federation.unlink")]
    FederationUnlink {},
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
        /// A picture at most this many px on its longer side, as a JPEG the
        /// desk keeps, rather than the file itself: what a phone draws in
        /// the conversation. Only a picture has one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<u32>,
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
        /// The picture at most this many px wide, still a PNG, rather than
        /// as kept: what a list row or a bubble's face needs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<u32>,
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
    /// Makes `id` the desk's lead, moving the role from whoever had it, or
    /// clears the lead when `id` is absent. A desk has at most one. A desk
    /// of one teammate always has that one as lead, so clearing it there is
    /// refused. Owners only, as `persona.pin`.
    #[serde(rename = "persona.lead")]
    PersonaLead {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// One key at a time, `null` clearing a key back to its default.
    #[serde(rename = "settings.update")]
    SettingsUpdate {
        #[ts(type = "Record<string, unknown>")]
        patch: Map<String, Value>,
    },
    #[serde(rename = "images.status")]
    ImagesStatus {},
    /// Each web search provider: whether the desk has it on and whether a key
    /// is saved for it, never the key. Owner or local desk only.
    #[serde(rename = "websearch.status")]
    WebsearchStatus {},
    /// Switches one provider on or off for the whole desk. Off stays off for
    /// every teammate. Answers the status. Owner or local desk only.
    #[serde(rename = "websearch.set_enabled")]
    WebsearchSetEnabled {
        provider: WebSearchProvider,
        enabled: bool,
    },
    /// Saves a provider's API key in the vault, or forgets it when `key` is
    /// absent or blank. A key moves its provider ahead of the keyless ones.
    /// The key is never answered back. Owner or local desk only.
    #[serde(rename = "websearch.set_key")]
    WebsearchSetKey {
        provider: WebSearchProvider,
        #[serde(default)]
        key: Option<String>,
    },
    /// The card for a link written in a message, read by the desk; `null`
    /// when the page has none or is not one the desk will read.
    #[serde(rename = "link.preview")]
    LinkPreview { url: String },
    /// Nothing but proof the link is alive both ways: answered `null`. A
    /// phone that named `lean` sends one every so often, and a lean socket
    /// that has heard nothing at all for the wire's idle limit is
    /// closed as gone.
    #[serde(rename = "ping")]
    Ping {},
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
    /// Runs the teammate's last turn again when it failed, without writing
    /// the person's message a second time.
    #[serde(rename = "session.retry")]
    SessionRetry { persona_id: String },
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
    /// Stops one command the teammate left running. The teammate is not
    /// woken by its end: the person stopped it.
    #[serde(rename = "process.stop")]
    ProcessStop { persona_id: String, id: String },
    /// The whole command and the latest of its output, as `ProcessOutput`.
    #[serde(rename = "process.output")]
    ProcessOutput { persona_id: String, id: String },
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
    /// `reminder` makes it a plain reminder: the prompt is posted to the chat
    /// when it fires, and no work is done.
    #[serde(rename = "schedule.create")]
    ScheduleCreate {
        persona_id: String,
        kind: ScheduleKind,
        when: Option<i64>,
        every: Option<i64>,
        prompt: String,
        quiet: Option<bool>,
        #[serde(default)]
        reminder: Option<bool>,
    },
    #[serde(rename = "schedule.list")]
    ScheduleList {},
    /// Ends a living job, with a final post saying where it stood, or takes
    /// an ended one off the list. Its thread stays readable either way.
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
    /// Makes the computer again with the memory and CPU limits the teammate
    /// has now, between turns if it is running. The volumes survive.
    #[serde(rename = "computer.restart")]
    ComputerRestart { persona_id: String },
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
    /// The single owner/phone door for a live credential card.
    #[serde(rename = "secrets.capture.answer")]
    SecretsCaptureAnswer(CredentialAnswer),
    /// Changes only a login or passkey's use policy; desk/owner only.
    #[serde(rename = "secrets.use.set")]
    SecretsUseSet { name: String, r#use: SecretUse },
    /// Answers one live computer use; desk, owner and phone only.
    #[serde(rename = "secrets.use.answer")]
    SecretsUseAnswer {
        persona_id: String,
        use_id: String,
        answer: SecretUseAnswer,
    },
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        r#use: Option<SecretUse>,
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
    /// brought back first. The person speaks in the main conversation, in a
    /// work thread and in a job's thread, until the job has ended; a pair, a
    /// run and a call are not spoken in.
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

    #[expect(
        clippy::ref_option,
        reason = "serde's serialize_with hands over the field by reference"
    )]
    pub fn serialize<S, T>(value: &Option<Option<T>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        value.serialize(serializer)
    }
}
