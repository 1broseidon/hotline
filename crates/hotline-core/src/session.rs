//! The room's live sessions: one conversation per teammate, and the funnel
//! every word of it passes through.
//!
//! A [`Driver`] runs the turn and knows nothing else. Everything that makes a
//! turn part of a room happens here, in one place, for both kinds of agent:
//!
//! - The user's line is on the tape **before** the driver sees it. What was
//!   said is a fact the moment somebody said it, and a turn that fails must
//!   not lose the message that started it.
//! - Every driver update becomes the tape events it is, in the shapes the
//!   previous edition wrote — [`crate::contract::TranscriptEvent`] pins them, and
//!   a tape written here opens in that app unchanged. An agent's message is
//!   split into bubbles ([`pacing`]) here so both kinds of agent and a
//!   peer thread get the same ones.
//! - Every append is offered to the search index. The index is rebuildable, so
//!   a failure there is printed and swallowed; a failure to write the tape is
//!   the record, and is printed too because nothing above can undo it.
//! - Deltas go out on [`Room::subscribe_deltas`] and are never written; the
//!   durable line is the message that lands when it is whole.
//! - A job on the room stream fires when its `nextAt` arrives. That clock is
//!   [`schedule`]; each firing is a turn on the job's own thread
//!   ([`job_thread`]), never the DM, and what the job tells the person comes
//!   into the DM as a post.
//! - A message goes to an agent whose context is the chapter it is in. The
//!   room opens a chapter when a session starts, closes one that has gone
//!   quiet, and replaces the agent whose chapter closed before the next
//!   message reaches it. That is [`chapters`], and it lives here for the same
//!   reason: a chapter is a fact about the room, not about the agent.
//!
//! Drivers and tools capture revocable session authority. Changes to reach,
//! tools, the workspace, or the harness invalidate old handles and queued
//! work before a live session is rebuilt.

mod agent;
mod asking;
pub(crate) mod avatar;
mod call_cards;
mod call_files;
mod chapters;
mod computer;
mod dm;
mod driver_info;
pub(crate) mod exchanges;
pub(crate) mod files;
pub(crate) mod generate;
mod job_thread;
pub(crate) mod jobs;
pub(crate) use hotline_room::tool_ledger as ledger;
mod calls;
mod lifecycle;
mod links;
mod narration;
mod pacing;
mod peers;
mod person_name;
mod post;
mod preamble;
pub(crate) use post::{POST_CHARS, Post, SUBAGENT};
pub(crate) use preamble::preamble;
pub(crate) mod capture;
mod cookie_offers;
mod prompt;
pub(crate) mod runner;
pub(crate) mod schedule;
mod secret_uses;
mod secrets;
mod sides;
mod start;
pub(crate) use sides::preview_line;
mod stopping;
mod threads;
mod turns;
mod web_search;
use asking::HumanWait;
use chapters::sweep_idle_chapters;
use computer::{ComputerSetup, ComputerSwap};
use driver_info::follow_model_changes;
pub use hotline_room::provider_auth::ProviderAuth;
use prompt::{PendingTool, stamped};
use secrets::PasskeyArming;
pub use stopping::Stopped;

pub use peers::TEAMMATE_MESSAGE_MAX;
pub(crate) use schedule::{Brief, Ends, Report, Trigger};
pub use schedule::{parse_duration, parse_when};

use crate::computer::Computer;
use crate::contract::ThreadId;
use crate::contract::{
    Attachment, ChapterClose, Client, ComputerStatus, ConfigChoice, CookieSite, DeliveryCause,
    DeliveryFrom, HostBrowser, HumanActionStatus, HumanAnswer, NoticeLevel, PasskeyRegistration,
    PasskeyRegistrationState, Persona, Reach, Receipt, RuntimeReport, SessionCapabilities,
    SessionInfo, SessionState, SharedSecret, StreamDelta, TeammateToolLedger, ToolOutput,
    ToolSourceKind, ToolState, ToolStatus, TranscriptEvent,
};
use crate::driver::acp::{self, ChildAgent};
use crate::driver::rig;
use crate::driver::rig::{InProcess, Said};
use crate::driver::{
    CapabilityEpoch, CapabilityLease, Driver, DriverInfo, HOTLINE_BACKEND_ID, MessageKind, Update,
    clip,
};
use crate::log::{Log, StreamId, thread};
use crate::mcp;
use crate::mcp::server::TeammateTools;
use crate::room;
use crate::store::chapters as chapter_view;
use crate::store::search::Indexer;
use crate::vault::Vault;
use async_trait::async_trait;
use chrono::{Local, TimeZone};
use jobs::Processes;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;
use tokio::sync::{Mutex as TokioMutex, Notify, broadcast, mpsc, oneshot, watch};
use turns::Turns;

/// How much of a tool's output the transcript keeps. The model was given all
/// of it; this is the size of the bubble.
const TOOL_OUTPUT_CHARS: usize = 4_000;

/// How far behind a listener may fall before it starts missing news. Both
/// channels carry things a slow reader can recover from — a session's state is
/// re-askable, and a lost delta is made good by the message that follows it.
const BROADCAST_DEPTH: usize = 256;

/// How long a stamp left by a prompt may wait for the user line it belongs to.
///
/// A prompt and the line it puts on the tape are one motion here, so the wait
/// is nothing at all. The expiry is checked anyway, because the funnel must
/// not trust that the mark it finds was left for the line it is writing now:
/// in the previous edition a prompt could be refused between leaving the mark and
/// writing the line, and these fifteen seconds are what kept that stamp off
/// somebody's later, unrelated message.
const MARK_TTL_MS: i64 = 15_000;

/// How the idle clock runs. The first look is soon after the room opens,
/// because a chapter that went stale while Hotline was closed should be closed
/// before the person who closed it comes back; after that a minute is finer
/// than any setting the room allows, and the work is a fold nobody is waiting
/// on.
const FIRST_SWEEP: Duration = Duration::from_secs(5);
const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// While a turn is running at the idle mark, look again after this long.
const BUSY_RECHECK_MS: i64 = 10 * 60_000;

/// How long a `request_human` card waits for the person. Tests pass a
/// shorter deadline; the tool uses this.
pub const HUMAN_DEADLINE: Duration = Duration::from_mins(10);

/// How long a `request_human` card that delivers stays up unanswered before
/// it is expired and the teammate told, unless the teammate asked for longer.
/// Two hours, because a card the teammate has stopped needing keeps the
/// conversation reading "needs you" on every desk and phone until it goes.
const ASK_HOURS: u32 = 2;

/// The longest a teammate may ask a card to wait: a day, for an ask that
/// really waits for the person to come back.
const ASK_MAX_HOURS: u32 = 24;

/// Where the provider credentials come from.
///
/// The room never holds a secret: it asks at the start of every turn, so a
/// key or login added on the desk is in force on the next one and nothing
/// keeps a stale copy. The desk implements this from the vault and the room
/// log; a test hands over a map.
pub trait ProviderKeys: Send + Sync {
    fn provider_auth(&self) -> HashMap<String, ProviderAuth>;

    /// The models each provider may offer in the picker. An empty map shows
    /// every model: a missing filter is not an empty one. The desk reads it
    /// from the room log each time, so a saved filter is in force on the
    /// next turn without a restart. Test doubles leave it empty.
    fn enabled_models(&self) -> HashMap<String, Vec<String>> {
        HashMap::new()
    }

    /// The model a Hotline Agent teammate starts on when it has no choice of
    /// its own: the room's `defaultModelId`, else `lastModelId`. The desk
    /// reads the log each time, like [`Self::enabled_models`]. Test doubles
    /// leave it absent.
    fn preferred_model(&self) -> Option<String> {
        None
    }

    /// Validated live metadata, keyed by exact provider/model identity. Missing
    /// fields fall back to that exact bundled entry; no labels choose routes.
    fn model_metadata(&self) -> HashMap<String, crate::contract::CatalogModel> {
        HashMap::new()
    }

    /// Discovered and permitted manual ids for each active connection. An
    /// absent provider uses bundled fallback choices; a present empty list
    /// remains empty. Test doubles leave the map empty.
    fn account_models(&self) -> HashMap<String, Vec<String>> {
        HashMap::new()
    }

    /// The API key saved for each web search provider that has one. The desk
    /// reads them from the vault each time, so a key saved in Settings is in
    /// force on the next search. Test doubles have none: every provider keyless.
    fn web_search_keys(&self) -> crate::websearch::Keys {
        HashMap::new()
    }

    /// The desk's spending budgets, which a teammate's turn on a per-token
    /// key is metered against (the Chat budget). Test doubles have none, so
    /// their turns are neither metered nor refused.
    fn budget(&self) -> Option<Arc<crate::voice::metering::Budget>> {
        None
    }
}

/// What the room asks a model for: an agent to run a teammate's turns, and a
/// single answer to a single question.
///
/// The room builds its agents rather than being handed them, because it is the
/// room that decides when one is needed: a chapter closing means the next
/// message must reach a context that has never seen the chapter before it. The
/// note that closes a chapter is the other thing a model is asked for, and it
/// is asked here too — one seam for everything with a provider behind it, so
/// the room's own rules can be driven end to end without reaching one.
#[async_trait]
pub trait Agents: Send + Sync {
    /// A teammate's agent, chosen by the backend its record names, told the
    /// preamble, seeded with what has been said in the chapter it is joining,
    /// and given that teammate's own tools over its conversation. A backend
    /// this desk cannot run is refused here, in a sentence naming what is
    /// missing.
    fn agent(
        &self,
        persona: &Persona,
        preamble: String,
        said: Vec<Said>,
        tools: TeammateTools,
        extra_mcp: Vec<mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String>;

    /// One answer, with no tools and no conversation.
    async fn complete(&self, model_id: &str, system: &str, prompt: &str) -> Result<String, String>;
}

/// A voice front's reach into the room: see [`Room::front_reach`].
pub(crate) struct FrontReach {
    pub tools: TeammateTools,
    pub capability: CapabilityLease,
}

/// The agents this desk can really run: Hotline Agent on whatever keys the desk
/// holds at the moment it is asked, and any ACP harness the registry knows.
struct DeskAgents {
    keys: Arc<dyn ProviderKeys>,
    /// The data directory, which is where the ACP catalogue's cache lives.
    root: PathBuf,
    log: Log,
    mcp_vault: Option<Arc<Vault>>,
    acp_start: acp::StartBounds,
}

#[async_trait]
impl Agents for DeskAgents {
    fn agent(
        &self,
        persona: &Persona,
        preamble: String,
        said: Vec<Said>,
        tools: TeammateTools,
        extra_mcp: Vec<mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        let mut grant = mcp::grant(
            &mcp::servers(&room::settings(&self.log)),
            &persona.mcp_policy,
        );
        // The computer is not part of mcpPolicy: a teammate that asked for a
        // machine gets it even on a policy of none.
        grant.servers.extend(extra_mcp);
        let capability = tools.capability();
        if persona.backend_id == HOTLINE_BACKEND_ID {
            let driver = InProcess::new(
                self.keys.clone(),
                preamble,
                said,
                self.root.join("tool-output").join(&persona.id),
                tools,
            )
            .with_mcp(grant.servers, grant.missing);
            let driver = match &self.mcp_vault {
                Some(vault) => driver.with_mcp_vault(vault.clone()),
                None => driver,
            };
            let driver = match capability {
                Some(capability) => driver.with_capability(capability),
                None => driver,
            };
            return Ok(Arc::new(driver));
        }
        // The registry answers whether this machine can start that harness at
        // all, and says what is missing when it cannot.
        acp::registry::launch(&self.root, &persona.backend_id)?;
        let driver = ChildAgent::new(
            self.root.clone(),
            persona.backend_id.clone(),
            preamble,
            tools,
        )
        .with_history(said)
        .with_mcp(grant.servers, grant.missing)
        .with_start_bounds(self.acp_start);
        let driver = match &self.mcp_vault {
            Some(vault) => driver.with_mcp_vault(vault.clone()),
            None => driver,
        };
        let driver = match capability {
            Some(capability) => driver.with_capability(capability),
            None => driver,
        };
        Ok(Arc::new(driver))
    }

    async fn complete(&self, model_id: &str, system: &str, prompt: &str) -> Result<String, String> {
        rig::complete(
            &self.keys.provider_auth(),
            model_id,
            system,
            prompt,
            self.keys
                .model_metadata()
                .get(model_id)
                .and_then(|model| model.output_limit),
        )
        .await
    }
}

/// A session that is not running.
///
/// A fresh session and a room with nothing to report need this exact shape,
/// and a capability accidentally reading `true` in one of the two would light
/// up UI for something no agent ever claimed.
pub fn idle_info(persona_id: &str) -> SessionInfo {
    SessionInfo {
        persona_id: persona_id.to_string(),
        state: SessionState::Idle,
        session_id: None,
        agent_name: None,
        agent_version: None,
        context_restored: false,
        restore_note: None,
        models: Vec::new(),
        current_model_id: None,
        model_label: None,
        modes: Vec::new(),
        current_mode_id: None,
        mode_label: None,
        configs: Vec::new(),
        slash_commands: Vec::new(),
        capabilities: SessionCapabilities {
            active_input: false,
            load_session: false,
            resume: false,
            fork: false,
            mcp_http: false,
            image: false,
            additional_directories: false,
        },
        error: None,
        awaiting_subagents: false,
        processes: Vec::new(),
    }
}

/// One teammate's live conversation.
struct Session {
    persona_id: String,
    /// Whether this session was started with the teammate's computer
    /// attached. A session started while the image was still downloading, or
    /// after the computer failed, runs without it.
    computer: bool,
    /// What this session's agent holds the teammate's computer under.
    holder: crate::computer::gate::Holder,
    /// The generation every driver and tool handle for this session shares.
    capability: CapabilityLease,
    /// Which agent is answering, because a checkpoint is kept per backend and
    /// the tape's chapter markers name the one that wrote them.
    backend_id: String,
    driver: Arc<dyn Driver>,
    info: Mutex<SessionInfo>,
    /// Queued lines and the claim on the active driver share one lock.
    turns: Mutex<Turns>,
    input_ready: Notify,
    /// The message the next user line answers.
    pending_reply: Mutex<Option<Mark<String>>>,
    /// What the teammate's other threads posted to the chat since its last
    /// line ([`Room::post`]). A live session's context was read from the tape
    /// when it started, so these ride on its next line; a session that starts
    /// later reads them from the tape.
    pending_posts: Mutex<Vec<String>>,
    /// The user lines the driver has taken and not yet shown it read.
    ///
    /// A line is `sent` when it is written and `read` once the agent has
    /// produced anything with it in context — a thought, a tool call, a
    /// word. Taking the wire is not enough: a prompt the model errored on
    /// before it read it is the one case a read tick would lie about.
    unread: Mutex<Vec<String>>,
    voice_origin: Mutex<Option<crate::voice::Origin>>,
    dispatched_deliveries: Mutex<std::collections::HashSet<String>>,
    /// The turn's latest reply, which goes to the phone when the turn ends:
    /// a chatty turn is one notification, not one per bubble.
    glance: Mutex<Option<Glance>>,
    /// The agent's own id for this conversation, waiting for the turn that
    /// makes it worth remembering.
    ///
    /// Some agents issue an id at `session/new` and cannot reopen it until a
    /// prompt has committed, so the checkpoint is written when the first turn
    /// of a fresh session ends and not before. A session that was itself
    /// restored from a checkpoint has nothing to write: the id is already on
    /// the record.
    pending_checkpoint: Mutex<Option<String>>,
}

/// Pacing keeps this ID on the first bubble, while narration needs the full reply.
struct Glance {
    event_id: String,
    text: String,
}

/// Something a prompt wants stamped on the user line it is about to write,
/// and the moment the stamp stops being true.
///
/// The funnel is the only place a user event is made, so a prompt says what it
/// wants stamped by leaving one of these rather than by threading a parameter
/// through everything in between. See [`MARK_TTL_MS`] for the expiry.
struct Mark<T> {
    value: T,
    until: i64,
}

/// Leaves a mark for the next user line.
fn mark<T>(held: &Mutex<Option<Mark<T>>>, value: T) {
    *lock(held) = Some(Mark {
        value,
        until: now_ms() + MARK_TTL_MS,
    });
}

/// Takes the mark, if it is still the one its prompt meant.
fn take_fresh<T>(held: &Mutex<Option<Mark<T>>>, now: i64) -> Option<T> {
    lock(held)
        .take()
        .filter(|mark| now < mark.until)
        .map(|mark| mark.value)
}

/// One message on its way to a teammate: what the tape records, and what the
/// driver is handed, which are not always the same: the driver is told when
/// and from where it was said.
struct Sending {
    shown: String,
    wire: Wired,
    attachments: Option<Vec<Attachment>>,
}

/// One line as a driver takes it: the words, and the files handed over with
/// them. The two travel together and are never merged here, because how an
/// attachment reaches an agent is the driver's answer and not the room's.
#[derive(Clone)]
struct Wired {
    text: String,
    attachments: Vec<Attachment>,
    /// Operator input can steer. Internal nudges and deliveries queue.
    steer: bool,
    /// The user event this line was written as, so the tape can be told when
    /// the agent has read it. A nudge is never written and has none.
    said: Option<String>,
    /// The thread this line came from, when it came from one: a direct call's
    /// turn is its call thread's. Read in place of the prefix of `said`.
    from: Option<DeliveryFrom>,
    /// The call this line was said on, when it was: a direct call's is also
    /// `from`, and a desk call, which is no thread, is only this.
    voice: Option<crate::voice::Origin>,
    /// Whether it was said by voice, which decides how the reply is told.
    spoken: bool,
    /// Work the agent took up by itself between turns. Nothing is said to
    /// the driver; the turn is these updates.
    unprompted: Option<Unprompted>,
    /// The person's Retry of the turn that failed on this line: the driver
    /// runs it again rather than taking a new line.
    retry: bool,
}

/// The call a queued line came from, if it came from one: the thread it says
/// it came from, or, for the desk's own calls, which are no thread, the call
/// its producer named.
fn call_origin(wired: &Wired) -> Option<crate::voice::Origin> {
    wired
        .from
        .as_ref()
        .and_then(crate::voice::Origin::from_delivery)
        .or_else(|| wired.voice.clone())
}

impl Wired {
    fn words(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            attachments: Vec::new(),
            steer: false,
            said: None,
            from: None,
            voice: None,
            spoken: false,
            unprompted: None,
            retry: false,
        }
    }

    fn unprompted(updates: mpsc::Receiver<Update>) -> Self {
        Self {
            unprompted: Some(Unprompted(Arc::new(Mutex::new(Some(updates))))),
            ..Self::words(String::new())
        }
    }
}

/// The updates of work the agent started itself, taken once by the turn that
/// runs it. Shared only so the line it rides on stays cloneable.
#[derive(Clone)]
struct Unprompted(Arc<Mutex<Option<mpsc::Receiver<Update>>>>);

/// Every session in the room, and the one place their words are written down.
pub struct Room {
    voice: Mutex<std::sync::Weak<crate::voice::Calls>>,
    /// The room itself, for work that outlives the call that began it.
    me: Mutex<Weak<Room>>,
    log: Log,
    spending: crate::spending::SpendLedger,
    #[cfg(test)]
    image_generators: Mutex<Option<crate::imagegen::ImageSet>>,
    /// Where a test points web search: mock servers on localhost.
    #[cfg(test)]
    web_search_endpoints: Mutex<Option<crate::websearch::Endpoints>>,
    /// Which web providers are resting and the answers lately given.
    web_search_state: Arc<crate::websearch::State>,
    /// Pictures `generate_image` posted lately, per teammate, by content and
    /// when: `send_file` of the same picture soon after is a repeat.
    drawn: Mutex<HashMap<String, Vec<files::Drawn>>>,
    /// A computer being set up behind a session that started without it,
    /// per teammate: downloading, ready to join once the turn ends, or
    /// failed. Watched by `computer_status`, which can wait on it.
    computer_setups: Mutex<HashMap<String, watch::Sender<ComputerSetup>>>,
    /// Computers being updated behind a running session: the new release
    /// downloads while the old computer keeps working, and the swap waits
    /// for the turn in flight to end. Keyed by teammate.
    /// Why a teammate's last pressed update did not land, for its pane.
    computer_update_failures: Mutex<HashMap<String, String>>,
    computer_swaps: Mutex<HashMap<String, ComputerSwap>>,
    keys: Arc<dyn ProviderKeys>,
    agents: Arc<dyn Agents>,
    /// The one writer of the search index. `None` when it could not be opened,
    /// which costs search and never a record.
    indexer: Mutex<Option<Indexer>>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// Authorities are retained across stop so an old lease can never become
    /// valid merely because a new session starts for the same teammate.
    capabilities: Mutex<HashMap<String, CapabilityEpoch>>,
    /// Serializes candidate publication with invalidation.
    lifecycle: Mutex<()>,
    /// One start at a time, per teammate.
    ///
    /// The wire, a schedule firing and the chapter gate all bring a teammate
    /// up by the same motion, and that motion is long enough — a directory, a
    /// child process, a handshake — that two callers are routinely inside it
    /// at once. Two of them spawn two agents and open two chapter markers on
    /// one tape, and only the session inserted second is the one anything can
    /// stop afterwards. The gate is per teammate because a teammate is what is
    /// being started; two teammates starting together is not a race.
    starts: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    info_changes: broadcast::Sender<SessionInfo>,
    /// Teammates whose picture is being drawn right now. Not session state:
    /// a picture is drawn whether or not a session is running, and nothing
    /// about it survives a restart, so it lives here and not in the room.
    drawing: Mutex<HashSet<String>>,
    /// The turns in flight that `post_to_chat` may post from, by thread, and
    /// whether each has (`session/post.rs`).
    posting: Mutex<HashMap<ThreadId, post::Posting>>,
    call_closings: Mutex<HashMap<String, calls::CallClosing>>,
    /// The job threads the room holds: each job's queue, and its agent while
    /// it has one (`session/job_thread.rs`). Made at a job's first firing and
    /// dropped when it ends.
    job_threads: Mutex<HashMap<String, Arc<job_thread::LiveJob>>>,
    /// Subagents running now, by teammate, for the roster row. Like a
    /// drawing, nothing about a run survives a restart.
    subagents: Mutex<HashMap<String, Vec<crate::contract::RunningSubagent>>>,
    deltas: broadcast::Sender<StreamDelta>,
    /// Wakes the scheduler when a job is written, so a create does not wait
    /// for the nearest existing nextAt.
    schedule_changed: Arc<Notify>,
    /// Serializes schedule folds with tombstones and replacement events. A
    /// fire never holds this across its session start or prompt: it takes the
    /// lock only when it is ready to re-fold and append the next durable word.
    schedule_mutations: Mutex<()>,
    /// The sessions teammates answer each other out of.
    peers: peers::Peers,
    /// The side threads that are live: second conversations with a teammate.
    sides: sides::Sides,
    /// A `request_human` wait, by the card's action id. The tool parks on
    /// the oneshot; the person's answer, the deadline, or a settle (session
    /// stop, room restart) is what sends.
    human_waits: Mutex<HashMap<String, HumanWait>>,
    voice_questions: Mutex<HashMap<String, asking::Question>>,
    /// Held while a card that delivers is read and settled, so two answers,
    /// or an answer and the expiry sweep, cannot both deliver.
    answering_later: Mutex<()>,
    /// The live half of this desk's links: open channels, the calls waiting
    /// for them, and linked desks' teammates (ADR 0003).
    links: Arc<crate::linked::Links>,
    /// Serializes each durable exchange queue and its pair-wide brake.
    exchange_lock: Mutex<()>,
    exchange_workers: Mutex<std::collections::HashSet<String>>,
    exchange_leases: Mutex<HashMap<String, (CapabilityLease, CapabilityLease)>>,
    /// Who is driving each teammate's computer: one thread at a time, whichever
    /// of the teammate's threads asked first (`computer/gate.rs`).
    computer_leases: Arc<crate::computer::gate::Leases>,
    /// The phones paired with this desk, told when a reply lands or a card
    /// needs the person.
    push: crate::push::Push,
    /// One container per teammate, tokens in process state.
    computers: Computer,
    /// Where the secrets a computer is granted are read from, on their way
    /// to it. A room without one — a test's — grants none.
    vault: Option<Arc<Vault>>,
    /// The passkey each teammate's computer is armed to make, by teammate:
    /// the name it will be stored under and the site, until the poll that
    /// finds it made, a cancel, or the computer's own ten minutes run out.
    passkey_armings: Mutex<HashMap<String, PasskeyArming>>,
    #[cfg(test)]
    pub(crate) cookie_candidates: Mutex<Vec<crate::computer::cookies::Candidate>>,
    /// A host permission failure is said once per browser for this desk run,
    /// across teammates and session restarts.
    cookie_access_notices: Mutex<HashSet<String>>,
    secret_uses: Mutex<HashMap<String, Arc<TokioMutex<secret_uses::UseWatch>>>>,
    captures: Mutex<HashMap<String, Arc<tokio::sync::Mutex<capture::Capture>>>>,
    /// Serializes policy changes across sockets, including the interval from
    /// capability invalidation through the durable append and reattach.
    policy_updates: Arc<TokioMutex<()>>,
    /// Work holds a read lease; an installer can pause an idle room atomically.
    activity: Arc<tokio::sync::RwLock<()>>,
    /// Set once the desk is stopping for a restart: new work is refused while
    /// the turns already running drain (see `stopping`).
    closing: std::sync::atomic::AtomicBool,
}

impl Room {
    /// The room on the agents this desk can really run, with no vault: what
    /// tests build. The desk itself builds its room with
    /// [`Room::new_with_mcp`].
    ///
    /// The constructors telescope rather than take an options struct because
    /// each one names the single seam its callers replace, and every one has
    /// callers; an options struct would lengthen each of them by the seams
    /// it leaves alone.
    pub fn new(log: Log, keys: Arc<dyn ProviderKeys>) -> Arc<Self> {
        let agents = Arc::new(DeskAgents {
            keys: keys.clone(),
            root: log.root().to_path_buf(),
            log: log.clone(),
            mcp_vault: None,
            acp_start: acp::StartBounds::default(),
        });
        Self::with_agents(log, keys, agents)
    }

    /// The production room passes its protected vault to both drivers. Tests
    /// use new or with_agents and therefore keep their in-memory seams.
    pub(crate) fn new_with_mcp(
        log: Log,
        keys: Arc<dyn ProviderKeys>,
        vault: Arc<Vault>,
        acp_start: acp::StartBounds,
    ) -> Arc<Self> {
        let agents = Arc::new(DeskAgents {
            keys: keys.clone(),
            root: log.root().to_path_buf(),
            log: log.clone(),
            mcp_vault: Some(vault.clone()),
            acp_start,
        });
        Self::with_agents_computers_and_vault(log, keys, agents, Computer::new(), Some(vault))
    }

    /// The room, on an agent seam a test can script. [`Room::new`] is this on
    /// the agents this desk can really run.
    pub(crate) fn with_agents(
        log: Log,
        keys: Arc<dyn ProviderKeys>,
        agents: Arc<dyn Agents>,
    ) -> Arc<Self> {
        Self::with_agents_and_computers(log, keys, agents, Computer::new())
    }

    /// The room on a computer runtime a test can script as well.
    pub(crate) fn with_agents_and_computers(
        log: Log,
        keys: Arc<dyn ProviderKeys>,
        agents: Arc<dyn Agents>,
        computers: Computer,
    ) -> Arc<Self> {
        Self::with_agents_computers_and_vault(log, keys, agents, computers, None)
    }

    /// The room with a vault to read a computer's granted secrets from.
    pub(crate) fn with_agents_computers_and_vault(
        log: Log,
        keys: Arc<dyn ProviderKeys>,
        agents: Arc<dyn Agents>,
        computers: Computer,
        vault: Option<Arc<Vault>>,
    ) -> Arc<Self> {
        let push = crate::push::Push::new(log.root());
        let indexer = match Indexer::open(&log) {
            Ok(indexer) => Some(indexer),
            Err(error) => {
                eprintln!("the search index could not be opened: {error}");
                None
            }
        };
        let room = Arc::new(Self {
            spending: crate::spending::SpendLedger::new(log.root().to_path_buf()),
            #[cfg(test)]
            image_generators: Mutex::new(None),
            #[cfg(test)]
            web_search_endpoints: Mutex::new(None),
            web_search_state: Arc::default(),
            drawn: Mutex::default(),
            voice: Mutex::new(std::sync::Weak::new()),
            me: Mutex::new(Weak::new()),
            log,
            keys,
            agents,
            indexer: Mutex::new(indexer),
            sessions: Mutex::new(HashMap::new()),
            capabilities: Mutex::new(HashMap::new()),
            lifecycle: Mutex::new(()),
            starts: Mutex::new(HashMap::new()),
            computer_setups: Mutex::new(HashMap::new()),
            computer_swaps: Mutex::new(HashMap::new()),
            computer_update_failures: Mutex::new(HashMap::new()),
            info_changes: broadcast::channel(BROADCAST_DEPTH).0,
            drawing: Mutex::new(HashSet::new()),
            posting: Mutex::new(HashMap::new()),
            call_closings: Mutex::new(HashMap::new()),
            job_threads: Mutex::new(HashMap::new()),
            subagents: Mutex::new(HashMap::new()),
            deltas: broadcast::channel(BROADCAST_DEPTH).0,
            schedule_changed: Arc::new(Notify::new()),
            schedule_mutations: Mutex::new(()),
            peers: peers::Peers::default(),
            sides: sides::Sides::default(),
            human_waits: Mutex::new(HashMap::new()),
            voice_questions: Mutex::new(HashMap::new()),
            answering_later: Mutex::new(()),
            links: Arc::default(),
            exchange_lock: Mutex::new(()),
            exchange_workers: Mutex::new(HashSet::default()),
            exchange_leases: Mutex::new(HashMap::default()),
            computer_leases: crate::computer::gate::Leases::new(),
            push,
            computers,
            vault,
            passkey_armings: Mutex::new(HashMap::new()),
            secret_uses: Mutex::default(),
            #[cfg(test)]
            cookie_candidates: Mutex::default(),
            cookie_access_notices: Mutex::default(),
            captures: Mutex::new(HashMap::new()),
            policy_updates: Arc::new(TokioMutex::new(())),
            activity: Arc::new(tokio::sync::RwLock::new(())),
            closing: std::sync::atomic::AtomicBool::new(false),
        });
        *lock(&room.me) = Arc::downgrade(&room);
        room.settle();
        secret_uses::watch(Arc::downgrade(&room));
        // What the person said that was still waiting when the last desk
        // stopped, and what came back to a job that had not heard it yet, is
        // handed on once, now that the room is up.
        let resuming = Arc::downgrade(&room);
        tokio::spawn(async move {
            if let Some(room) = resuming.upgrade() {
                room.resume_pending().await;
                room.resume_job_deliveries().await;
            }
        });
        follow_model_changes(Arc::downgrade(&room), room.log.subscribe(&StreamId::Room));
        sweep_idle_chapters(Arc::downgrade(&room));
        schedule::start(Arc::downgrade(&room), room.schedule_changed.clone());
        room
    }

    fn working(&self) -> Result<tokio::sync::OwnedRwLockReadGuard<()>, String> {
        if self.closing() {
            return Err("Hotline is restarting. Try again in a moment.".to_string());
        }
        self.lease()
    }

    /// A work lease even while the room is closing: for work already admitted
    /// that hands a line on, never for anything new.
    fn lease(&self) -> Result<tokio::sync::OwnedRwLockReadGuard<()>, String> {
        self.activity.clone().try_read_owned().map_err(|_| {
            "Hotline is preparing to restart for an update. Try again when it finishes.".to_string()
        })
    }

    /// Refuses while any turn, queued work, session start, peer request or
    /// chapter handoff is running. Dropping the lease resumes the room after
    /// a failed installation; scheduled jobs remain on disk throughout.
    pub fn prepare_restart(&self) -> Result<tokio::sync::OwnedRwLockWriteGuard<()>, String> {
        let held = self.activity.clone().try_write_owned()
            .map_err(|_| "Teammates are still working. Wait for them to finish or stop their turns, then try again.".to_string())?;
        self.log
            .sync(&StreamId::Room)
            .map_err(|error| error.to_string())?;
        for persona in room::roster(&self.log) {
            self.log
                .sync(&StreamId::Tape(persona.id))
                .map_err(|error| error.to_string())?;
        }
        for key in thread::list_all_keys(self.log.root()) {
            self.log
                .sync(&StreamId::Pair(key))
                .map_err(|error| error.to_string())?;
        }
        Ok(held)
    }

    /// The room-wide gate used by the wire around policy updates. A caller
    /// holds it across invalidate, the log append, and reattach so another
    /// socket cannot reactivate a generation between those steps.
    pub(crate) fn policy_update_lock(&self) -> Arc<TokioMutex<()>> {
        self.policy_updates.clone()
    }

    pub async fn agent_auth_start(
        &self,
        persona_id: &str,
        method_id: &str,
        owner: tokio_util::sync::CancellationToken,
    ) -> Result<String, String> {
        let _working = self.working()?;
        let session = self.session(persona_id)?;
        if lock(&session.info).state == SessionState::Thinking {
            return Err("Stop this teammate's turn before signing in.".into());
        }
        session.driver.clone().auth_start(method_id, owner).await
    }
    pub fn agent_auth_poll(
        &self,
        persona_id: &str,
        id: &str,
    ) -> Result<crate::wire::AuthStatus, String> {
        self.session(persona_id)?.driver.auth_poll(id)
    }

    pub fn agent_auth_input(&self, persona_id: &str, id: &str, input: &str) -> Result<(), String> {
        self.session(persona_id)?.driver.auth_input(id, input)
    }
    pub fn agent_auth_cancel(&self, persona_id: &str, id: &str) -> Result<(), String> {
        self.session(persona_id)?.driver.auth_cancel(id)
    }

    /// What the teammate's session is doing. A teammate with no session is
    /// idle, which is a state and not an absence.
    pub fn info(&self, persona_id: &str) -> SessionInfo {
        let mut info = match lock(&self.sessions).get(persona_id) {
            Some(session) => lock(&session.info).clone(),
            None => idle_info(persona_id),
        };
        for processes in self.sides.processes(persona_id) {
            info.processes.extend(processes.listed());
        }
        info
    }

    /// Whether this teammate's picture is being drawn.
    pub fn drawing(&self, persona_id: &str) -> bool {
        lock(&self.drawing).contains(persona_id)
    }

    /// The subagents this teammate has running, oldest first.
    pub fn subagents(&self, persona_id: &str) -> Vec<crate::contract::RunningSubagent> {
        lock(&self.subagents)
            .get(persona_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Lists a run on its teammate's roster row while it is running, and
    /// takes it off once it is not, telling the roster either way.
    pub(crate) fn list_subagent(
        &self,
        persona_id: &str,
        run: crate::contract::RunningSubagent,
        running: bool,
    ) {
        {
            let mut all = lock(&self.subagents);
            let runs = all.entry(persona_id.to_string()).or_default();
            runs.retain(|one| one.run_id != run.run_id);
            if running {
                runs.push(run);
            }
            if runs.is_empty() {
                all.remove(persona_id);
            }
        }
        let _ = self.info_changes.send(self.info(persona_id));
    }

    /// Marks a picture as being drawn until the guard drops, telling the
    /// roster both times. The roster rebuilds a row on any info change, so
    /// the teammate's own info is the nudge.
    pub(crate) fn start_drawing(&self, persona_id: &str) -> Drawing<'_> {
        lock(&self.drawing).insert(persona_id.to_string());
        let _ = self.info_changes.send(self.info(persona_id));
        Drawing {
            room: self,
            persona_id: persona_id.to_string(),
        }
    }

    /// Every session state change from here on, for the roster view.
    pub fn subscribe_info(&self) -> broadcast::Receiver<SessionInfo> {
        self.info_changes.subscribe()
    }

    /// Text as the agents write it. Never written down; the durable line is
    /// the tape event that lands when the message is whole.
    pub fn subscribe_deltas(&self) -> broadcast::Receiver<StreamDelta> {
        self.deltas.subscribe()
    }

    /// The models this desk's keys unlock, as the picker lists them.
    pub fn models_for_desk(&self) -> Vec<ConfigChoice> {
        crate::models::choices(
            &self.keys.provider_auth(),
            &self.keys.enabled_models(),
            &self.keys.account_models(),
            &self.keys.model_metadata(),
        )
    }

    /// What tools this teammate was given the last time it started, less
    /// the missing rows of any server that no longer exists. `None` when it
    /// has never started under a Hotline that keeps a ledger. Deleting a
    /// server settles it: there is nothing to pick or fix, so nothing is owed
    /// an error, even when an older policy still names its id. A server that
    /// exists and failed stays missing out loud, and so does the teammate's
    /// computer, which settings never list.
    pub fn teammate_tools(&self, persona_id: &str) -> Option<TeammateToolLedger> {
        let mut ledger = ledger::teammate_tools(persona_id)?;
        let servers = mcp::servers(&room::settings(&self.log));
        ledger.rows.retain(|row| {
            row.source != ToolSourceKind::Mcp
                || row.state != ToolState::Absent
                || row.origin == crate::computer::SERVER_ID
                || servers.iter().any(|server| server.id == row.origin)
        });
        Some(ledger)
    }

    /// The room's streams, for the teammate tools that read a tape.
    pub(crate) fn log(&self) -> &Log {
        &self.log
    }

    fn mid_turn(&self, persona_id: &str) -> bool {
        matches!(
            self.info(persona_id).state,
            SessionState::Thinking | SessionState::Starting
        )
    }

    /// The desk window has the person, or has lost them; the phone is quiet
    /// while it has them.
    pub fn desk_looking(&self, looking: bool) {
        self.push.looking(looking);
    }

    /// The turn's reply to the phone, one notification however many bubbles
    /// it took: the report when the agent worked, or the answer that needed
    /// no tool.
    fn send_glance(&self, session: &Session, from_voice: bool) {
        let Some(Glance { event_id, text }) = lock(&session.glance).take() else {
            return;
        };
        let name = self
            .persona(&session.persona_id)
            .map_or_else(|_| "Hotline".to_string(), |persona| persona.name);
        if let Some(voice) = lock(&self.voice).upgrade()
            && voice.delivery(
                &session.persona_id,
                &event_id,
                &name,
                &text,
                from_voice,
                lock(&session.voice_origin).as_ref(),
            )
        {
            return;
        }
        self.push.notify(&name, &text, &session.persona_id, None);
    }

    pub(crate) fn set_voice(&self, voice: &Arc<crate::voice::Calls>) {
        *lock(&self.voice) = Arc::downgrade(voice);
        self.push.set_voice(voice);
    }

    /// Keeps `spoken` as what a call to the teammate said for its reply
    /// `event_id`, on the reply's first bubble: the call assistant's rewrite
    /// of a reply that wrote no spoken version ([`crate::voice::spoken`]).
    /// False while the reply is not yet on the tape.
    pub(crate) fn voice_spoken(&self, persona_id: &str, event_id: &str, spoken: &str) -> bool {
        let tape = self.tape(persona_id);
        let Some(reply) = tape
            .iter()
            .rev()
            .find(|event| event["id"] == event_id && event["kind"] == "agent")
        else {
            return false;
        };
        let mut reply = reply.clone();
        reply["spoken"] = Value::from(spoken);
        self.write_value(persona_id, &reply);
        true
    }

    pub(crate) fn voice_push(&self, persona_id: &str, name: &str, text: &str) {
        self.push.notify(name, text, persona_id, None);
    }

    #[cfg(test)]
    pub(crate) fn sent_voice_pushes(&self) -> Vec<Value> {
        lock(&self.push.sent).clone()
    }

    /// The title of a notification about a card: the teammate needs you.
    fn needs_you(&self, persona_id: &str) -> String {
        match self.persona(persona_id) {
            Ok(persona) => format!("{} needs you", persona.name),
            Err(_) => "Hotline needs you".to_string(),
        }
    }

    /// Remembers the agent's session id, now that a turn on it has completed.
    ///
    /// Written once per fresh session and only after a turn, because some
    /// agents issue an id they cannot reopen until a prompt has committed —
    /// and a checkpoint that fails to load is a teammate that starts cold
    /// believing it did not have to.
    fn checkpoint(&self, session: &Session) {
        let _lifecycle = lock(&self.lifecycle);
        if !session.capability.is_current() {
            return;
        }
        let Some(session_id) = lock(&session.pending_checkpoint).take() else {
            return;
        };
        if let Err(error) = room::checkpoint_session(
            &self.log,
            &session.persona_id,
            &session.backend_id,
            &session_id,
        ) {
            eprintln!(
                "the session for {} was not remembered: {error}",
                session.persona_id
            );
        }
        // The marker keeps the id, which is what makes reopening this
        // chapter possible later. The persona record is what the next
        // start will find; the marker is what a resume points that record
        // back at.
        self.stamp_chapter_session(&session.persona_id, &session_id);
    }

    /// Writes the agent's session id onto the open chapter marker, so a
    /// later resume can point the checkpoint back at this stretch.
    fn stamp_chapter_session(&self, persona_id: &str, session_id: &str) {
        let (open, _) = self
            .log
            .chapter_activity(&StreamId::Tape(persona_id.to_string()));
        let Some(open) = open else {
            return;
        };
        if open.get("sessionId").and_then(Value::as_str) == Some(session_id) {
            return;
        }
        let mut stamped = open;
        if let Some(object) = stamped.as_object_mut() {
            object.insert("sessionId".into(), Value::from(session_id));
            self.write_value(persona_id, &stamped);
        }
    }

    fn fail_in_flight(&self, session: &Session, in_flight: &mut HashMap<String, PendingTool>) {
        for (call_id, pending) in in_flight.drain() {
            self.append(session, pending.event(&call_id, ToolStatus::Failed, None));
        }
    }

    /// Writes one event to the teammate's tape and offers it to the index.
    ///
    /// Every line the room writes down passes here, which is what lets the
    /// stamps a prompt left be stated once for both kinds of agent.
    fn append(&self, session: &Session, event: TranscriptEvent) {
        self.write(&session.persona_id, &stamped(session, event, now_ms()));
    }

    /// One event onto the tape and into the index, with nothing stamped on it.
    ///
    /// A chapter marker and the notice that a note is missing come this way:
    /// they are the room writing in its own voice, not a teammate speaking,
    /// and there is no reply for them to be part of.
    /// Where an image pull shows: live progress for the computer's button,
    /// never a line on the tape. A download is the room's business, and the
    /// conversation carries on as if it were not happening.
    fn pull_reporter<'a>(
        &'a self,
        persona_id: &'a str,
    ) -> impl FnMut(crate::computer::PullReport) + 'a {
        use crate::computer::PullOutcome;
        use crate::contract::PullStatus;
        move |report| {
            let status = match report.outcome {
                PullOutcome::Pulling => PullStatus::Pulling,
                PullOutcome::Done { .. } => PullStatus::Done,
                PullOutcome::Failed => PullStatus::Failed,
            };
            let _ = self.deltas.send(StreamDelta::ComputerPull {
                persona_id: persona_id.to_string(),
                layers_done: report.layers_done,
                layers_total: report.layers_total,
                status,
            });
        }
    }

    fn write(&self, persona_id: &str, event: &TranscriptEvent) {
        match serde_json::to_value(event) {
            Ok(event) => self.write_value(persona_id, &event),
            Err(error) => {
                eprintln!("a transcript event for {persona_id} could not be written: {error}");
            }
        }
    }

    /// One line onto the tape and into the index. Everything the room writes
    /// down ends here; the only callers that spell an event as JSON rather
    /// than as a [`TranscriptEvent`] are the ones superseding a line they read
    /// off the tape, which is already JSON.
    fn write_value(&self, persona_id: &str, event: &Value) {
        if let Err(error) = self.try_write_value(persona_id, event) {
            eprintln!("the tape for {persona_id} could not be appended to: {error}");
        }
    }

    /// The same, saying whether the line landed, for a caller about to tell
    /// someone it did.
    fn try_write_value(&self, persona_id: &str, event: &Value) -> std::io::Result<()> {
        // A teammate on a linked desk has no tape here: what it says and is
        // told is on its own desk. A marker meant for its side has nowhere to
        // land, and that is not a failure.
        if crate::linked::is_linked(persona_id) {
            return Ok(());
        }
        let stamped = self.card_with_origin(persona_id, event);
        let event = stamped.as_ref();
        self.log
            .append(&StreamId::Tape(persona_id.to_string()), event)?;
        if call_cards::is_card(event)
            && event["from"]["kind"] == "call"
            && let Some(key) = event["from"]["thread"].as_str()
        {
            let thread = ThreadId {
                kind: crate::contract::ThreadKind::Call,
                key: key.to_string(),
            };
            let stream = StreamId::of_thread(&thread).expect("a call has a stream");
            self.log.append(&stream, event)?;
        }
        self.index(persona_id, event);
        if persona_id != "voice-dispatcher"
            && let Some(voice) = lock(&self.voice).upgrade()
        {
            voice.card(persona_id, event);
        }
        Ok(())
    }

    /// The index is an index: it is rebuilt from the tape whenever the two
    /// disagree, so a failure here costs a search and never a record.
    fn index(&self, persona_id: &str, event: &Value) {
        let mut indexer = lock(&self.indexer);
        let Some(indexer) = indexer.as_mut() else {
            return;
        };
        if let Err(error) = indexer.index_event(persona_id, event) {
            eprintln!("the search index rejected an event for {persona_id}: {error}");
        }
    }

    fn set_state(&self, session: &Arc<Session>, state: SessionState) {
        let _lifecycle = lock(&self.lifecycle);
        if !session.capability.is_current()
            || !lock(&self.sessions)
                .get(&session.persona_id)
                .is_some_and(|current| Arc::ptr_eq(current, session))
            || (state == SessionState::Ready && lock(&session.turns).running)
        {
            return;
        }
        let info = {
            let mut info = lock(&session.info);
            info.state = state;
            // A turn beginning or ending is the end of any wait on subagents.
            info.awaiting_subagents = false;
            info.clone()
        };
        let _ = self.info_changes.send(info);
    }

    /// Says whether the turn in flight reads as done: the agent has said its
    /// reply and the turn is open only for subagents it started. A display
    /// fact, told to the roster when it changes; the turn is still running,
    /// so `state` stays `thinking` and nothing that waits on a turn's end
    /// (a chapter's close, a computer's swap) stops waiting.
    fn await_subagents(&self, session: &Arc<Session>, awaiting: bool) {
        let _lifecycle = lock(&self.lifecycle);
        if !session.capability.is_current() || !self.current_session(session) {
            return;
        }
        let info = {
            let mut info = lock(&session.info);
            if info.awaiting_subagents == awaiting
                || (awaiting && info.state != SessionState::Thinking)
            {
                return;
            }
            info.awaiting_subagents = awaiting;
            info.clone()
        };
        let _ = self.info_changes.send(info);
    }

    /// Tells the roster which commands the session holds running whenever
    /// that changes. A display fact like `awaiting_subagents`: a process
    /// never makes the session read as working.
    fn watch_processes(self: &Arc<Self>, session: &Arc<Session>, processes: &Arc<Processes>) {
        let room = Arc::downgrade(self);
        let weak_session = Arc::downgrade(session);
        let weak_processes = Arc::downgrade(processes);
        let mut changes = processes.changes();
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                let (Some(room), Some(session), Some(processes)) = (
                    room.upgrade(),
                    weak_session.upgrade(),
                    weak_processes.upgrade(),
                ) else {
                    break;
                };
                let listed = processes.listed();
                let _lifecycle = lock(&room.lifecycle);
                if !session.capability.is_current() || !room.current_session(&session) {
                    break;
                }
                let info = {
                    let mut info = lock(&session.info);
                    if info.processes == listed {
                        continue;
                    }
                    info.processes = listed;
                    info.clone()
                };
                let _ = room.info_changes.send(info);
            }
        });
    }

    /// Find a command only among this teammate's home and work threads.
    fn processes_of(&self, persona_id: &str, id: &str) -> Result<Arc<Processes>, String> {
        let home = lock(&self.sessions)
            .get(persona_id)
            .and_then(|session| session.driver.processes());
        home.into_iter()
            .chain(self.sides.processes(persona_id))
            .find(|processes| processes.output(id).is_some())
            .ok_or_else(|| "That process is not running.".to_string())
    }

    /// Stops one command the teammate left running. It does not wake the
    /// teammate: the person stopped it, and the chip going says so.
    pub fn process_stop(&self, persona_id: &str, id: &str) -> Result<(), String> {
        self.processes_of(persona_id, id)?
            .cancel(jobs::JobArgs {
                job_id: id.to_string(),
                reason: Some("stopped by the person".to_string()),
            })
            .map(|_| ())
            .map_err(|_| "That process is not running.".to_string())
    }

    /// One running command in full, and the latest of its output.
    pub fn process_output(
        &self,
        persona_id: &str,
        id: &str,
    ) -> Result<crate::contract::ProcessOutput, String> {
        let (command, tail) = self
            .processes_of(persona_id, id)?
            .output(id)
            .ok_or_else(|| "That process is not running.".to_string())?;
        Ok(crate::contract::ProcessOutput { command, tail })
    }

    fn persona(&self, persona_id: &str) -> Result<Persona, String> {
        room::roster(&self.log)
            .into_iter()
            .find(|persona| persona.id == persona_id)
            .ok_or_else(|| "There is no such teammate in this room.".to_string())
    }

    fn reach_of(&self, persona_id: &str) -> Reach {
        self.persona(persona_id)
            .ok()
            .and_then(|persona| persona.reach)
            .unwrap_or_default()
    }

    fn tape(&self, persona_id: &str) -> Vec<Value> {
        self.log.load(&StreamId::Tape(persona_id.to_string()))
    }

    fn session(&self, persona_id: &str) -> Result<Arc<Session>, String> {
        let session = lock(&self.sessions)
            .get(persona_id)
            .cloned()
            .ok_or_else(|| "That teammate is not running.".to_string())?;
        session.capability.check()?;
        Ok(session)
    }
}

fn lock<T>(held: &Mutex<T>) -> MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(crate) fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

/// The system's own opener, so the link lands in whatever browser the
/// person made their default.
async fn open_in_browser(link: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = tokio::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        // Not `cmd /c start`: cmd would read a link's `&` as its own.
        let mut command = tokio::process::Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = tokio::process::Command::new("xdg-open");
    command
        .arg(link)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    crate::process_windows::quiet(&mut command);
    let status = tokio::time::timeout(Duration::from_secs(10), command.status())
        .await
        .map_err(|_| "The browser did not answer in time.".to_string())?
        .map_err(|_| "This computer has no way to open a browser.".to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("This computer could not open its browser.".into())
    }
}

/// A line as the model hears it: the local day and time it was said, then
/// the words. The time rides on the line and never in the preamble, so the
/// preamble reads the same every day and stays a cached prefix, and a
/// chapter that runs past midnight still knows what day it is.
pub(crate) fn timed(ts: i64, text: &str) -> String {
    timed_from(ts, None, text)
}

/// The same, naming which of the person's apps wrote it when the door it came
/// in at knows: `[Fri 2 Oct 2026, 16:20 · phone]`.
/// The line a retry would send again: the person's latest message, when the
/// last turn after it failed and nothing else has been said since.
struct FailedLine {
    ts: i64,
    client: Option<Client>,
    text: String,
    attachments: Vec<Attachment>,
}

fn failed_line(tape: &[Value]) -> Option<FailedLine> {
    let last_turn = tape.iter().rposition(|event| event["kind"] == "turn")?;
    let last_user = tape.iter().rposition(|event| event["kind"] == "user")?;
    if last_turn < last_user || tape[last_turn]["stopReason"] != "failed" {
        return None;
    }
    let TranscriptEvent::User {
        ts,
        text,
        attachments,
        client,
        ..
    } = serde_json::from_value(tape[last_user].clone()).ok()?
    else {
        return None;
    };
    Some(FailedLine {
        ts,
        client,
        text,
        attachments: attachments.unwrap_or_default(),
    })
}

pub(crate) fn timed_from(ts: i64, client: Option<Client>, text: &str) -> String {
    let at = Local
        .timestamp_millis_opt(ts)
        .single()
        .unwrap_or_else(Local::now);
    let from = match client {
        Some(Client::Desktop) => " · desktop",
        Some(Client::Phone) => " · phone",
        None => "",
    };
    format!("[{}{from}] {text}", at.format("%a %-d %b %Y, %H:%M"))
}

#[cfg(test)]
pub(crate) mod tests;

/// A picture being drawn: dropping it, on success, refusal or cancellation
/// alike, takes the teammate off the roster's drawing list.
pub(crate) struct Drawing<'a> {
    room: &'a Room,
    persona_id: String,
}

impl Drop for Drawing<'_> {
    fn drop(&mut self) {
        lock(&self.room.drawing).remove(&self.persona_id);
        let _ = self
            .room
            .info_changes
            .send(self.room.info(&self.persona_id));
    }
}
