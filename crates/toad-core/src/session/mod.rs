//! The room's live sessions: one conversation per teammate, and the funnel
//! every word of it passes through.
//!
//! A [`Driver`] runs the turn and knows nothing else. Everything that makes a
//! turn part of a room happens here, in one place, for both kinds of agent:
//!
//! - The user's line is on the tape **before** the driver sees it. What was
//!   said is a fact the moment somebody said it, and a turn that fails must
//!   not lose the message that started it.
//! - Every driver update becomes exactly one tape event, in the shapes the
//!   previous Toad wrote — [`crate::contract::TranscriptEvent`] pins them, and
//!   a tape written here opens in that app unchanged.
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
//! Reach is read from the roster at every prompt rather than from the persona
//! the session started with: a live session is not told when its teammate is
//! edited, and the switch has to take on the next turn.

mod chapters;
pub(crate) mod ledger;
mod quiet;
pub(crate) mod schedule;

pub use schedule::{parse_duration, parse_when};

use crate::contract::{
    Attachment, ChapterClose, ChapterSummary, ConfigChoice, NoticeLevel, Persona, Reach,
    ScheduleKind, ScheduledRun, SessionCapabilities, SessionInfo, SessionState, StreamDelta,
    TeammateToolLedger, ToolOutput, ToolStatus, TranscriptEvent,
};
use crate::driver::acp::{self, ChildAgent};
use crate::driver::rig;
use crate::driver::rig::{InProcess, Said, models};
use crate::driver::{Driver, MessageKind, PI_BACKEND_ID, Update, clip};
use crate::log::{Log, StreamId};
use crate::mcp;
use crate::room;
use crate::store::chapters as chapter_view;
use crate::store::search::Indexer;
use async_trait::async_trait;
use chrono::Local;
use quiet::QuietWindow;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;
use tokio::sync::{Notify, broadcast};

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

/// Where the provider keys come from.
///
/// The room never holds a secret: it asks for the keys at the start of every
/// turn, so a key added or rotated on the desk is in force on the next one and
/// nothing keeps a stale copy. The vault implements this; a test hands over a
/// map.
pub trait ProviderKeys: Send + Sync {
    fn provider_keys(&self) -> HashMap<String, String>;
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
    /// preamble and seeded with what has been said in the chapter it is
    /// joining. A backend this desk cannot run is refused here, in a sentence
    /// naming what is missing.
    fn agent(
        &self,
        persona: &Persona,
        preamble: String,
        said: Vec<Said>,
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
}

#[async_trait]
impl Agents for DeskAgents {
    fn agent(
        &self,
        persona: &Persona,
        preamble: String,
        said: Vec<Said>,
    ) -> Result<Arc<dyn Driver>, String> {
        if persona.backend_id == PI_BACKEND_ID {
            let grant = mcp::grant(
                &mcp::servers(&room::settings(&self.log)),
                &persona.mcp_policy,
            );
            return Ok(Arc::new(
                InProcess::new(
                    self.keys.clone(),
                    preamble,
                    said,
                    self.root.join("tool-output").join(&persona.id),
                )
                .with_mcp(grant.servers, grant.missing),
            ));
        }
        // The registry answers whether this machine can start that harness at
        // all, and says what is missing when it cannot.
        acp::registry::launch(&self.root, &persona.backend_id)?;
        Ok(Arc::new(ChildAgent::new(
            self.root.clone(),
            persona.backend_id.clone(),
            preamble,
        )))
    }

    async fn complete(&self, model_id: &str, system: &str, prompt: &str) -> Result<String, String> {
        rig::complete(&self.keys.provider_keys(), model_id, system, prompt).await
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
    /// Which agent is answering, because a checkpoint is kept per backend and
    /// the tape's chapter markers name the one that wrote them.
    backend_id: String,
    driver: Arc<dyn Driver>,
    info: Mutex<SessionInfo>,
    /// Lines that arrived while a turn was running, as the driver will hear
    /// them. One turn at a time: a redirect waits for the turn it would have
    /// interrupted.
    queue: Mutex<VecDeque<Wired>>,
    running: Mutex<bool>,
    /// The message the next user line answers.
    pending_reply: Mutex<Option<Mark<String>>>,
    /// The firing the next user line belongs to.
    pending_scheduled: Mutex<Option<Mark<ScheduledRun>>>,
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
}

impl Wired {
    fn words(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            attachments: Vec::new(),
        }
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
    info_changes: broadcast::Sender<SessionInfo>,
    deltas: broadcast::Sender<StreamDelta>,
    /// Wakes the scheduler when a job is written, so a create does not wait
    /// for the nearest existing nextAt.
    schedule_changed: Arc<Notify>,
}

impl Room {
    pub fn new(log: Log, keys: Arc<dyn ProviderKeys>) -> Arc<Self> {
        let agents = Arc::new(DeskAgents {
            keys: keys.clone(),
            root: log.root().to_path_buf(),
            log: log.clone(),
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
            info_changes: broadcast::channel(BROADCAST_DEPTH).0,
            deltas: broadcast::channel(BROADCAST_DEPTH).0,
            schedule_changed: Arc::new(Notify::new()),
        });
        room.settle_tapes();
        sweep_idle_chapters(Arc::downgrade(&room));
        schedule::start(Arc::downgrade(&room), room.schedule_changed.clone());
        room
    }

    /// The startup fold and the index, brought in line with the files before
    /// anything is served from them.
    ///
    /// A permission card left open by the last process is a button nobody is
    /// behind, so it is expired and the tape compacted; then the index is
    /// synced, because the fold just rewrote files and a tape written by the
    /// importer or the previous Toad has never been indexed here at all.
    fn settle_tapes(&self) {
        let now = now_ms();
        let teammates: Vec<String> = room::roster(&self.log)
            .into_iter()
            .map(|persona| persona.id)
            .collect();
        for persona_id in &teammates {
            let stream = StreamId::Tape(persona_id.clone());
            for expired in crate::log::expire_orphaned_permissions(&self.log.load(&stream), now) {
                if let Err(error) = self.log.append(&stream, &expired) {
                    eprintln!("could not expire a card on {persona_id}'s tape: {error}");
                }
            }
            if let Err(error) = self.log.compact(&stream) {
                eprintln!("could not compact {persona_id}'s tape: {error}");
            }
        }
        if let Some(indexer) = lock(&self.indexer).as_mut()
            && let Err(error) = indexer.sync(&teammates)
        {
            eprintln!("the search index could not be synced: {error}");
        }
    }

    /// Brings a teammate up, on the driver its backend names.
    pub async fn start(self: &Arc<Self>, persona_id: &str) -> Result<SessionInfo, String> {
        let persona = self.persona(persona_id)?;
        let in_process = persona.backend_id == PI_BACKEND_ID;
        // The directory exists from the moment the teammate can be spoken to.
        // A workspace under the data directory is made here; one the user
        // typed is made too, because a path they chose is a path they meant.
        std::fs::create_dir_all(&persona.cwd).map_err(|error| {
            format!(
                "{}'s working directory {} could not be made: {error}",
                persona.name, persona.cwd
            )
        })?;
        if !in_process {
            // An ACP session takes no system prompt, so who the teammate is
            // has to be on disk before the child is started.
            acp::materialize_agents_md(&persona).map_err(|error| {
                format!("{}'s AGENTS.md could not be written: {error}", persona.name)
            })?;
        }
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
        )?;
        let reported = driver.start(&persona).await?;
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
        info.capabilities = reported.capabilities;
        let session = Arc::new(Session {
            persona_id: persona.id.clone(),
            backend_id: persona.backend_id.clone(),
            driver,
            info: Mutex::new(info.clone()),
            queue: Mutex::new(VecDeque::new()),
            running: Mutex::new(false),
            pending_reply: Mutex::new(None),
            pending_scheduled: Mutex::new(None),
            quiet: Mutex::new(None),
            pending_checkpoint: Mutex::new(
                reported.session_id.filter(|_| !reported.context_restored),
            ),
        });
        lock(&self.sessions).insert(persona.id.clone(), session);
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
        let _ = self.info_changes.send(info.clone());
        Ok(info)
    }

    /// Ends the session. The teammate keeps its tape; what stops is the agent.
    pub fn stop(&self, persona_id: &str) -> Result<(), String> {
        let Some(session) = lock(&self.sessions).remove(persona_id) else {
            return Ok(());
        };
        session.driver.cancel();
        self.settle_permissions(persona_id);
        let mut info = idle_info(persona_id);
        info.state = SessionState::Stopped;
        let _ = self.info_changes.send(info);
        Ok(())
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
    async fn in_this_chapter(self: &Arc<Self>, persona_id: &str) -> Result<Arc<Session>, String> {
        let session = self.session(persona_id)?;
        if chapter_view::open_chapter(&self.tape(persona_id)).is_some() {
            return Ok(session);
        }
        self.stop(persona_id)?;
        self.start(persona_id).await?;
        self.session(persona_id)
    }

    /// Hands the teammate a message and returns at once: the turn runs on its
    /// own task and everything it does arrives as tape events and deltas. A
    /// message sent during a turn is queued behind it.
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
        let session = self.in_this_chapter(persona_id).await?;
        let wire = Wired::words(scheduled_wire_text(&run, prompt));
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
        let session = self.session(persona_id)?;
        self.dispatch(session, Wired::words(text));
        Ok(())
    }

    /// The user's line onto the tape, and then the driver's turn. The order is
    /// the invariant: what was said is a fact the moment somebody said it, and
    /// a turn that fails must not lose the message that started it.
    fn say(self: &Arc<Self>, session: &Arc<Session>, sending: Sending) {
        self.append(
            session,
            TranscriptEvent::User {
                id: new_id(),
                ts: now_ms(),
                text: sending.shown,
                attachments: sending.attachments,
                reactions: None,
                reply_to: None,
                scheduled: None,
                ring: None,
                receipt: None,
            },
        );
        self.dispatch(session.clone(), sending.wire);
    }

    /// Hands the driver a line: on the turn in flight if there is one, on a
    /// new turn if there is not.
    fn dispatch(self: &Arc<Self>, session: Arc<Session>, wire: Wired) {
        let running_already = {
            let mut running = lock(&session.running);
            let was = *running;
            *running = true;
            was
        };
        if running_already {
            lock(&session.queue).push_back(wire);
            return;
        }
        let room = self.clone();
        tokio::spawn(async move { room.run_turns(session, wire).await });
    }

    /// Stops the turn in flight and drops whatever was waiting behind it.
    pub fn cancel(&self, persona_id: &str) -> Result<(), String> {
        let session = self.session(persona_id)?;
        lock(&session.queue).clear();
        session.driver.cancel();
        Ok(())
    }

    pub async fn set_model(&self, persona_id: &str, model_id: &str) -> Result<SessionInfo, String> {
        let session = self.session(persona_id)?;
        let reported = session.driver.set_model(model_id).await?;
        let info = {
            let mut info = lock(&session.info);
            info.models = reported.models;
            info.current_model_id = Some(reported.current_model_id);
            info.model_label = reported.model_label;
            info.clone()
        };
        let _ = self.info_changes.send(info.clone());
        Ok(info)
    }

    pub async fn set_mode(&self, persona_id: &str, mode_id: &str) -> Result<SessionInfo, String> {
        let session = self.session(persona_id)?;
        let reported = session.driver.set_mode(mode_id).await?;
        let info = {
            let mut info = lock(&session.info);
            info.modes = reported.modes;
            info.current_mode_id = reported.current_mode_id;
            info.mode_label = reported.mode_label;
            info.clone()
        };
        let _ = self.info_changes.send(info.clone());
        Ok(info)
    }

    /// Answers a permission the agent is waiting on.
    ///
    /// The driver is asked first, because it is the only thing that knows
    /// whether anything is still behind that request; only then is the card
    /// superseded, so the transcript never shows a decision the agent never
    /// heard.
    pub fn answer_permission(
        &self,
        persona_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), String> {
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
        models(&self.keys.provider_keys())
    }

    /// What tools this teammate was given the last time it started. `None`
    /// when it has never started under a Toad that keeps a ledger.
    pub fn teammate_tools(&self, persona_id: &str) -> Option<TeammateToolLedger> {
        ledger::teammate_tools(persona_id)
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
        let events = self.tape(persona_id);
        let open = chapter_view::open_chapter(&events)?.clone();
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
        if let Err(error) = room::clear_checkpoint(&self.log, persona_id, &persona.backend_id) {
            eprintln!("{}'s checkpoint was not withdrawn: {error}", persona.name);
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
            "Here is the chapter, oldest first.\n<toad_chapter_transcript>\n{}\n</toad_chapter_transcript>\nWrite the JSON note now.",
            chapters::serialize_chapter(slice)
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
        let keys = self.keys.provider_keys();
        persona
            .model_id
            .clone()
            .filter(|id| keys.contains_key(id.split('/').next().unwrap_or_default()))
            .or_else(|| models(&keys).first().map(|model| model.id.clone()))
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
            self.set_state(&session, SessionState::Thinking);
            let reach = self.reach_of(&session.persona_id);
            let mut updates = session
                .driver
                .prompt(wired.text, wired.attachments, reach)
                .await;
            let mut in_flight: HashMap<String, PendingTool> = HashMap::new();
            let mut asked = false;
            while let Some(update) = updates.recv().await {
                asked |= matches!(update, Update::Permission { .. });
                self.record(&session, update, &mut in_flight);
            }
            // A driver that stopped without a turn — its model errored, its
            // child died — leaves a tool spinning in the transcript forever,
            // and a card nobody is behind.
            self.fail_in_flight(&session, &mut in_flight);
            if asked {
                self.settle_permissions(&session.persona_id);
            }
            next = lock(&session.queue).pop_front();
        }
        *lock(&session.running) = false;
        self.set_state(&session, SessionState::Ready);
    }

    /// One driver update, as the tape and the wire see it.
    fn record(
        &self,
        session: &Session,
        update: Update,
        in_flight: &mut HashMap<String, PendingTool>,
    ) {
        match update {
            Update::Delta {
                kind,
                message_id,
                text,
            } => {
                // A muted turn must not run the writing indicator for a
                // message that will never land, so the delta is demoted with
                // the event it is building.
                let muted = kind == MessageKind::Agent
                    && quiet::mutes_deltas(lock(&session.quiet).as_ref(), now_ms());
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
            }
            Update::Message { kind, id, text } => self.append(
                session,
                match kind {
                    MessageKind::Agent => TranscriptEvent::Agent {
                        id,
                        ts: now_ms(),
                        text,
                        reactions: None,
                        ring: None,
                        receipt: None,
                    },
                    MessageKind::Thought => TranscriptEvent::Thought {
                        id,
                        ts: now_ms(),
                        text,
                    },
                },
            ),
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
                self.append(
                    session,
                    pending.event(&call_id, ToolStatus::InProgress, None),
                );
                in_flight.insert(call_id, pending);
            }
            Update::ToolResult {
                call_id,
                ok,
                output,
            } => {
                let Some(pending) = in_flight.remove(&call_id) else {
                    return;
                };
                let status = if ok {
                    ToolStatus::Completed
                } else {
                    ToolStatus::Failed
                };
                let output = ToolOutput::Text {
                    text: clip(&output, TOOL_OUTPUT_CHARS),
                };
                self.append(session, pending.event(&call_id, status, Some(output)));
            }
            Update::Permission {
                request_id,
                title,
                options,
            } => self.append(
                session,
                TranscriptEvent::Permission {
                    // The card is superseded by this id when it is answered,
                    // so the decision lands on the line already drawn rather
                    // than adding a second one below it.
                    id: format!("perm:{request_id}"),
                    ts: now_ms(),
                    request_id,
                    title,
                    options,
                    decision: None,
                    decided_option_name: None,
                },
            ),
            Update::Turn { stop_reason, usage } => {
                // A cancelled turn leaves tools running; they are marked
                // before the turn is closed, so the transcript never shows a
                // finished turn above a tool still in progress.
                self.fail_in_flight(session, in_flight);
                self.checkpoint(session);
                self.append(
                    session,
                    TranscriptEvent::Turn {
                        id: new_id(),
                        ts: now_ms(),
                        stop_reason,
                        usage,
                    },
                );
            }
            Update::Notice { level, text } => self.append(
                session,
                TranscriptEvent::Notice {
                    id: new_id(),
                    ts: now_ms(),
                    level,
                    text,
                },
            ),
        }
    }

    /// Remembers the agent's session id, now that a turn on it has completed.
    ///
    /// Written once per fresh session and only after a turn, because some
    /// agents issue an id they cannot reopen until a prompt has committed —
    /// and a checkpoint that fails to load is a teammate that starts cold
    /// believing it did not have to.
    fn checkpoint(&self, session: &Session) {
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

    fn set_state(&self, session: &Session, state: SessionState) {
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
        lock(&self.sessions)
            .get(persona_id)
            .cloned()
            .ok_or_else(|| "That teammate is not running.".to_string())
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
                Some(room) => room.sweep_chapters(&mut looked_again).await,
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
/// as one implicit chapter.
fn said(events: &[Value]) -> Vec<Said> {
    let within = match chapter_view::open_chapter(events) {
        Some(open) => chapter_view::slice_of(events, open),
        None if chapter_view::chapters_of(events).is_empty() => events,
        None => &[],
    };
    within
        .iter()
        .filter_map(|event| {
            let text = event.get("text")?.as_str()?.to_string();
            match event.get("kind")?.as_str()? {
                "user" => Some(Said::User(text)),
                "agent" => Some(Said::Agent(text)),
                _ => None,
            }
        })
        .collect()
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
        let busy = *lock(&session.running);
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
/// stands, how far it can reach, what day it is, and — when it is joining a
/// conversation that already has chapters behind it — what happened in the one
/// that closed. Everything here is something it would otherwise have to ask
/// for or guess.
fn preamble(persona: &Persona, reach: Option<Reach>, wake: Option<String>) -> String {
    // No reach is an agent whose tools are its own: Toad enforces nothing over
    // them, so it promises nothing about them either.
    let reach_sentence = match reach {
        Some(Reach::Workspace) => {
            " Your tools reach inside that directory and nowhere else: a path that leaves it is refused."
        }
        Some(Reach::Machine) => " Your tools reach the whole machine, not only that directory.",
        None => "",
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
    let standing = format!(
        "{identity}\n\nYour working directory is {}.{reach_sentence}\n\nToday is {}.",
        persona.cwd,
        Local::now().format("%A %-d %B %Y")
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

fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

#[cfg(test)]
mod tests;
