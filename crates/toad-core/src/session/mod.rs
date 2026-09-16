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
//!   previous Toad wrote — [`crate::contract::TranscriptEvent`] pins them, and
//!   a tape written here opens in that app unchanged. An agent's message is
//!   split into bubbles ([`pacing`]) here so both kinds of agent and a
//!   peer thread get the same ones.
//! - Every append is offered to the search index. The index is rebuildable, so
//!   a failure there is printed and swallowed; a failure to write the tape is
//!   the record, and is printed too because nothing above can undo it.
//! - Deltas go out on [`Room::subscribe_deltas`] and are never written; the
//!   durable line is the message that lands when it is whole.
//! - A scheduled run marked quiet has its teammate's voice demoted to thinking
//!   for the length of its own turn. That gate is [`quiet`], and it lives here
//!   because it must hold for whichever driver ran the turn.
//! - A job on the room stream wakes a teammate when its `nextAt` arrives.
//!   That clock is [`schedule`], and it lives here because a firing is a
//!   prompt — the same funnel, the same quiet window — not a second way of
//!   speaking to a teammate.
//! - A message goes to an agent whose context is the chapter it is in. The
//!   room opens a chapter when a session starts, closes one that has gone
//!   quiet, and replaces the agent whose chapter closed before the next
//!   message reaches it. That is [`chapters`], and it lives here for the same
//!   reason: a chapter is a fact about the room, not about the agent.
//!
//! Drivers and tools capture revocable session authority. Changes to reach,
//! tools, the workspace, or the harness invalidate old handles and queued
//! work before a live session is rebuilt.

mod chapters;
pub(crate) mod jobs;
pub(crate) mod ledger;
mod narration;
mod pacing;
mod peers;
mod quiet;
pub(crate) mod schedule;

pub use peers::{DeliverResult, TEAMMATE_MESSAGE_MAX};
pub use schedule::{parse_duration, parse_when};

use crate::computer::Computer;
use crate::contract::{
    Attachment, ChapterClose, ChapterSummary, ComputerStatus, ConfigChoice, HumanActionStatus,
    HumanAnswer, NoticeLevel, Persona, Reach, Receipt, RuntimeReport, ScheduleKind, ScheduledRun,
    SessionCapabilities, SessionInfo, SessionState, StreamDelta, TeammateToolLedger, ToolOutput,
    ToolStatus, TranscriptEvent,
};
use crate::driver::acp::{self, ChildAgent};
use crate::driver::rig;
use crate::driver::rig::{InProcess, Said};
use crate::driver::{
    CapabilityEpoch, CapabilityLease, Driver, DriverInfo, MessageKind, TOAD_BACKEND_ID, Update,
    clip,
};
use crate::log::{Log, StreamId, thread};
use crate::mcp;
use crate::mcp::server::TeammateTools;
use crate::paths;
use crate::room;
use crate::store::chapters as chapter_view;
use crate::store::search::Indexer;
use crate::vault::Vault;
use async_trait::async_trait;
use chrono::Local;
use quiet::QuietWindow;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;
use tokio::sync::{Mutex as TokioMutex, Notify, broadcast, oneshot, watch};

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
/// in the previous Toad a prompt could be refused between leaving the mark and
/// writing the line, and these fifteen seconds are what kept that stamp off
/// somebody's later, unrelated message.
const MARK_TTL_MS: i64 = 15_000;

/// How the idle clock runs. The first look is soon after the room opens,
/// because a chapter that went stale while Toad was closed should be closed
/// before the person who closed it comes back; after that a minute is finer
/// than any setting the room allows, and the work is a fold nobody is waiting
/// on.
const FIRST_SWEEP: Duration = Duration::from_secs(5);
const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// While a turn is running at the idle mark, look again after this long.
const BUSY_RECHECK_MS: i64 = 10 * 60_000;

/// How long a `request_human` card waits for the person. Tests pass a
/// shorter deadline; the tool uses this.
pub const HUMAN_DEADLINE: Duration = Duration::from_secs(10 * 60);

/// How Toad Agent reaches a provider: a key, a login directory, or a
/// configured endpoint with its own protocol and optional key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderAuth {
    ApiKey(String),
    Unavailable(String),
    StoredLogin {
        tokens: crate::credentials::CredentialFile,
    },
    Login {
        token_dir: PathBuf,
    },
    Local {
        base_url: String,
    },
    Custom {
        name: String,
        base_url: String,
        api_key: Option<String>,
        config: crate::contract::CustomProvider,
    },
}

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

    /// The model a Toad Agent teammate starts on when it has no choice of
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

/// The agents this desk can really run: Toad Agent on whatever keys the desk
/// holds at the moment it is asked, and any ACP harness the registry knows.
struct DeskAgents {
    keys: Arc<dyn ProviderKeys>,
    /// The data directory, which is where the ACP catalogue's cache lives.
    root: PathBuf,
    log: Log,
    mcp_vault: Option<Arc<Vault>>,
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
        if persona.backend_id == TOAD_BACKEND_ID {
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
        .with_mcp(grant.servers, grant.missing);
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
        },
        error: None,
    }
}

/// One teammate's live conversation.
struct Session {
    persona_id: String,
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
    /// The firing the next user line belongs to.
    pending_scheduled: Mutex<Option<Mark<ScheduledRun>>>,
    /// The user lines the driver has taken and not yet shown it read.
    ///
    /// A line is `sent` when it is written and `read` once the agent has
    /// produced anything with it in context — a thought, a tool call, a
    /// word. Taking the wire is not enough: a prompt the model errored on
    /// before it read it is the one case a read tick would lie about.
    unread: Mutex<Vec<String>>,
    /// The window a quiet schedule is holding this teammate's voice with.
    quiet: Mutex<Option<QuietWindow>>,
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
/// driver is handed, which are not always the same.
///
/// A schedule's firing is what forces the words apart — the agent is told
/// which job woke it, and the transcript keeps the bare prompt so the
/// conversation can draw one line instead of a wall.
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
    /// Authorization travels with the queued turn: a one-shot job may have
    /// left the room stream by the time the driver is free to take it.
    scheduled: Option<ScheduledRun>,
    /// Operator input can steer. Internal nudges and scheduled runs queue.
    steer: bool,
    /// The user event this line was written as, so the tape can be told when
    /// the agent has read it. A nudge is never written and has none.
    said: Option<String>,
}

impl Wired {
    fn words(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            attachments: Vec::new(),
            scheduled: None,
            steer: false,
            said: None,
        }
    }
}

/// The lines waiting for a teammate, and whether a driver is already taking
/// them.
///
/// The two are one fact and so they are one lock. Held apart, there was a
/// moment in which the driver had looked at an empty queue and not yet let go
/// of the turn: a line dispatched into it was filed behind a turn that was
/// already over, and the teammate then sat on it until the next thing said
/// shook it loose.
#[derive(Default)]
struct Turns {
    waiting: VecDeque<Wired>,
    running: bool,
}

impl Turns {
    /// Queues the line behind the turn in flight, or claims the driver for it.
    ///
    /// `Some` is the caller's to run: it holds the claim from here until
    /// [`Turns::next_line`] gives it back.
    fn claim(&mut self, wire: Wired) -> Option<Wired> {
        if self.running {
            self.waiting.push_back(wire);
            return None;
        }
        self.running = true;
        Some(wire)
    }

    /// The next line for whoever holds the claim — or, when there is none, the
    /// release of that claim, in the same breath as the look.
    fn next_line(&mut self) -> Option<Wired> {
        let next = self.waiting.pop_front();
        self.running = next.is_some();
        next
    }
}

/// Every session in the room, and the one place their words are written down.
pub struct Room {
    log: Log,
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
    /// A `request_human` wait, by the card's action id. The tool parks on
    /// the oneshot; the person's answer, the deadline, or a settle (session
    /// stop, room restart) is what sends.
    human_waits: Mutex<HashMap<String, HumanWait>>,
    /// The phones paired with this desk, told when a reply lands or a card
    /// needs the person.
    push: crate::push::Push,
    /// One container per teammate, tokens in process state.
    computers: Computer,
    /// Serializes policy changes across sockets, including the interval from
    /// capability invalidation through the durable append and reattach.
    policy_updates: Arc<TokioMutex<()>>,
    /// Work holds a read lease; an installer can pause an idle room atomically.
    activity: Arc<tokio::sync::RwLock<()>>,
}

impl Room {
    pub fn new(log: Log, keys: Arc<dyn ProviderKeys>) -> Arc<Self> {
        let agents = Arc::new(DeskAgents {
            keys: keys.clone(),
            root: log.root().to_path_buf(),
            log: log.clone(),
            mcp_vault: None,
        });
        Self::with_agents(log, keys, agents)
    }

    /// The production room passes its protected vault to both drivers. Tests
    /// use new or with_agents and therefore keep their in-memory seams.
    pub(crate) fn new_with_mcp(
        log: Log,
        keys: Arc<dyn ProviderKeys>,
        vault: Arc<Vault>,
    ) -> Arc<Self> {
        let agents = Arc::new(DeskAgents {
            keys: keys.clone(),
            root: log.root().to_path_buf(),
            log: log.clone(),
            mcp_vault: Some(vault),
        });
        Self::with_agents(log, keys, agents)
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
        let push = crate::push::Push::new(log.root());
        let indexer = match Indexer::open(&log) {
            Ok(indexer) => Some(indexer),
            Err(error) => {
                eprintln!("the search index could not be opened: {error}");
                None
            }
        };
        let room = Arc::new(Self {
            log,
            keys,
            agents,
            indexer: Mutex::new(indexer),
            sessions: Mutex::new(HashMap::new()),
            capabilities: Mutex::new(HashMap::new()),
            lifecycle: Mutex::new(()),
            starts: Mutex::new(HashMap::new()),
            info_changes: broadcast::channel(BROADCAST_DEPTH).0,
            deltas: broadcast::channel(BROADCAST_DEPTH).0,
            schedule_changed: Arc::new(Notify::new()),
            schedule_mutations: Mutex::new(()),
            peers: peers::Peers::default(),
            human_waits: Mutex::new(HashMap::new()),
            push,
            computers,
            policy_updates: Arc::new(TokioMutex::new(())),
            activity: Arc::new(tokio::sync::RwLock::new(())),
        });
        room.settle_tapes();
        sweep_idle_chapters(Arc::downgrade(&room));
        schedule::start(Arc::downgrade(&room), room.schedule_changed.clone());
        room
    }

    fn working(&self) -> Result<tokio::sync::OwnedRwLockReadGuard<()>, String> {
        self.activity.clone().try_read_owned().map_err(|_| {
            "Toad is preparing to restart for an update. Try again when it finishes.".to_string()
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
                .sync(&StreamId::Thread(key))
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

    /// The startup fold and the index, brought in line with the files before
    /// anything is served from them.
    ///
    /// A permission or human-action card left open by the last process is a
    /// button nobody is behind, so it is expired and the stream compacted;
    /// then the index is synced, because the fold just rewrote files and a
    /// tape written by the importer or the previous Toad has never been
    /// indexed here at all.
    ///
    /// Threads are settled with the tapes. A card raised inside a peer turn is
    /// written to the thread and nowhere else, and the resolver behind it only
    /// ever existed in the process that received the request — so a thread
    /// left unfolded draws a live button forever, on a stream nothing else
    /// revisits.
    fn settle_tapes(&self) {
        let now = now_ms();
        let teammates: Vec<String> = room::roster(&self.log)
            .into_iter()
            .map(|persona| persona.id)
            .collect();
        let streams = teammates.iter().cloned().map(StreamId::Tape).chain(
            thread::list_all_keys(self.log.root())
                .into_iter()
                .map(StreamId::Thread),
        );
        for stream in streams {
            for expired in crate::log::expire_orphaned_permissions(&self.log.load(&stream), now) {
                if let Err(error) = self.log.append(&stream, &expired) {
                    eprintln!("could not expire a card left open by the last process: {error}");
                }
            }
            if let Err(error) = self.log.compact(&stream) {
                eprintln!("could not compact a stream the startup fold rewrote: {error}");
            }
        }
        if let Some(indexer) = lock(&self.indexer).as_mut()
            && let Err(error) = indexer.sync(&teammates)
        {
            eprintln!("the search index could not be synced: {error}");
        }
    }

    /// Brings a teammate up, on the driver its backend names. One caller at a
    /// time per teammate; see [`Room::starts`]. A teammate that is already up
    /// is the answer: the second caller — a schedule firing as the window
    /// starts the same teammate — gets the session that exists, not a second
    /// agent on the same tape that only one of them could ever stop.
    pub async fn start(self: &Arc<Self>, persona_id: &str) -> Result<SessionInfo, String> {
        let _working = self.working()?;
        let capability = self.capability_lease(persona_id);
        let gate = self.start_gate(persona_id);
        let _held = gate.lock().await;
        capability.check()?;
        if lock(&self.sessions).contains_key(persona_id) {
            self.session(persona_id)?;
            return Ok(self.info(persona_id));
        }
        self.start_now(persona_id, capability).await
    }

    /// This teammate's start gate, made the first time anyone starts it.
    fn start_gate(&self, persona_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.starts)
            .entry(persona_id.to_string())
            .or_default()
            .clone()
    }

    fn capability_epoch(&self, persona_id: &str) -> CapabilityEpoch {
        lock(&self.capabilities)
            .entry(persona_id.to_string())
            .or_default()
            .clone()
    }

    fn capability_lease(&self, persona_id: &str) -> CapabilityLease {
        self.capability_epoch(persona_id).lease()
    }

    fn current_session(&self, candidate: &Arc<Session>) -> bool {
        lock(&self.sessions)
            .get(&candidate.persona_id)
            .is_some_and(|current| Arc::ptr_eq(current, candidate))
    }

    /// The start itself. The caller is holding this teammate's gate.
    async fn start_now(
        self: &Arc<Self>,
        persona_id: &str,
        capability: CapabilityLease,
    ) -> Result<SessionInfo, String> {
        let _working = self.working()?;
        capability.check()?;
        let persona = self.persona(persona_id)?;
        capability.check()?;
        let in_process = persona.backend_id == TOAD_BACKEND_ID;
        // The directory exists from the moment the teammate can be spoken to.
        // A workspace under the data directory is made here; one the user
        // typed is made too, because a path they chose is a path they meant.
        std::fs::create_dir_all(&persona.cwd).map_err(|error| {
            format!(
                "{}'s working directory {} could not be made: {error}",
                persona.name, persona.cwd
            )
        })?;
        // The skills the teammate may read are files in its workspace, for
        // either driver: the built-ins and whatever the gateway grants it,
        // copied under Toad's marker so a revoked grant leaves nothing of
        // Toad's behind and nothing of the teammate's is ever touched.
        crate::skills::materialize(
            Path::new(&persona.cwd),
            &paths::skills_path(self.log.root()),
            &persona.skill_policy,
        )
        .map_err(|error| format!("{}'s skills could not be written: {error}", persona.name))?;
        capability.check()?;
        if !in_process {
            // An ACP session takes no system prompt, so who the teammate is
            // has to be on disk before the child is started.
            acp::materialize_agents_md_with_capability(&persona, Some(capability.clone()))
                .map_err(|error| {
                    format!("{}'s AGENTS.md could not be written: {error}", persona.name)
                })?;
        }
        // Wake the computer before the grant so a teammate that asked for a
        // machine either has one or the start fails with the runtime's
        // sentence — never a silent absence. The grant itself is appended
        // regardless of mcpPolicy.
        let extra_mcp = self.grant_computer(&persona).await?;
        capability.check()?;
        // The agent's context is one chapter: it hears what was said in the
        // chapter it is joining, and the wake block tells it about the one
        // that closed before it — which is the whole of what a fresh context
        // knows about a conversation that has been going on for months.
        let events = self.tape(&persona.id);
        // Reach is Toad Agent's one policy, and only Toad Agent's: a child
        // brings its own tools and Toad enforces nothing over them, so telling
        // one that a path outside its directory would be refused is a promise
        // nobody here can keep.
        let reach = in_process.then(|| persona.reach.unwrap_or_default());
        let driver = self.agents.agent(
            &persona,
            preamble(&persona, reach, chapters::wake_block(&events, now_ms())),
            said(&events),
            TeammateTools::new(self, &persona.id).with_capability(capability.clone()),
            extra_mcp,
        )?;
        // Subscribe before startup: ACP may publish a picker change between
        // its handshake snapshot and the moment this session enters the room.
        // The receiver keeps that update until the guarded watcher is attached
        // after publication below.
        let info_updates = driver.subscribe_info();
        let reported = match driver.start(&persona).await {
            Ok(reported) => reported,
            Err(error) => {
                driver.invalidate();
                return Err(error);
            }
        };
        if let Err(error) = capability.check() {
            driver.invalidate();
            return Err(error);
        }
        let mut info = idle_info(&persona.id);
        info.state = SessionState::Ready;
        info.agent_name = Some(reported.agent_name);
        info.agent_version = reported.agent_version;
        info.session_id = reported.session_id.clone();
        info.context_restored = reported.context_restored;
        info.models = reported.models;
        info.current_model_id = Some(reported.current_model_id);
        info.model_label = reported.model_label;
        info.modes = reported.modes;
        info.current_mode_id = reported.current_mode_id;
        info.mode_label = reported.mode_label;
        info.configs = reported.configs;
        info.capabilities = reported.capabilities;
        let session = Arc::new(Session {
            persona_id: persona.id.clone(),
            capability: capability.clone(),
            backend_id: persona.backend_id.clone(),
            driver,
            info: Mutex::new(info.clone()),
            turns: Mutex::new(Turns::default()),
            input_ready: Notify::new(),
            pending_reply: Mutex::new(None),
            pending_scheduled: Mutex::new(None),
            unread: Mutex::new(Vec::new()),
            quiet: Mutex::new(None),
            pending_checkpoint: Mutex::new(
                reported.session_id.filter(|_| !reported.context_restored),
            ),
        });
        {
            let _lifecycle = lock(&self.lifecycle);
            if let Err(error) = capability.check() {
                session.driver.invalidate();
                return Err(error);
            }
            lock(&self.sessions).insert(persona.id.clone(), session.clone());
            let _ = self.info_changes.send(info.clone());
        }
        if let Some(info_updates) = info_updates {
            self.watch_driver_info(&session, info_updates);
        }
        // Nothing said is outside a chapter: a session that starts on a tape
        // whose last chapter is closed — or that has none at all — opens one.
        self.begin_chapter(&persona.id, &persona.backend_id);
        // Toad draws the permission cards but does not decide whether the
        // agent sends the requests, and somebody who believes they are behind
        // a gate that is not there should be told.
        if let Some(text) = acp::containment_notice(&persona.backend_id) {
            self.write(
                &persona.id,
                &TranscriptEvent::Notice {
                    id: new_id(),
                    ts: now_ms(),
                    level: NoticeLevel::Warn,
                    text,
                },
            );
        }
        Ok(info)
    }

    /// Starts the teammate's computer when it asked for one, and answers
    /// the MCP server the session should be granted. A failure here is a
    /// start failure: the teammate asked for a machine.
    async fn grant_computer(&self, persona: &Persona) -> Result<Vec<mcp::McpServer>, String> {
        if !persona
            .computer
            .as_ref()
            .is_some_and(|computer| computer.enabled)
        {
            return Ok(Vec::new());
        }
        let settings = room::settings(&self.log);
        let prefer = crate::computer::preferred_runtime(&settings);
        let room_image = crate::computer::preferred_image(&settings);
        let computers = self.computers.clone();
        let persona_id = persona.id.clone();
        let ready = computers
            .ensure_running(
                persona,
                &persona.cwd,
                prefer,
                room_image.as_deref(),
                |text| {
                    self.write(
                        &persona_id,
                        &TranscriptEvent::Notice {
                            id: new_id(),
                            ts: now_ms(),
                            level: NoticeLevel::Info,
                            text: text.to_string(),
                        },
                    );
                },
            )
            .await?;
        // The guide the running release serves is the teammate's
        // `toad-computer` skill: written here, after the container is up and
        // before the preamble is built, so the index lists it and the file it
        // names is the release actually running. A computer that will not
        // hand one over leaves no skill, and the preamble tells the teammate
        // to ask it directly.
        match crate::computer::guide::fetch(&ready).await {
            Ok(guide) => {
                crate::skills::write_computer(
                    Path::new(&persona.cwd),
                    &guide.version,
                    &guide.sha256,
                    &guide.skill,
                )
                .map_err(|error| {
                    format!("{}'s computer guide could not be written: {error}", persona.name)
                })?;
                self.computers.learned_release(&persona.id, &guide.version);
            }
            Err(reason) => self.write(
                &persona.id,
                &TranscriptEvent::Notice {
                    id: new_id(),
                    ts: now_ms(),
                    level: NoticeLevel::Info,
                    text: format!(
                        "The computer did not hand over its guide, so {} will ask it directly. {reason}",
                        persona.name
                    ),
                },
            ),
        }
        Ok(vec![crate::computer::mcp_server(&ready)])
    }

    /// Revokes a teammate's current generation before its replacement policy
    /// is written. Keeping the session record until reattach lets a live
    /// caller restart it, while the inactive epoch blocks starts that race
    /// inside the wire's invalidate-to-append window.
    pub fn invalidate(&self, persona_id: &str) -> Result<(), String> {
        self.persona(persona_id)?;
        {
            let _lifecycle = lock(&self.lifecycle);
            self.capability_epoch(persona_id).invalidate();
            let session = lock(&self.sessions).get(persona_id).cloned();
            if let Some(session) = &session {
                self.revoke_session(session);
            }
        }
        self.drop_peer_sessions(persona_id);
        self.settle_collaboration(persona_id);
        self.settle_permissions(persona_id);
        self.release_human_waits(persona_id);
        self.computers.mark_idle(persona_id, now_ms());
        Ok(())
    }

    /// The caller holds the lifecycle lock and has closed the epoch. Keep
    /// the record for reattachment, but never present revoked execution as
    /// ready if persisting its replacement policy fails.
    fn revoke_session(&self, session: &Arc<Session>) {
        lock(&session.turns).waiting.clear();
        session.driver.invalidate();
        let info = {
            let mut info = lock(&session.info);
            info.state = SessionState::Stopped;
            info.clone()
        };
        let _ = self.info_changes.send(info);
    }

    /// Revokes every live and cached session when room-wide infrastructure
    /// changes. All authorities remain quarantined until `reattach_all`
    /// activates the generations against the new settings.
    pub fn invalidate_all(&self) -> Result<(), String> {
        let roster_ids: Vec<String> = room::roster(&self.log)
            .into_iter()
            .map(|persona| persona.id)
            .collect();
        let sessions = {
            let _lifecycle = lock(&self.lifecycle);
            for persona_id in &roster_ids {
                self.capability_epoch(persona_id).invalidate();
            }
            let sessions: Vec<Arc<Session>> = lock(&self.sessions).values().cloned().collect();
            for session in &sessions {
                self.revoke_session(session);
            }
            sessions
        };
        self.peers.invalidate_all();
        self.settle_all_collaboration();
        for session in sessions {
            self.settle_permissions(&session.persona_id);
            self.release_human_waits(&session.persona_id);
            self.computers.mark_idle(&session.persona_id, now_ms());
        }
        Ok(())
    }

    /// A deleted teammate: its own session stopped, every peer session it was
    /// a side of dropped, and its start gate let go, because nothing should
    /// wait behind — or be kept for — an id that names nobody any more.
    pub fn forget(&self, persona_id: &str) {
        let _ = self.stop(persona_id);
        self.drop_peer_sessions(persona_id);
        lock(&self.capabilities).remove(persona_id);
        lock(&self.starts).remove(persona_id);
        let computers = self.computers.clone();
        let id = persona_id.to_string();
        tokio::spawn(async move {
            let _ = computers.remove(&id, None).await;
        });
    }

    /// Ends the session. The teammate keeps its tape; what stops is the agent.
    pub fn stop(&self, persona_id: &str) -> Result<(), String> {
        self.stop_with_capability(persona_id);
        Ok(())
    }

    /// A chapter restart captures its replacement authority in the same
    /// critical section as stopping the old session, so a later stop also
    /// cancels a replacement that has not reached its first await yet.
    fn stop_with_capability(&self, persona_id: &str) -> CapabilityLease {
        let (session, capability) = {
            let _lifecycle = lock(&self.lifecycle);
            // A stop invalidates this session's handles. Preserve an already
            // quarantined epoch so a concurrent policy update is not reopened
            // by the routine stop path.
            let epoch = self.capability_epoch(persona_id);
            epoch.stop();
            (lock(&self.sessions).remove(persona_id), epoch.lease())
        };
        self.drop_peer_sessions(persona_id);
        let Some(session) = session else {
            self.settle_collaboration(persona_id);
            self.settle_permissions(persona_id);
            self.release_human_waits(persona_id);
            self.computers.mark_idle(persona_id, now_ms());
            return capability;
        };
        lock(&session.turns).waiting.clear();
        session.driver.invalidate();
        self.settle_collaboration(persona_id);
        self.settle_permissions(persona_id);
        self.release_human_waits(persona_id);
        self.computers.mark_idle(persona_id, now_ms());
        let mut info = idle_info(persona_id);
        info.state = SessionState::Stopped;
        let _lifecycle = lock(&self.lifecycle);
        if !lock(&self.sessions).contains_key(persona_id) {
            let _ = self.info_changes.send(info);
        }
        capability
    }

    /// Rebuilds a live session from the teammate's current record, so a change
    /// to what it can use takes effect without waiting for the next start.
    ///
    /// Behind the start gate: two reattaches, or a reattach racing a chapter
    /// swap, would otherwise stop the session the other had just brought up.
    /// An old turn cannot delay the change: its handles are revoked before
    /// replacement, and its remaining updates cannot change the new session.
    pub async fn reattach(self: &Arc<Self>, persona_id: &str) -> Result<(), String> {
        let _working = self.working()?;
        let gate = self.start_gate(persona_id);
        let _held = gate.lock().await;
        // The wire invalidates before appending. Repeating it also gives
        // direct callers the same stale-driver boundary.
        self.invalidate(persona_id)?;
        let capability = {
            let _lifecycle = lock(&self.lifecycle);
            let was_running = lock(&self.sessions).remove(persona_id).is_some();
            let epoch = self.capability_epoch(persona_id);
            epoch.activate();
            self.schedule_changed.notify_one();
            was_running.then(|| epoch.lease())
        };
        let Some(capability) = capability else {
            return Ok(());
        };
        // A restart that fails to start (a key gone, a harness missing) leaves
        // the teammate stopped, and the band has to say
        // why, because nobody pressed Start to be handed the error.
        if let Err(error) = self.start_now(persona_id, capability).await {
            let mut info = idle_info(persona_id);
            info.state = SessionState::Stopped;
            info.error = Some(error.clone());
            let _ = self.info_changes.send(info);
            return Err(error);
        }
        Ok(())
    }

    /// Every live session, because a policy of "all" includes every server
    /// and a narrower one is cheap to restart anyway.
    pub async fn reattach_all(self: &Arc<Self>) -> Result<(), String> {
        let mut ids: HashSet<String> = room::roster(&self.log)
            .into_iter()
            .map(|persona| persona.id)
            .collect();
        ids.extend(lock(&self.sessions).keys().cloned());
        let mut first_error = None;
        for id in ids {
            if let Err(error) = self.reattach(&id).await {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// The session this message goes to, which is not always the one that was
    /// running.
    ///
    /// A chapter closes while nobody is being spoken to — on idle, on request,
    /// or because the agent asked — and the session that belonged to it still
    /// has the whole of it in its context. So the message before it reaches
    /// the agent is where the swap happens: the old session stops, a fresh one
    /// starts, and starting it opens the chapter this message will land in. A
    /// running session with an open chapter is left alone, which is every
    /// message but the first of a chapter.
    ///
    /// The whole of that decision is behind the teammate's start gate. Which
    /// session is running and which chapter it is in are only true together,
    /// and the swap is a stop and a start with a gap in the middle: two
    /// messages arriving on a closed chapter would each perform it, and the
    /// second stop would cancel the session the first had just brought up.
    /// Behind the gate the second message finds the chapter the first opened
    /// and joins it.
    async fn in_this_chapter(self: &Arc<Self>, persona_id: &str) -> Result<Arc<Session>, String> {
        let gate = self.start_gate(persona_id);
        let _held = gate.lock().await;
        let session = self.session(persona_id)?;
        if chapter_view::open_chapter(&self.tape(persona_id)).is_some() {
            return Ok(session);
        }
        let capability = self.stop_with_capability(persona_id);
        self.start_now(persona_id, capability).await?;
        self.session(persona_id)
    }

    /// Hands the teammate a message and returns at once: the turn runs on its
    /// own task and everything it does arrives as tape events and deltas. A
    /// message sent during a turn steers when the driver supports active input.
    ///
    /// `reply_to` is the id of the message this one answers, and the
    /// attachments are files the teammate is handed alongside the words.
    pub async fn prompt(
        self: &Arc<Self>,
        persona_id: &str,
        text: &str,
        reply_to: Option<String>,
        attachments: Option<Vec<Attachment>>,
    ) -> Result<(), String> {
        let _working = self.working()?;
        let session = self.in_this_chapter(persona_id).await?;
        if let Some(answered) = reply_to {
            mark(&session.pending_reply, answered);
        }
        // An empty list is no list: the record should not carry a field
        // saying nothing was attached.
        let attachments = attachments.filter(|attachments| !attachments.is_empty());
        self.say(
            &session,
            Sending {
                shown: text.to_string(),
                wire: Wired {
                    text: text.to_string(),
                    attachments: attachments.clone().unwrap_or_default(),
                    scheduled: None,
                    steer: true,
                    said: None,
                },
                attachments,
            },
        );
        Ok(())
    }

    /// A schedule firing, down the same funnel as everything else — with two
    /// differences the tape can see.
    ///
    /// The agent hears the framed prompt, which says which job woke it; the
    /// transcript keeps the bare prompt and the stamp naming that job, so the
    /// conversation can draw one line instead of a wall. And if the job is
    /// quiet, this is where the window over its turn opens.
    pub async fn prompt_scheduled(
        self: &Arc<Self>,
        persona_id: &str,
        prompt: &str,
        run: ScheduledRun,
    ) -> Result<(), String> {
        let _working = self.working()?;
        let session = self.in_this_chapter(persona_id).await?;
        if !schedule::scheduled_run_allowed(&self.log, persona_id, &run) {
            return Err("Background work is not granted for this teammate.".to_string());
        }
        let mut wire = Wired::words(scheduled_wire_text(&run, prompt));
        wire.scheduled = Some(run.clone());
        mark(&session.pending_scheduled, run);
        self.say(
            &session,
            Sending {
                shown: prompt.to_string(),
                wire,
                attachments: None,
            },
        );
        Ok(())
    }

    /// Toad's own words to a running teammate — a reopened chapter told what
    /// was said while it was away, never something a person typed.
    ///
    /// Queued like a prompt and never written down: the driver hears it, the
    /// record does not.
    pub fn nudge(self: &Arc<Self>, persona_id: &str, text: &str) -> Result<(), String> {
        let _working = self.working()?;
        let session = self.session(persona_id)?;
        self.dispatch(session, Wired::words(text));
        Ok(())
    }

    /// The user's line onto the tape, and then the driver's turn. The order is
    /// the invariant: what was said is a fact the moment somebody said it, and
    /// a turn that fails must not lose the message that started it.
    fn say(self: &Arc<Self>, session: &Arc<Session>, sending: Sending) {
        let id = new_id();
        self.append(
            session,
            TranscriptEvent::User {
                id: id.clone(),
                ts: now_ms(),
                text: sending.shown,
                attachments: sending.attachments,
                reactions: None,
                reply_to: None,
                scheduled: None,
                ring: None,
                receipt: Some(Receipt::Sent),
            },
        );
        let mut wire = sending.wire;
        wire.said = Some(id);
        self.dispatch(session.clone(), wire);
    }

    /// Stamps every line the driver has taken as read, now that the agent has
    /// produced something with it in context. Nothing un-reads.
    fn mark_read(&self, session: &Session) {
        let taken: Vec<String> = std::mem::take(&mut *lock(&session.unread));
        if taken.is_empty() {
            return;
        }
        let tape = self.tape(&session.persona_id);
        for id in taken {
            let Some(line) = tape.iter().rev().find(|event| event["id"] == id.as_str()) else {
                continue;
            };
            let Ok(event) = serde_json::from_value::<TranscriptEvent>(line.clone()) else {
                continue;
            };
            self.write(&session.persona_id, &peers::stamped(event, Receipt::Read));
        }
    }

    /// The agent's reaction to what the person last said: one emoji on that
    /// message, never a message of its own. A reaction the message already
    /// carries is left as it is.
    pub fn react(&self, persona_id: &str, emoji: &str) -> Result<(), String> {
        let emoji = emoji.trim();
        if emoji.is_empty() || emoji.chars().count() > 4 || emoji.chars().any(char::is_alphanumeric)
        {
            return Err("react needs one emoji.".to_string());
        }
        let session = self.session(persona_id)?;
        let tape = self.tape(&session.persona_id);
        let line = tape
            .iter()
            .rev()
            .find(|event| event["kind"] == "user" && event.get("scheduled").is_none())
            .ok_or_else(|| "There is no message from the person to react to.".to_string())?;
        let TranscriptEvent::User {
            id,
            ts,
            text,
            attachments,
            reactions,
            reply_to,
            scheduled,
            ring,
            receipt,
        } = serde_json::from_value::<TranscriptEvent>(line.clone())
            .map_err(|error| format!("The last message could not be read: {error}"))?
        else {
            return Err("There is no message from the person to react to.".to_string());
        };
        let mut reactions = reactions.unwrap_or_default();
        if reactions.iter().any(|had| had == emoji) {
            return Ok(());
        }
        reactions.push(emoji.to_string());
        self.write(
            &session.persona_id,
            &TranscriptEvent::User {
                id,
                ts,
                text,
                attachments,
                reactions: Some(reactions),
                reply_to,
                scheduled,
                ring,
                receipt,
            },
        );
        Ok(())
    }

    /// Hands the driver a line: on the turn in flight if there is one, on a
    /// new turn if there is not.
    fn dispatch(self: &Arc<Self>, session: Arc<Session>, wire: Wired) {
        let working = self.working().expect("dispatch caller holds a work lease");
        // Joining the queue and claiming an idle driver are one decision under
        // one lock, so a line can never be filed behind a turn that has
        // already stopped coming back for it.
        let Some(wire) = lock(&session.turns).claim(wire) else {
            session.input_ready.notify_one();
            return;
        };
        let room = self.clone();
        tokio::spawn(async move {
            let _working = working;
            room.run_turns(session, wire).await;
        });
    }

    /// Stops the turn in flight and drops whatever was waiting behind it.
    ///
    /// A `request_human` the cancelled turn was parked on dies with it: the
    /// tool call is inside the turn, so nothing is reading the answer any
    /// more. The card is superseded and the wait released here, because
    /// otherwise the transcript keeps a live button for ten minutes and
    /// pressing it writes `done` for an agent that stopped listening.
    pub fn cancel(&self, persona_id: &str) -> Result<(), String> {
        let session = self.session(persona_id)?;
        lock(&session.turns).waiting.clear();
        lock(&session.unread).clear();
        session.driver.cancel();
        self.settle_collaboration(persona_id);
        self.settle_permissions(persona_id);
        self.release_human_waits(persona_id);
        Ok(())
    }

    /// Keeps ACP's live picker metadata in the roster without putting a
    /// protocol notification on the tape. Weak references let the watcher
    /// disappear with a stopped session instead of retaining a room/session
    /// cycle while the driver is being torn down.
    fn watch_driver_info(
        self: &Arc<Self>,
        session: &Arc<Session>,
        mut updates: watch::Receiver<DriverInfo>,
    ) {
        let room = Arc::downgrade(self);
        let session = Arc::downgrade(session);
        tokio::spawn(async move {
            while updates.changed().await.is_ok() {
                let Some(room) = room.upgrade() else {
                    break;
                };
                let Some(session) = session.upgrade() else {
                    break;
                };
                let reported = updates.borrow_and_update().clone();
                if room.apply_driver_info(&session, &reported).is_err() {
                    break;
                }
            }
        });
    }

    /// Applies only mutable picker metadata from a driver response. The
    /// lifecycle and lease checks happen while the session is still captured,
    /// so a setter response or ACP notification from a replaced driver cannot
    /// overwrite the current session's state, identity, or checkpoint.
    fn apply_driver_info(
        &self,
        session: &Arc<Session>,
        reported: &DriverInfo,
    ) -> Result<SessionInfo, String> {
        let _lifecycle = lock(&self.lifecycle);
        session.capability.check()?;
        if !lock(&self.sessions)
            .get(&session.persona_id)
            .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            return Err("That session is no longer current.".to_string());
        }
        let info = {
            let mut info = lock(&session.info);
            if info.session_id != reported.session_id {
                *lock(&session.pending_checkpoint) = reported.session_id.clone();
                info.session_id = reported.session_id.clone();
                info.context_restored = reported.context_restored;
            }
            info.models = reported.models.clone();
            info.current_model_id = Some(reported.current_model_id.clone());
            info.model_label = reported.model_label.clone();
            info.modes = reported.modes.clone();
            info.current_mode_id = reported.current_mode_id.clone();
            info.mode_label = reported.mode_label.clone();
            info.configs = reported.configs.clone();
            info.clone()
        };
        let _ = self.info_changes.send(info.clone());
        Ok(info)
    }

    pub async fn set_model(&self, persona_id: &str, model_id: &str) -> Result<SessionInfo, String> {
        let session = self.session(persona_id)?;
        let reported = session.driver.set_model(model_id).await?;
        self.apply_driver_info(&session, &reported)
    }

    pub async fn set_config(
        &self,
        persona_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<SessionInfo, String> {
        let session = self.session(persona_id)?;
        let reported = session.driver.set_config(config_id, value).await?;
        self.apply_driver_info(&session, &reported)
    }

    pub async fn set_mode(&self, persona_id: &str, mode_id: &str) -> Result<SessionInfo, String> {
        let session = self.session(persona_id)?;
        let reported = session.driver.set_mode(mode_id).await?;
        self.apply_driver_info(&session, &reported)
    }

    /// Answers a permission the agent is waiting on.
    ///
    /// Collaboration cards belong to the room, not to a driver. They are
    /// routed here before the normal driver answer path so an ACP child cannot
    /// decide whether another teammate may use its workspace. A standing
    /// grant is appended while the policy lock is held; the lock is acquired
    /// only after the pending wait has been taken, so it is never held while
    /// the delivery waits for the operator.
    pub async fn answer_permission(
        &self,
        persona_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), String> {
        if request_id.starts_with(peers::COLLAB_REQUEST_PREFIX) {
            let Some(wait) = self.take_collaboration_wait(request_id, persona_id) else {
                return Err("That request is no longer waiting for an answer.".to_string());
            };
            let Some((decision, option_name)) =
                peers::collaboration_decision(option_id, &wait.caller_name)
            else {
                self.write_collaboration_decision(&wait, "expired", "Invalid answer");
                let _ = wait.sender.send(peers::CollaborationDecision::Deny);
                return Err("That collaboration card has no such answer.".to_string());
            };

            if wait.caller_capability.check().is_err()
                || wait.target_capability.check().is_err()
                || !self
                    .peers
                    .scope_current(&wait.caller_id, &wait.target_id, wait.scope)
                || wait.sender.is_closed()
            {
                self.write_collaboration_decision(&wait, "expired", "Expired unanswered");
                let _ = wait.sender.send(peers::CollaborationDecision::Deny);
                return Err("That request is no longer waiting for an answer.".to_string());
            }

            if decision == peers::CollaborationDecision::Permanent {
                let policy_updates = self.policy_update_lock();
                let _held = policy_updates.lock().await;
                if wait.caller_capability.check().is_err()
                    || wait.target_capability.check().is_err()
                    || !self
                        .peers
                        .scope_current(&wait.caller_id, &wait.target_id, wait.scope)
                    || wait.sender.is_closed()
                {
                    self.write_collaboration_decision(&wait, "expired", "Expired unanswered");
                    let _ = wait.sender.send(peers::CollaborationDecision::Deny);
                    return Err("That request is no longer waiting for an answer.".to_string());
                }
                if let Err(error) = self.allow_sender(&wait.target_id, &wait.caller_id) {
                    self.write_collaboration_decision(&wait, "expired", "Grant failed");
                    let _ = wait.sender.send(peers::CollaborationDecision::Deny);
                    return Err(error);
                }
            }

            self.write_collaboration_decision(&wait, option_id, &option_name);
            let _ = wait.sender.send(decision);
            return Ok(());
        }

        let session = self.session(persona_id)?;
        if !session.driver.answer_permission(request_id, option_id) {
            return Err("That request is no longer waiting for an answer.".to_string());
        }
        let Some(card) = self.permission_card(persona_id, request_id) else {
            return Ok(());
        };
        let TranscriptEvent::Permission {
            id,
            request_id,
            title,
            options,
            ..
        } = card
        else {
            return Ok(());
        };
        let decided_option_name = options
            .iter()
            .find(|option| option.option_id == option_id)
            .map(|option| option.name.clone());
        self.write(
            persona_id,
            &TranscriptEvent::Permission {
                id,
                ts: now_ms(),
                request_id,
                title,
                options,
                decision: Some(option_id.to_string()),
                decided_option_name,
            },
        );
        Ok(())
    }

    /// Posts a `human_action` card and waits until the person answers it or
    /// `deadline` runs out.
    ///
    /// The wait is a oneshot this room holds by `actionId`. The card is
    /// superseded with the outcome; the sentence the tool returns is what
    /// the agent reads. Tests pass a short deadline; the tool uses
    /// [`HUMAN_DEADLINE`].
    pub async fn request_human(
        &self,
        persona_id: &str,
        reason: &str,
        deadline: Duration,
    ) -> Result<String, String> {
        self.persona(persona_id)?;
        let reason = reason.trim();
        if reason.len() < 3 {
            return Err("request_human needs a `reason` of at least three characters.".to_string());
        }
        let reason: String = reason.chars().take(500).collect();
        let action_id = new_id();
        let (sender, receiver) = oneshot::channel();
        lock(&self.human_waits).insert(
            action_id.clone(),
            HumanWait {
                persona_id: persona_id.to_string(),
                sender,
            },
        );
        self.write(
            persona_id,
            &TranscriptEvent::HumanAction {
                id: format!("human:{action_id}"),
                ts: now_ms(),
                action_id: action_id.clone(),
                reason: reason.clone(),
                status: HumanActionStatus::Pending,
                note: None,
            },
        );
        self.push
            .notify(&self.needs_you(persona_id), &reason, persona_id);
        let answer = tokio::select! {
            answered = receiver => answered.unwrap_or_else(|_| HumanAnswered::expired()),
            _ = tokio::time::sleep(deadline) => {
                self.expire_human(&action_id);
                HumanAnswered::expired()
            }
        };
        Ok(human_outcome(answer))
    }

    /// Resolves a waiting `request_human` and supersedes its card.
    ///
    /// Refused when nothing is behind that id any more — the deadline
    /// passed, the session stopped, or somebody else answered first — so a
    /// stale button cannot quietly settle a wait that is already gone.
    pub fn answer_human(
        &self,
        persona_id: &str,
        action_id: &str,
        status: HumanAnswer,
        note: Option<String>,
    ) -> Result<(), String> {
        let status = match status {
            HumanAnswer::Done => HumanActionStatus::Done,
            HumanAnswer::Declined => HumanActionStatus::Dismissed,
        };
        let note = note
            .map(|note| note.trim().chars().take(2_000).collect::<String>())
            .filter(|note| !note.is_empty());
        let wait = lock(&self.human_waits).remove(action_id);
        let Some(wait) = wait else {
            return Err("That request is no longer waiting for an answer.".to_string());
        };
        if wait.persona_id != persona_id {
            lock(&self.human_waits).insert(action_id.to_string(), wait);
            return Err("That request is no longer waiting for an answer.".to_string());
        }
        self.supersede_human(persona_id, action_id, status, note.clone());
        let _ = wait.sender.send(HumanAnswered { status, note });
        Ok(())
    }

    /// The card this request wrote, read back off the tape it was written to.
    fn permission_card(&self, persona_id: &str, request_id: &str) -> Option<TranscriptEvent> {
        let id = Value::from(format!("perm:{request_id}"));
        self.tape(persona_id)
            .into_iter()
            .find(|event| event.get("id") == Some(&id))
            .and_then(|event| serde_json::from_value(event).ok())
    }

    /// Supersedes every card that still claims to be live.
    ///
    /// A turn that has ended and a session that has stopped are the same fact
    /// for a permission: the agent is no longer waiting, so the button has
    /// nothing behind it and the transcript should not draw one.
    fn settle_permissions(&self, persona_id: &str) {
        for expired in crate::log::expire_orphaned_permissions(&self.tape(persona_id), now_ms()) {
            self.write_value(persona_id, &expired);
        }
    }

    /// Unblocks every `request_human` still waiting for this teammate.
    ///
    /// The cards are already expired by [`Room::settle_permissions`] — the
    /// same fold that expires a permission left open. What remains is the
    /// oneshot the tool is parked on, so it hears expired rather than
    /// hanging until its own deadline.
    fn release_human_waits(&self, persona_id: &str) {
        let senders: Vec<oneshot::Sender<HumanAnswered>> = {
            let mut waits = lock(&self.human_waits);
            let ids: Vec<String> = waits
                .iter()
                .filter(|(_, wait)| wait.persona_id == persona_id)
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| waits.remove(&id).map(|wait| wait.sender))
                .collect()
        };
        for sender in senders {
            let _ = sender.send(HumanAnswered::expired());
        }
    }

    /// The deadline won: take the wait if it is still there and expire the
    /// card. An answer that landed first already removed the wait, so this
    /// is a no-op then.
    fn expire_human(&self, action_id: &str) {
        let Some(wait) = lock(&self.human_waits).remove(action_id) else {
            return;
        };
        self.supersede_human(
            &wait.persona_id,
            action_id,
            HumanActionStatus::Expired,
            None,
        );
    }

    fn supersede_human(
        &self,
        persona_id: &str,
        action_id: &str,
        status: HumanActionStatus,
        note: Option<String>,
    ) {
        let Some(card) = self.human_card(persona_id, action_id) else {
            return;
        };
        let TranscriptEvent::HumanAction { id, reason, .. } = card else {
            return;
        };
        self.write(
            persona_id,
            &TranscriptEvent::HumanAction {
                id,
                ts: now_ms(),
                action_id: action_id.to_string(),
                reason,
                status,
                note,
            },
        );
    }

    fn human_card(&self, persona_id: &str, action_id: &str) -> Option<TranscriptEvent> {
        let id = Value::from(format!("human:{action_id}"));
        self.tape(persona_id)
            .into_iter()
            .find(|event| event.get("id") == Some(&id))
            .and_then(|event| serde_json::from_value(event).ok())
    }

    /// What the teammate's session is doing. A teammate with no session is
    /// idle, which is a state and not an absence.
    pub fn info(&self, persona_id: &str) -> SessionInfo {
        match lock(&self.sessions).get(persona_id) {
            Some(session) => lock(&session.info).clone(),
            None => idle_info(persona_id),
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

    /// What tools this teammate was given the last time it started. `None`
    /// when it has never started under a Toad that keeps a ledger.
    pub fn teammate_tools(&self, persona_id: &str) -> Option<TeammateToolLedger> {
        ledger::teammate_tools(persona_id)
    }

    pub async fn computer_runtimes(&self) -> Vec<RuntimeReport> {
        self.computers.runtimes().await
    }

    pub async fn computer_status(&self, persona_id: &str) -> Result<ComputerStatus, String> {
        let settings = room::settings(&self.log);
        let mut status = self
            .computers
            .status(persona_id, crate::computer::preferred_runtime(&settings))
            .await?;
        // An existing container keeps the release it was made on, so the
        // pane is told when the one it would be made on now is different.
        if let Some(release) = &status.release
            && let Some(persona) = room::roster(&self.log)
                .into_iter()
                .find(|persona| persona.id == persona_id)
        {
            let image = self.computers.image_for(
                &persona,
                crate::computer::preferred_image(&settings).as_deref(),
            );
            let wanted = crate::computer::release_of(&image);
            if wanted != *release {
                status.available = Some(wanted);
            }
        }
        Ok(status)
    }

    /// Recreates a teammate's computer on the release it would be created on
    /// now. The container goes; the workspace, scratch and home volumes stay,
    /// and a teammate that was running is started again on the new one.
    pub async fn computer_update(self: &Arc<Self>, persona_id: &str) -> Result<(), String> {
        let was_live = lock(&self.sessions).contains_key(persona_id);
        let old = self.computer_status(persona_id).await?.release;
        if was_live {
            self.stop(persona_id)?;
        }
        self.computer_remove(persona_id).await?;
        if was_live {
            self.start(persona_id).await?;
        }
        // The release that was running is not coming back; its image is
        // removed unless another container is still on it.
        if let Some(old) = old {
            self.computers
                .forget_image(
                    &crate::computer::image_at(&old),
                    crate::computer::preferred_runtime(&room::settings(&self.log)),
                )
                .await;
        }
        Ok(())
    }

    /// The release a new computer is created on, as the desk knows it now.
    pub fn computer_releases(&self) -> crate::contract::ComputerReleases {
        self.computers.releases_known()
    }

    /// The same, after asking the endpoint now.
    pub async fn computer_releases_check(&self) -> crate::contract::ComputerReleases {
        self.computers.check_releases(now_ms()).await
    }

    pub async fn computer_stop(&self, persona_id: &str) -> Result<(), String> {
        self.computers
            .stop(
                persona_id,
                crate::computer::preferred_runtime(&room::settings(&self.log)),
            )
            .await
    }

    pub async fn computer_remove(&self, persona_id: &str) -> Result<(), String> {
        self.computers
            .remove(
                persona_id,
                crate::computer::preferred_runtime(&room::settings(&self.log)),
            )
            .await
    }

    async fn sweep_computers(&self) {
        let live: HashSet<String> = lock(&self.sessions).keys().cloned().collect();
        self.computers.sweep(now_ms(), &live).await;
        // The newest release is looked up on this clock too: once when the
        // room opens, then every six hours.
        self.computers.refresh_releases(now_ms()).await;
    }

    /// The room's streams, for the teammate tools that read a tape.
    pub(crate) fn log(&self) -> &Log {
        &self.log
    }

    // -- chapters -----------------------------------------------------------

    /// Closes the teammate's open chapter now and answers with what it became.
    ///
    /// Nothing else happens here. The session that belonged to the chapter
    /// keeps running until there is something to say to it, and the swap is
    /// [`Room::prompt`]'s: an agent that asks for a fresh chapter is mid-turn
    /// when it asks, and its own turn is the one that has to finish answering.
    pub async fn start_fresh_chapter(
        self: &Arc<Self>,
        persona_id: &str,
        by: ChapterClose,
    ) -> Result<ChapterSummary, String> {
        self.close_chapter(persona_id, by)
            .await
            .ok_or_else(|| "That teammate has no open chapter to close.".to_string())
    }

    /// Reopens the previous chapter's full context in place of the current one.
    ///
    /// Only the chapter immediately before the open one is offered: a context
    /// from three weeks ago brings back sludge. The current chapter closes as
    /// "Back to: …" without asking a model for a note — it is a turning point,
    /// not a stretch of work — and a new marker opens carrying that earlier
    /// chapter's note under `resumedFrom`. The session is stopped and started
    /// again: Toad Agent is seeded from the previous chapter's tape slice,
    /// an ACP child from the checkpoint the marker still names. The user
    /// lines said in the meantime arrive as a [`Room::nudge`], Toad's words,
    /// never a line of the tape. If the restore itself fails, the new session
    /// still starts, reads the note the wake block already carries, and a
    /// notice says the context could not be reopened.
    pub async fn resume_chapter(
        self: &Arc<Self>,
        persona_id: &str,
    ) -> Result<ChapterSummary, String> {
        let _working = self.working()?;
        let persona = self.persona(persona_id)?;
        let events = self.tape(persona_id);
        let previous = chapter_view::previous_chapter(&events)
            .cloned()
            .ok_or_else(|| "There is no previous chapter to reopen.".to_string())?;
        if previous.get("closedBy").and_then(Value::as_str) == Some("resume") {
            return Err("There is no previous chapter to reopen.".to_string());
        }
        if previous.get("backendId").and_then(Value::as_str) != Some(persona.backend_id.as_str()) {
            return Err(
                "The previous chapter ran on a different agent; its context cannot be reopened here."
                    .to_string(),
            );
        }
        let open = chapter_view::open_chapter(&events).cloned();
        let interim: Vec<String> = open
            .as_ref()
            .map(|open| {
                chapter_view::slice_of(&events, open)
                    .iter()
                    .filter(|event| event.get("kind").and_then(Value::as_str) == Some("user"))
                    .filter_map(|event| {
                        let text = event.get("text")?.as_str()?.trim();
                        (!text.is_empty()).then(|| text.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();
        let title = previous
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or("the previous chapter");

        self.stop(persona_id)?;
        if let Some(open) = open {
            self.close_marker(
                persona_id,
                &open,
                now_ms(),
                chapters::Closing::Titled {
                    title: format!("Back to: {title}"),
                    note: None,
                },
                ChapterClose::Resume,
            );
        }
        // Point the persona's checkpoint back at the previous chapter's
        // session, which is what an ACP child will try to resume. Toad Agent
        // has no checkpoint; its restore is the tape slice `said` will seed.
        {
            let policy_updates = self.policy_update_lock();
            let _held = policy_updates.lock().await;
            if let Some(session_id) = previous.get("sessionId").and_then(Value::as_str) {
                if let Err(error) =
                    room::checkpoint_session(&self.log, persona_id, &persona.backend_id, session_id)
                {
                    eprintln!(
                        "{}'s checkpoint was not pointed back at the previous chapter: {error}",
                        persona.name
                    );
                }
            } else if persona.backend_id != TOAD_BACKEND_ID
                && let Err(error) =
                    room::clear_checkpoint(&self.log, persona_id, &persona.backend_id)
            {
                eprintln!("{}'s checkpoint was not withdrawn: {error}", persona.name);
            }
        }

        let id = new_id();
        let opened = chapters::reopened(&persona.backend_id, id.clone(), now_ms(), &previous)
            .ok_or_else(|| "There is no previous chapter to reopen.".to_string())?;
        self.write(persona_id, &opened);

        match self.start(persona_id).await {
            Ok(info) => {
                if persona.backend_id != TOAD_BACKEND_ID && !info.context_restored {
                    self.write(
                        persona_id,
                        &TranscriptEvent::Notice {
                            id: new_id(),
                            ts: now_ms(),
                            level: NoticeLevel::Warn,
                            text: "The previous chapter's context could not be reopened."
                                .to_string(),
                        },
                    );
                }
            }
            Err(error) => {
                self.write(
                    persona_id,
                    &TranscriptEvent::Notice {
                        id: new_id(),
                        ts: now_ms(),
                        level: NoticeLevel::Warn,
                        text: "The previous chapter's context could not be reopened.".to_string(),
                    },
                );
                return Err(error);
            }
        }
        if let Err(error) = self.nudge(persona_id, &chapters::resume_nudge(&interim)) {
            eprintln!(
                "{} was not told what was said in the meantime: {error}",
                persona.name
            );
        }
        self.chapter_summary(persona_id, &json!({ "id": id }))
            .ok_or_else(|| "The reopened chapter could not be read back.".to_string())
    }

    /// Opens a chapter, unless one is already open.
    fn begin_chapter(&self, persona_id: &str, backend_id: &str) {
        if chapter_view::open_chapter(&self.tape(persona_id)).is_some() {
            return;
        }
        self.write(
            persona_id,
            &chapters::opened(backend_id, new_id(), now_ms()),
        );
    }

    /// Closes the open chapter and writes the note the next one wakes on.
    ///
    /// The note is a model call, so this takes as long as an answer takes, and
    /// the close is one supersession of the marker when it comes back: the
    /// chapter goes from open to everything it turned out to be, in one line.
    /// A chapter in which nothing was said never asks — there is nothing to
    /// write a note about — and closes untitled.
    async fn close_chapter(
        self: &Arc<Self>,
        persona_id: &str,
        by: ChapterClose,
    ) -> Option<ChapterSummary> {
        let _working = self.working().ok()?;
        let events = self.tape(persona_id);
        let open = chapter_view::open_chapter(&events)?.clone();
        // A chapter replacement is a session boundary for collaboration
        // authority even when the existing driver remains alive until the
        // next user line reaches it. Peer sessions are dropped with the
        // chapter so an active or cached conversation cannot keep using the
        // old chapter's temporary authority.
        {
            let _lifecycle = lock(&self.lifecycle);
            self.peers.advance_generation(persona_id);
            self.drop_peer_sessions(persona_id);
        }
        self.settle_collaboration(persona_id);
        let persona = self.persona(persona_id).ok()?;
        let slice = chapter_view::slice_of(&events, &open).to_vec();
        // An idle close ends the chapter when the conversation stopped, not
        // when the sweep noticed — the two can be a night apart, or longer if
        // Toad was closed — so "ended nine hours ago" on wake means what it
        // says.
        let ended_at = match by {
            ChapterClose::Idle => chapter_view::last_activity(&slice)
                .or_else(|| open.get("ts").and_then(Value::as_i64))
                .unwrap_or_else(now_ms),
            _ => now_ms(),
        };
        // Closing a chapter lets go of the agent's own memory of it: the next
        // message starts a context that has never seen it and reads the note
        // instead. The session is not touched; the promise to reopen it is.
        {
            let policy_updates = self.policy_update_lock();
            let _held = policy_updates.lock().await;
            if let Err(error) = room::clear_checkpoint(&self.log, persona_id, &persona.backend_id) {
                eprintln!("{}'s checkpoint was not withdrawn: {error}", persona.name);
            }
        }
        let spoken_in = slice
            .iter()
            .any(chapter_view::is_message)
            .then(|| chapters::fallback_title(&slice));
        let Some(title) = spoken_in else {
            self.close_marker(persona_id, &open, ended_at, chapters::Closing::Empty, by);
            return self.chapter_summary(persona_id, &open);
        };

        let note = self.note(&persona, &slice).await;
        let missing = note.is_none();
        self.close_marker(
            persona_id,
            &open,
            ended_at,
            chapters::Closing::Titled {
                title: note.as_ref().map_or(title, |note| note.title.clone()),
                note,
            },
            by,
        );
        if missing {
            // The chapter closed either way; what is gone is the handoff the
            // next chapter would have woken on, and a person who never hears
            // about it will not know why the next chapter starts cold.
            self.write(
                persona_id,
                &TranscriptEvent::Notice {
                    id: new_id(),
                    ts: now_ms(),
                    level: NoticeLevel::Warn,
                    text: "This chapter closed without a handoff note: no model answered."
                        .to_string(),
                },
            );
        }
        self.chapter_summary(persona_id, &open)
    }

    fn close_marker(
        &self,
        persona_id: &str,
        open: &Value,
        ended_at: i64,
        closing: chapters::Closing,
        by: ChapterClose,
    ) {
        if let Some(closed) = chapters::closed(open, ended_at, closing, by) {
            self.write(persona_id, &closed);
        }
    }

    /// The chapter as the drawer would list it, read back off the tape it was
    /// just written to.
    fn chapter_summary(&self, persona_id: &str, chapter: &Value) -> Option<ChapterSummary> {
        let id = chapter.get("id")?;
        chapter_view::summarize(&self.tape(persona_id))
            .into_iter()
            .find(|summary| summary.get("id") == Some(id))
            .and_then(|summary| serde_json::from_value(summary).ok())
    }

    /// The chapter's note, asked of a model with no tools and no memory.
    ///
    /// `None` is every way this can fail to produce one — no key on the desk,
    /// a provider that refused, an answer that was not the JSON asked for, or
    /// one that never came — because the chapter closes the same way in all of
    /// them.
    async fn note(&self, persona: &Persona, slice: &[Value]) -> Option<chapters::Note> {
        let model_id = self.note_model(persona)?;
        let prompt = format!(
            "Here is the chapter, oldest first.\n{}\nWrite the JSON note now.",
            crate::fence::fenced(
                "toad_chapter_transcript",
                &chapters::serialize_chapter(slice)
            )
        );
        let answer = tokio::time::timeout(
            Duration::from_millis(chapters::ANSWER_MS),
            self.agents
                .complete(&model_id, chapters::INSTRUCTIONS, &prompt),
        )
        .await;
        match answer {
            Ok(Ok(answer)) => chapters::parse_note(&answer),
            Ok(Err(error)) => {
                eprintln!(
                    "the note for {}'s chapter was refused: {error}",
                    persona.name
                );
                None
            }
            Err(_) => {
                eprintln!(
                    "the note for {}'s chapter did not arrive in time",
                    persona.name
                );
                None
            }
        }
    }

    /// The teammate's own model when the desk holds its key, and otherwise the
    /// first model the desk can reach — so a chapter written by a teammate on
    /// a provider nobody has a key for still gets a note.
    fn note_model(&self, persona: &Persona) -> Option<String> {
        let keys = self.keys.provider_auth();
        persona
            .model_id
            .clone()
            .filter(|id| keys.contains_key(id.split('/').next().unwrap_or_default()))
            .or_else(|| {
                crate::models::choices(
                    &keys,
                    &self.keys.enabled_models(),
                    &self.keys.account_models(),
                    &self.keys.model_metadata(),
                )
                .first()
                .map(|model| model.id.clone())
            })
    }

    /// Closes every chapter that has gone quiet for longer than the room
    /// allows. `looked_again` carries the teammates whose chapter was stale
    /// while a turn was running, and when to look at them next.
    async fn sweep_chapters(self: &Arc<Self>, looked_again: &mut HashMap<String, i64>) {
        let idle_ms = chapters::idle_ms(&room::settings(&self.log));
        for persona in room::roster(&self.log) {
            let now = now_ms();
            if looked_again
                .get(&persona.id)
                .is_some_and(|until| now < *until)
            {
                continue;
            }
            let events = self.tape(&persona.id);
            let Some(open) = chapter_view::open_chapter(&events) else {
                continue;
            };
            let last = chapter_view::last_activity(chapter_view::slice_of(&events, open))
                .or_else(|| open.get("ts").and_then(Value::as_i64))
                .unwrap_or(now);
            if now - last < idle_ms {
                continue;
            }
            // A chapter is closed between turns: the note is written from the
            // slice, and a turn still running is still adding to it.
            if matches!(
                self.info(&persona.id).state,
                SessionState::Thinking | SessionState::Starting
            ) {
                looked_again.insert(persona.id.clone(), now + BUSY_RECHECK_MS);
                continue;
            }
            looked_again.remove(&persona.id);
            self.close_chapter(&persona.id, ChapterClose::Idle).await;
        }
    }

    async fn run_turns(self: Arc<Self>, session: Arc<Session>, first: Wired) {
        let mut next = Some(first);
        while let Some(wired) = next.take() {
            if !session.capability.is_current() || !self.current_session(&session) {
                let mut turns = lock(&session.turns);
                turns.waiting.clear();
                turns.running = false;
                break;
            }
            self.set_state(&session, SessionState::Thinking);
            let reach = self.reach_of(&session.persona_id);
            if !session.capability.is_current() || !self.current_session(&session) {
                let mut turns = lock(&session.turns);
                turns.waiting.clear();
                turns.running = false;
                break;
            }
            if wired.scheduled.as_ref().is_some_and(|run| {
                !schedule::scheduled_run_allowed(&self.log, &session.persona_id, run)
            }) {
                next = lock(&session.turns).next_line();
                continue;
            }
            if let Some(said) = wired.said.clone() {
                lock(&session.unread).push(said);
            }
            let mut updates = session
                .driver
                .prompt(wired.text, wired.attachments, reach)
                .await;
            let mut in_flight: HashMap<String, PendingTool> = HashMap::new();
            // What the agent says between its tool calls is held here until
            // the next update says whether it was narration or the report.
            let mut voice = narration::Voice::new();
            let mut asked = false;
            loop {
                self.steer_waiting(&session);
                let update = tokio::select! {
                    biased;
                    () = session.input_ready.notified() => continue,
                    update = updates.recv() => update,
                };
                let Some(update) = update else { break };
                for update in voice.step(update) {
                    if let Update::Chapter { boundary } = update {
                        let gate = self.start_gate(&session.persona_id);
                        let _held = gate.lock().await;
                        if session.capability.is_current() && self.current_session(&session) {
                            self.close_chapter(&session.persona_id, ChapterClose::Agent)
                                .await;
                            let note =
                                chapters::wake_block(&self.tape(&session.persona_id), now_ms());
                            self.begin_chapter(&session.persona_id, &session.backend_id);
                            boundary.finish(note);
                        } else {
                            boundary.finish(None);
                        }
                        continue;
                    }
                    asked |= matches!(update, Update::Permission { .. });
                    self.record(&session, update, &mut in_flight, &voice);
                }
            }
            // A driver that stopped without a turn — its model errored, its
            // child died — leaves a tool spinning in the transcript forever,
            // and a card nobody is behind. A line it was still holding is the
            // last thing it said.
            for update in voice.finish() {
                self.record(&session, update, &mut in_flight, &voice);
            }
            self.fail_in_flight(&session, &mut in_flight);
            if asked {
                // A permission the turn left open is a button nobody is
                // behind. A `request_human` wait is not: the tool is still
                // parked on it, and only the person, the deadline, or a
                // session stop settles that card.
                for expired in crate::log::expire_orphaned_permissions(
                    &self.tape(&session.persona_id),
                    now_ms(),
                ) {
                    if expired.get("kind").and_then(Value::as_str) == Some("permission") {
                        self.write_value(&session.persona_id, &expired);
                    }
                }
            }
            for (text, attachments) in session.driver.take_unconsumed().into_iter().rev() {
                let mut wire = Wired::words(text);
                wire.attachments = attachments;
                lock(&session.turns).waiting.push_front(wire);
            }
            next = lock(&session.turns).next_line();
        }
        if !self.current_session(&session) || !session.capability.is_current() {
            lock(&session.turns).running = false;
            return;
        }
        self.set_state(&session, SessionState::Ready);
    }

    fn steer_waiting(&self, session: &Arc<Session>) {
        if !session.capability.is_current() || !self.current_session(session) {
            return;
        }
        let mut turns = lock(&session.turns);
        let mut steered = false;
        while let Some(index) = turns.waiting.iter().position(|wire| wire.steer) {
            let wire = &turns.waiting[index];
            if !session
                .driver
                .steer(wire.text.clone(), wire.attachments.clone())
            {
                break;
            }
            if let Some(said) = wire.said.clone() {
                lock(&session.unread).push(said);
            }
            turns.waiting.remove(index);
            steered = true;
        }
        drop(turns);
        if steered {
            // A new instruction is neither an answer nor permission. Release
            // an obsolete human wait so the model can reconsider the request.
            self.release_human_waits(&session.persona_id);
        }
    }

    /// One driver update, as the tape and the wire see it.
    fn record(
        &self,
        session: &Session,
        update: Update,
        in_flight: &mut HashMap<String, PendingTool>,
        voice: &narration::Voice,
    ) {
        // Anything the agent produces proves it has what it was handed; a
        // notice can be an error raised before the prompt reached the model.
        if !matches!(update, Update::Notice { .. }) {
            self.mark_read(session);
        }
        if let Update::Delta {
            kind,
            message_id,
            text,
        } = update
        {
            // A muted turn must not run the writing indicator for a message
            // that will never land, so the delta is demoted with the event it
            // is building; and after the acknowledgement a message may turn
            // out to be narration, so it streams as thinking too.
            let muted = kind == MessageKind::Agent
                && (quiet::mutes_deltas(lock(&session.quiet).as_ref(), now_ms())
                    || voice.mutes_deltas());
            let persona_id = session.persona_id.clone();
            let _ = self.deltas.send(match kind {
                MessageKind::Agent if !muted => StreamDelta::AgentDelta {
                    persona_id,
                    message_id,
                    text,
                },
                _ => StreamDelta::ThoughtDelta {
                    persona_id,
                    message_id,
                    text,
                },
            });
            return;
        }
        if matches!(update, Update::Turn { .. }) {
            // A cancelled turn leaves tools running; they are marked before
            // the turn is closed, so the transcript never shows a finished
            // turn above a tool still in progress.
            self.fail_in_flight(session, in_flight);
            if !session.driver.checkpoint_valid()
                || matches!(&update, Update::Turn { stop_reason, .. } if stop_reason == "failed")
            {
                lock(&session.pending_checkpoint).take();
                let _ = room::clear_checkpoint(&self.log, &session.persona_id, &session.backend_id);
            } else {
                self.checkpoint(session);
            }
        }
        // A phone hears a reply the moment it lands, and a question the
        // agent cannot go on without. A quiet schedule's reply is demoted to
        // a thought and says nothing anywhere.
        let glance = match &update {
            Update::Message {
                kind: MessageKind::Agent,
                text,
                ..
            } if !quiet::mutes_deltas(lock(&session.quiet).as_ref(), now_ms()) => Some((
                self.persona(&session.persona_id)
                    .map(|persona| persona.name)
                    .unwrap_or_else(|_| "Toad".to_string()),
                text.clone(),
            )),
            Update::Permission { title, .. } => {
                Some((self.needs_you(&session.persona_id), title.clone()))
            }
            _ => None,
        };
        for event in event_of(update, in_flight) {
            self.append(session, event);
        }
        if let Some((title, body)) = glance {
            self.push.notify(&title, &body, &session.persona_id);
        }
    }

    /// The title of a notification about a card: the teammate needs you.
    fn needs_you(&self, persona_id: &str) -> String {
        match self.persona(persona_id) {
            Ok(persona) => format!("{} needs you", persona.name),
            Err(_) => "Toad needs you".to_string(),
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
        let events = self.tape(persona_id);
        let Some(open) = chapter_view::open_chapter(&events) else {
            return;
        };
        if open.get("sessionId").and_then(Value::as_str) == Some(session_id) {
            return;
        }
        let mut stamped = open.clone();
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
    /// stamps a prompt left and the quiet window be stated once for both kinds
    /// of agent.
    fn append(&self, session: &Session, event: TranscriptEvent) {
        self.write(&session.persona_id, &stamped(session, event, now_ms()));
    }

    /// One event onto the tape and into the index, with nothing stamped on it.
    ///
    /// A chapter marker and the notice that a note is missing come this way:
    /// they are the room writing in its own voice, not a teammate speaking,
    /// and there is no reply, schedule or silence for them to be part of.
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
        if let Err(error) = self
            .log
            .append(&StreamId::Tape(persona_id.to_string()), event)
        {
            eprintln!("the tape for {persona_id} could not be appended to: {error}");
            return;
        }
        self.index(persona_id, event);
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
            info.clone()
        };
        let _ = self.info_changes.send(info);
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

/// A `request_human` wait the room holds until the person answers, the
/// deadline passes, or the session stops.
struct HumanWait {
    persona_id: String,
    sender: oneshot::Sender<HumanAnswered>,
}

/// What came back for a `request_human` card: the outcome, and the words
/// the person added to it, if any.
struct HumanAnswered {
    status: HumanActionStatus,
    note: Option<String>,
}

impl HumanAnswered {
    fn expired() -> Self {
        Self {
            status: HumanActionStatus::Expired,
            note: None,
        }
    }
}

/// The sentence the tool returns. The person's note, when there is one,
/// follows it word for word, so a card that asked a question gets its
/// answer. The expired wording always says ten minutes, even when a test
/// injected a shorter deadline: that is what the agent is told in life,
/// and a test that checks the sentence should see the same words.
fn human_outcome(answer: HumanAnswered) -> String {
    let outcome = match answer.status {
        HumanActionStatus::Done => "The person did it.",
        HumanActionStatus::Dismissed => "The person declined.",
        HumanActionStatus::Expired | HumanActionStatus::Pending => {
            return "Nobody answered in ten minutes.".to_string();
        }
    };
    match answer.note {
        Some(note) => format!("{outcome} They said: {note}"),
        None => outcome.to_string(),
    }
}

/// The idle clock, for the whole room at once.
///
/// One task rather than a timer per teammate: the tape already knows when each
/// chapter last heard anything, so there is nothing to arm when a message
/// lands, nothing to cancel when a teammate is deleted, and nothing to rebuild
/// when the setting changes. The first look is a few seconds after the room
/// opens, which is where a chapter that went stale while Toad was closed is
/// closed — nobody is waiting, so the note is written before anyone comes back
/// to read it. The task holds the room weakly, so it is the last thing the
/// room's own end stops.
fn sweep_idle_chapters(room: Weak<Room>) {
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_SWEEP).await;
        let mut looked_again: HashMap<String, i64> = HashMap::new();
        loop {
            match room.upgrade() {
                Some(room) => {
                    room.sweep_chapters(&mut looked_again).await;
                    // The same clock, because a peer session that has gone
                    // quiet is the same kind of fact as a chapter that has:
                    // nothing to arm when a message lands and nothing to
                    // cancel when a teammate is deleted.
                    room.sweep_peers(now_ms());
                    room.sweep_computers().await;
                }
                None => return,
            }
            tokio::time::sleep(SWEEP_EVERY).await;
        }
    });
}

/// What the teammate and its agent have said to each other in the chapter the
/// agent is joining, for a driver that starts back into the conversation.
///
/// A chapter is one working context, so a session hears its own chapter and
/// not the ones before it: the wake block is what carries those. A tape with
/// no marker at all was written before this room divided anything, and reads
/// as one implicit chapter. A chapter that reopened an earlier one is that
/// earlier stretch plus anything said since, because Toad Agent has no
/// checkpoint and the tape is its memory of the work.
///
/// The model said one thing; the tape may show it as several bubbles. Consecutive
/// agent events collapse back into one [`Said::Agent`], so the model sees one
/// thing again. The Rig history is built from this, not from the tape, so it
/// does not need a second fold.
fn said(events: &[Value]) -> Vec<Said> {
    let within: Vec<&Value> = match chapter_view::open_chapter(events) {
        Some(open) => {
            let mut lines = Vec::new();
            if let Some(from) = open.get("resumedFrom").and_then(Value::as_str)
                && let Some(origin) = events
                    .iter()
                    .find(|event| event.get("id").and_then(Value::as_str) == Some(from))
            {
                lines.extend(chapter_view::slice_of(events, origin));
            }
            lines.extend(chapter_view::slice_of(events, open));
            lines
        }
        None if chapter_view::chapters_of(events).is_empty() => events.iter().collect(),
        None => Vec::new(),
    };
    fold_said(within.iter().filter_map(|event| {
        let text = event.get("text")?.as_str()?;
        match event.get("kind")?.as_str()? {
            "user" => Some(Said::User(text.to_string())),
            "agent" => Some(Said::Agent(text.to_string())),
            _ => None,
        }
    }))
}

/// Consecutive agent events are one thing the model said, shown as several
/// bubbles. Fold here so a teammate's tape and a peer thread put the pieces
/// back the same way.
fn fold_said(lines: impl IntoIterator<Item = Said>) -> Vec<Said> {
    let mut out = Vec::new();
    for line in lines {
        match (out.last_mut(), line) {
            (Some(Said::Agent(already)), Said::Agent(next)) => {
                already.push_str("\n\n");
                already.push_str(&next);
            }
            (_, line) => out.push(line),
        }
    }
    out
}

/// What one driver update is, written down.
///
/// The one place an update becomes events, because a teammate's tape and a
/// peer thread must record the same turn the same way — the shapes are the
/// previous Toad's, and there is nowhere for a second copy of them to drift
/// to. An empty vec is the one update that is never written: a delta, which
/// the message that follows it makes durable. An agent's message is split
/// into bubbles here so both kinds of agent and a peer thread get the same
/// ones.
fn event_of(update: Update, in_flight: &mut HashMap<String, PendingTool>) -> Vec<TranscriptEvent> {
    match update {
        Update::Chapter { boundary } => {
            boundary.finish(None);
            Vec::new()
        }
        Update::Delta { .. } => Vec::new(),
        Update::Message { kind, id, text } => match kind {
            MessageKind::Agent => {
                let ts = now_ms();
                pacing::paced(&text)
                    .into_iter()
                    .enumerate()
                    .map(|(i, text)| TranscriptEvent::Agent {
                        id: if i == 0 {
                            id.clone()
                        } else {
                            format!("{id}-{}", i + 1)
                        },
                        ts,
                        text,
                        reactions: None,
                        ring: None,
                        receipt: None,
                    })
                    .collect()
            }
            MessageKind::Thought => vec![TranscriptEvent::Thought {
                id,
                ts: now_ms(),
                text,
            }],
        },
        Update::ToolCall {
            call_id,
            title,
            kind,
        } => {
            let pending = PendingTool {
                ts: now_ms(),
                title,
                kind,
            };
            let event = pending.event(&call_id, ToolStatus::InProgress, None);
            in_flight.insert(call_id, pending);
            vec![event]
        }
        Update::ToolResult {
            call_id,
            ok,
            output,
            images,
        } => {
            let Some(pending) = in_flight.remove(&call_id) else {
                return Vec::new();
            };
            let status = if ok {
                ToolStatus::Completed
            } else {
                ToolStatus::Failed
            };
            let output = ToolOutput::Text {
                text: clip(&output, TOOL_OUTPUT_CHARS),
            };
            let mut events = vec![pending.event(&call_id, status, Some(output))];
            if crate::computer::is_computer_tool(&pending.kind, &pending.title) {
                for image in images {
                    events.push(TranscriptEvent::ComputerFrame {
                        id: new_id(),
                        ts: now_ms(),
                        data_url: image.data_url(),
                    });
                }
            }
            events
        }
        Update::Permission {
            request_id,
            title,
            options,
        } => vec![TranscriptEvent::Permission {
            // The card is superseded by this id when it is answered, so the
            // decision lands on the line already drawn rather than adding a
            // second one below it.
            id: format!("perm:{request_id}"),
            ts: now_ms(),
            request_id,
            title,
            options,
            decision: None,
            decided_option_name: None,
        }],
        Update::Turn { stop_reason, usage } => vec![TranscriptEvent::Turn {
            id: new_id(),
            ts: now_ms(),
            stop_reason,
            usage,
        }],
        Update::Notice { level, text } => vec![TranscriptEvent::Notice {
            id: new_id(),
            ts: now_ms(),
            level,
            text,
        }],
    }
}

/// A tool call the agent has made and not yet heard back about. The tape event
/// is written again by id when it does, so the call's own timestamp and title
/// have to outlive the call.
struct PendingTool {
    ts: i64,
    title: String,
    kind: String,
}

impl PendingTool {
    fn event(
        &self,
        call_id: &str,
        status: ToolStatus,
        output: Option<ToolOutput>,
    ) -> TranscriptEvent {
        TranscriptEvent::Tool {
            id: format!("tool:{call_id}"),
            ts: self.ts,
            tool_call_id: call_id.to_string(),
            title: self.title.clone(),
            tool_kind: Some(self.kind.clone()),
            status,
            locations: None,
            output: output.map(|output| vec![output]),
        }
    }
}

/// What the tape writes down in place of the event the room handed it.
///
/// Order matters: a new speaker closes any window that is open, and only then
/// may a scheduled firing open one of its own.
fn stamped(session: &Session, mut event: TranscriptEvent, now: i64) -> TranscriptEvent {
    if let TranscriptEvent::User { reply_to, .. } = &mut event {
        *reply_to = take_fresh(&session.pending_reply, now);
    }
    let event = through_quiet(session, event, now);
    stamp_scheduled(session, event, now)
}

/// Runs an event past an open quiet window, which may rewrite it or close.
fn through_quiet(session: &Session, event: TranscriptEvent, now: i64) -> TranscriptEvent {
    let mut held = lock(&session.quiet);
    let Some(window) = held.take() else {
        return event;
    };
    let (window, event) = quiet::step(window, event, now);
    *held = window;
    event
}

/// Claims a pending firing for the user line it woke, opening its silence.
fn stamp_scheduled(session: &Session, mut event: TranscriptEvent, now: i64) -> TranscriptEvent {
    if let TranscriptEvent::User { scheduled, .. } = &mut event
        && let Some(run) = take_fresh(&session.pending_scheduled, now)
    {
        // A firing that lands mid-turn is queued behind the turn already
        // running, so the first boundary to arrive belongs to that turn and
        // not to this one. The turn in flight is what `running` says, which is
        // set as the line is dispatched — after this line reaches the tape.
        let busy = lock(&session.turns).running;
        *lock(&session.quiet) = quiet::open_window(&run, busy, now);
        *scheduled = Some(run);
    }
    event
}

/// The framing the agent reads when a schedule wakes it.
///
/// Deliberately silent about `quiet`: the window in [`quiet`] does not need
/// the agent's cooperation, and asking for it is exactly how "No change —
/// staying silent per protocol" ended up in someone's chat.
fn scheduled_wire_text(run: &ScheduledRun, prompt: &str) -> String {
    let waking = match run.kind {
        ScheduleKind::Loop => "loop",
        ScheduleKind::Schedule => "scheduled",
    };
    format!("{waking} · {prompt}")
}

/// What the agent is told before it is told anything else: who it is, where it
/// stands, how far it can reach, what day it is, how to talk in this room, and
/// — when it is joining a conversation that already has chapters behind it —
/// what happened in the one that closed. Everything here is something it would
/// otherwise have to ask for or guess. Both kinds of agent hear this, so the
/// house style is not a second briefing an ACP child gets and Toad Agent does
/// not.
pub(crate) fn preamble(persona: &Persona, reach: Option<Reach>, wake: Option<String>) -> String {
    // An ACP harness manages its own tools; only file operations it delegates
    // to Toad share Toad's workspace boundary.
    let reach_sentence = match reach {
        Some(Reach::Workspace) => {
            if cfg!(target_os = "linux") {
                " Your built-in file tools stay in that directory: a path that leaves it is refused, except for your saved tool output. The shell also has read-only installed tools and a private home at .toad-home inside the workspace; other host files are hidden. Granted integrations have their own permissions."
            } else {
                " Your built-in file tools stay in that directory: a path that leaves it is refused, except for your saved tool output. Shell restrictions depend on the operating system; consult its tool description. Granted integrations have their own permissions."
            }
        }
        Some(Reach::Machine) => " Your tools reach the whole machine, not only that directory.",
        None => {
            " Your harness manages the permissions of its own tools. File operations delegated to Toad through ACP stay inside your working directory."
        }
    };
    // A computer is granted at start, outside the policy, so the agent is told
    // here rather than by a tool listing: what the desktop is, that the person
    // can watch it and take it over, and what to do when a page wants
    // something only the person has. What they type on the desktop goes
    // straight to the desktop and never through the agent.
    let computer_sentence = if persona
        .computer
        .as_ref()
        .is_some_and(|computer| computer.enabled)
    {
        if crate::skills::computer_entry(Path::new(&persona.cwd))
            .is_some_and(|entry| entry.invalid.is_none())
        {
            "\n\nYou have a computer: a Linux desktop of your own, with a browser, driven with the `computer__` tools. Its guide for the release it is running is your `toad-computer` skill; read it before your first call on the computer. Use its managed shell jobs and prepared workspaces for installs, builds, and app launches. The person can see its screen and take it over at any time. When a page wants credentials, a 2FA tap or a CAPTCHA, get that page on screen first, then call `request_human` and say exactly what to do; they act on your desktop directly, and whatever they type never passes through you. When the call returns, look at the screen again before going on."
        } else {
            "\n\nYou have a computer: a Linux desktop of your own, with a browser, driven with the `computer__` tools. On connection, read `computer__state` with action `info` and action `guide` to load the skill and environment catalog from that running computer. Use its managed shell jobs and prepared workspaces for installs, builds, and app launches. If an older image does not offer those actions, use its advertised tool schemas. The person can see its screen and take it over at any time. When a page wants credentials, a 2FA tap or a CAPTCHA, get that page on screen first, then call `request_human` and say exactly what to do; they act on your desktop directly, and whatever they type never passes through you. When the call returns, look at the screen again before going on."
        }
    } else {
        ""
    };
    let goal = persona.goal.trim();
    let identity = if goal.is_empty() {
        format!("You are {}.", persona.name)
    } else {
        format!(
            "You are {}. You were created for this:\n\n{goal}",
            persona.name
        )
    };
    // Every teammate has Toad's own tools — over its conversation and over
    // the room — on either driver, so the sentence about them is
    // unconditional: a tool an agent was never told about is a tool it does
    // not have.
    // Skills are the one thing here the agent is told about rather than told:
    // a line each, and the body is read when a task calls for it. The index
    // is what is in the workspace — the built-ins are always there, so it is
    // never empty — and a skill the teammate wrote over a built-in's name is
    // the one listed, because it is the one on disk.
    // The computer's guide is listed only for a teammate that has the
    // computer: it is written by that teammate's grant, and a colleague
    // sharing the working directory has no use for a line about a machine it
    // cannot drive.
    let has_computer = persona
        .computer
        .as_ref()
        .is_some_and(|computer| computer.enabled);
    let listed: Vec<_> = crate::skills::visible(Path::new(&persona.cwd))
        .into_iter()
        .filter(|skill| has_computer || skill.name != crate::skills::COMPUTER)
        .collect();
    let skills = crate::skills::index(&listed);
    let standing = format!(
        "{identity}\n\nYour working directory is {}.{reach_sentence}{computer_sentence}\n\nToday is {}.\n\n{}\n\n{skills}\n\n{}",
        persona.cwd,
        Local::now().format("%A %-d %B %Y"),
        crate::mcp::server::HOW_TO_USE,
        pacing::HOUSE_STYLE,
    );
    match wake {
        Some(wake) => format!("{standing}\n\n{wake}"),
        None => standing,
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

#[cfg(test)]
mod tests;
