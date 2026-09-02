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
//!
//! Reach is read from the roster at every prompt rather than from the persona
//! the session started with: a live session is not told when its teammate is
//! edited, and the switch has to take on the next turn.

use crate::contract::{
    ConfigChoice, Persona, Reach, SessionCapabilities, SessionInfo, SessionState, StreamDelta,
    ToolOutput, ToolStatus, TranscriptEvent,
};
use crate::driver::rig::{InProcess, Said, models};
use crate::driver::{Driver, MessageKind, Update, clip};
use crate::log::{Log, StreamId};
use crate::room;
use crate::store::search::Indexer;
use chrono::Local;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::broadcast;

/// How much of a tool's output the transcript keeps. The model was given all
/// of it; this is the size of the bubble.
const TOOL_OUTPUT_CHARS: usize = 4_000;

/// How far behind a listener may fall before it starts missing news. Both
/// channels carry things a slow reader can recover from — a session's state is
/// re-askable, and a lost delta is made good by the message that follows it.
const BROADCAST_DEPTH: usize = 256;

/// Where the provider keys come from.
///
/// The room never holds a secret: it asks for the keys at the start of every
/// turn, so a key added or rotated on the desk is in force on the next one and
/// nothing keeps a stale copy. The vault implements this; a test hands over a
/// map.
pub trait ProviderKeys: Send + Sync {
    fn provider_keys(&self) -> HashMap<String, String>;
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
    driver: Arc<dyn Driver>,
    info: Mutex<SessionInfo>,
    /// Prompts that arrived while a turn was running. One turn at a time: a
    /// redirect waits for the turn it would have interrupted.
    queue: Mutex<VecDeque<String>>,
    running: Mutex<bool>,
}

/// Every session in the room, and the one place their words are written down.
pub struct Room {
    log: Log,
    keys: Arc<dyn ProviderKeys>,
    /// The one writer of the search index. `None` when it could not be opened,
    /// which costs search and never a record.
    indexer: Mutex<Option<Indexer>>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    info_changes: broadcast::Sender<SessionInfo>,
    deltas: broadcast::Sender<StreamDelta>,
}

impl Room {
    pub fn new(log: Log, keys: Arc<dyn ProviderKeys>) -> Arc<Self> {
        let indexer = match Indexer::open(&log) {
            Ok(indexer) => Some(indexer),
            Err(error) => {
                eprintln!("the search index could not be opened: {error}");
                None
            }
        };
        Arc::new(Self {
            log,
            keys,
            indexer: Mutex::new(indexer),
            sessions: Mutex::new(HashMap::new()),
            info_changes: broadcast::channel(BROADCAST_DEPTH).0,
            deltas: broadcast::channel(BROADCAST_DEPTH).0,
        })
    }

    /// Brings a teammate up, on the driver its backend names.
    pub async fn start(self: &Arc<Self>, persona_id: &str) -> Result<SessionInfo, String> {
        let persona = self.persona(persona_id)?;
        if persona.backend_id != "pi" {
            return Err(format!(
                "{} runs on {}, and Toad Agent is the only driver this build has.",
                persona.name, persona.backend_id
            ));
        }
        // The directory exists from the moment the teammate can be spoken to.
        // A workspace under the data directory is made here; one the user
        // typed is made too, because a path they chose is a path they meant.
        std::fs::create_dir_all(&persona.cwd).map_err(|error| {
            format!(
                "{}'s working directory {} could not be made: {error}",
                persona.name, persona.cwd
            )
        })?;
        let reach = persona.reach.unwrap_or_default();
        let driver = Arc::new(InProcess::new(
            self.keys.clone(),
            preamble(&persona, reach),
            self.said(&persona.id),
        ));
        self.start_on(&persona, driver).await
    }

    /// Brings a teammate up on a driver the caller names. [`Room::start`] is
    /// this with the driver its backend chose.
    async fn start_on(
        self: &Arc<Self>,
        persona: &Persona,
        driver: Arc<dyn Driver>,
    ) -> Result<SessionInfo, String> {
        let reported = driver.start(persona).await?;
        let mut info = idle_info(&persona.id);
        info.state = SessionState::Ready;
        info.agent_name = Some(reported.agent_name);
        info.models = reported.models;
        info.current_model_id = Some(reported.current_model_id);
        info.model_label = reported.model_label;
        let session = Arc::new(Session {
            persona_id: persona.id.clone(),
            driver,
            info: Mutex::new(info.clone()),
            queue: Mutex::new(VecDeque::new()),
            running: Mutex::new(false),
        });
        lock(&self.sessions).insert(persona.id.clone(), session);
        let _ = self.info_changes.send(info.clone());
        Ok(info)
    }

    /// Ends the session. The teammate keeps its tape; what stops is the agent.
    pub fn stop(&self, persona_id: &str) -> Result<(), String> {
        let Some(session) = lock(&self.sessions).remove(persona_id) else {
            return Ok(());
        };
        session.driver.cancel();
        let mut info = idle_info(persona_id);
        info.state = SessionState::Stopped;
        let _ = self.info_changes.send(info);
        Ok(())
    }

    /// Hands the teammate a message and returns at once: the turn runs on its
    /// own task and everything it does arrives as tape events and deltas. A
    /// message sent during a turn is queued behind it.
    pub fn prompt(self: &Arc<Self>, persona_id: &str, text: &str) -> Result<(), String> {
        let session = self.session(persona_id)?;
        self.append(
            persona_id,
            &TranscriptEvent::User {
                id: new_id(),
                ts: now_ms(),
                text: text.to_string(),
                attachments: None,
                reactions: None,
                reply_to: None,
                scheduled: None,
                ring: None,
                receipt: None,
            },
        );
        let running_already = {
            let mut running = lock(&session.running);
            let was = *running;
            *running = true;
            was
        };
        if running_already {
            lock(&session.queue).push_back(text.to_string());
            return Ok(());
        }
        let room = self.clone();
        let first = text.to_string();
        tokio::spawn(async move { room.run_turns(session, first).await });
        Ok(())
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

    async fn run_turns(self: Arc<Self>, session: Arc<Session>, first: String) {
        let mut next = Some(first);
        while let Some(text) = next.take() {
            self.set_state(&session, SessionState::Thinking);
            let reach = self.reach_of(&session.persona_id);
            let mut updates = session.driver.prompt(text, reach).await;
            let mut in_flight: HashMap<String, PendingTool> = HashMap::new();
            while let Some(update) = updates.recv().await {
                self.record(&session.persona_id, update, &mut in_flight);
            }
            // A driver that stopped without a turn — its model errored, its
            // child died — leaves a tool spinning in the transcript forever.
            self.fail_in_flight(&session.persona_id, &mut in_flight);
            next = lock(&session.queue).pop_front();
        }
        *lock(&session.running) = false;
        self.set_state(&session, SessionState::Ready);
    }

    /// One driver update, as the tape and the wire see it.
    fn record(
        &self,
        persona_id: &str,
        update: Update,
        in_flight: &mut HashMap<String, PendingTool>,
    ) {
        match update {
            Update::Delta {
                kind,
                message_id,
                text,
            } => {
                let persona_id = persona_id.to_string();
                let _ = self.deltas.send(match kind {
                    MessageKind::Agent => StreamDelta::AgentDelta {
                        persona_id,
                        message_id,
                        text,
                    },
                    MessageKind::Thought => StreamDelta::ThoughtDelta {
                        persona_id,
                        message_id,
                        text,
                    },
                });
            }
            Update::Message { kind, id, text } => self.append(
                persona_id,
                &match kind {
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
                    persona_id,
                    &pending.event(&call_id, ToolStatus::InProgress, None),
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
                self.append(persona_id, &pending.event(&call_id, status, Some(output)));
            }
            Update::Turn { stop_reason, usage } => {
                // A cancelled turn leaves tools running; they are marked
                // before the turn is closed, so the transcript never shows a
                // finished turn above a tool still in progress.
                self.fail_in_flight(persona_id, in_flight);
                self.append(
                    persona_id,
                    &TranscriptEvent::Turn {
                        id: new_id(),
                        ts: now_ms(),
                        stop_reason,
                        usage,
                    },
                );
            }
            Update::Notice { level, text } => self.append(
                persona_id,
                &TranscriptEvent::Notice {
                    id: new_id(),
                    ts: now_ms(),
                    level,
                    text,
                },
            ),
        }
    }

    fn fail_in_flight(&self, persona_id: &str, in_flight: &mut HashMap<String, PendingTool>) {
        for (call_id, pending) in in_flight.drain() {
            self.append(
                persona_id,
                &pending.event(&call_id, ToolStatus::Failed, None),
            );
        }
    }

    /// Writes one event to the teammate's tape and offers it to the index.
    fn append(&self, persona_id: &str, event: &TranscriptEvent) {
        let event = match serde_json::to_value(event) {
            Ok(event) => event,
            Err(error) => {
                eprintln!("a transcript event for {persona_id} could not be written: {error}");
                return;
            }
        };
        if let Err(error) = self
            .log
            .append(&StreamId::Tape(persona_id.to_string()), &event)
        {
            eprintln!("the tape for {persona_id} could not be appended to: {error}");
            return;
        }
        self.index(persona_id, &event);
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

    /// What the teammate and its agent have said to each other so far, for a
    /// driver that starts back into the conversation.
    fn said(&self, persona_id: &str) -> Vec<Said> {
        self.log
            .load(&StreamId::Tape(persona_id.to_string()))
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

    fn session(&self, persona_id: &str) -> Result<Arc<Session>, String> {
        lock(&self.sessions)
            .get(persona_id)
            .cloned()
            .ok_or_else(|| "That teammate is not running.".to_string())
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

/// What the agent is told before it is told anything else: who it is, where it
/// stands, how far it can reach, and what day it is. Everything here is
/// something it would otherwise have to ask for or guess.
fn preamble(persona: &Persona, reach: Reach) -> String {
    let reach_sentence = match reach {
        Reach::Workspace => {
            "Your tools reach inside that directory and nowhere else: a path that leaves it is refused."
        }
        Reach::Machine => "Your tools reach the whole machine, not only that directory.",
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
    format!(
        "{identity}\n\nYour working directory is {}. {reach_sentence}\n\nToday is {}.",
        persona.cwd,
        Local::now().format("%A %-d %B %Y")
    )
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
