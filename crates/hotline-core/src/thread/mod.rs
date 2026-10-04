//! Threads: the one name for a conversation the room keeps.
//!
//! A teammate's DM, a side thread, an exchange between two teammates, a voice
//! call and a subagent run are each an append-only stream with participants, a
//! parent, a state and an agent that answers in it. They differ in policy, not
//! in kind of thing; the design is `docs/threads.md` and the decision is
//! `docs/adr/0001-threads-are-a-primitive.md`.
//!
//! This module is the shared vocabulary. A [`Thread`] is read from the records
//! each kind already keeps ([`ThreadStore`]), and what a kind decides is a
//! [`Policy`] value. The one write path that applies the policy is
//! `Room::threads().write`, beside the room it touches.
//!
//! None of it is on the wire yet, and no stream moves: a thread is a view over
//! the files that were there before it.

mod link;
mod policy;
mod store;

pub use link::Link;
pub use policy::{
    Answer, Idle, Lease, Policy, QUIET_MS, Restart, SIDE_IDLE_MS, Seed, Surface, Tools,
};
pub use store::ThreadStore;

use crate::log::StreamId;
use serde::{Deserialize, Serialize};
use std::fmt;
use ts_rs::TS;

/// What kind of conversation a thread is. The kind picks its [`Policy`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ThreadKind {
    /// The person and a teammate: the teammate's tape.
    Dm,
    /// A second, parallel conversation with a teammate, led by the person.
    Side,
    /// Two teammates talking to each other, by an ask or a handoff.
    Pair,
    /// A voice call. Not stored yet: its transcript is in memory.
    Call,
    /// A subagent's run of one task.
    Run,
}

impl ThreadKind {
    /// The kind as it is written in a link and in a log line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Dm => "dm",
            Self::Side => "side",
            Self::Pair => "pair",
            Self::Call => "call",
            Self::Run => "run",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        [Self::Dm, Self::Side, Self::Pair, Self::Call, Self::Run]
            .into_iter()
            .find(|kind| kind.name() == name)
    }
}

/// Which thread: its kind and the key its stream is named by. A teammate's id
/// for a DM, the minted id for a side thread, run or call, and the pair key
/// (see [`crate::paths::thread_key`]) for a pair.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThreadId {
    pub kind: ThreadKind,
    pub key: String,
}

impl ThreadId {
    pub fn dm(persona_id: impl Into<String>) -> Self {
        Self::of(ThreadKind::Dm, persona_id)
    }

    pub fn side(side_id: impl Into<String>) -> Self {
        Self::of(ThreadKind::Side, side_id)
    }

    pub fn pair(key: impl Into<String>) -> Self {
        Self::of(ThreadKind::Pair, key)
    }

    pub fn run(run_id: impl Into<String>) -> Self {
        Self::of(ThreadKind::Run, run_id)
    }

    pub fn new(kind: ThreadKind, key: impl Into<String>) -> Self {
        Self::of(kind, key)
    }

    fn of(kind: ThreadKind, key: impl Into<String>) -> Self {
        Self {
            kind,
            key: key.into(),
        }
    }

    /// The stream the thread's events are on, or none for a kind that keeps
    /// no stream yet (a call).
    pub fn stream(&self) -> Option<StreamId> {
        let key = self.key.clone();
        match self.kind {
            ThreadKind::Dm => Some(StreamId::Tape(key)),
            ThreadKind::Side => Some(StreamId::Side(key)),
            ThreadKind::Pair => Some(StreamId::Pair(key)),
            ThreadKind::Run => Some(StreamId::Run(key)),
            ThreadKind::Call => None,
        }
    }
}

/// `side:<id>`, the way a log line or a search hit names a thread.
impl fmt::Display for ThreadId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.kind.name(), self.key)
    }
}

/// Who takes part in a thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Participant {
    /// The person using the room.
    Person,
    /// A teammate, by persona id.
    Persona(String),
    /// A teammate's voice on a call, by persona id.
    Voice(String),
}

/// Where a thread stands. The records say what they say: a thread whose
/// record says `Live` has an agent only if the room holds one, and the room
/// parks the ones it does not at its next start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadState {
    /// An agent is running it.
    Live,
    /// Open, with no agent. Saying something in it brings one back.
    Parked,
    /// Over, and read-only until it is continued.
    Closed(End),
}

/// How a closed thread ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The person ended it.
    Person,
    /// The teammate said it was done.
    Agent,
    /// Nobody spoke in it for long enough.
    Idle,
    /// Its teammate was stopped, changed or removed, or the desk restarted.
    Stopped,
    /// A run finished its task.
    Done,
    /// A run could not finish.
    Failed,
    /// A run was stopped.
    Cancelled,
}

/// A thread's place under another: the parent, and the line on it the child
/// hangs off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadLink {
    pub thread: ThreadId,
    /// The id of the marker on the parent that stands for the child, if the
    /// kind writes one.
    pub event: Option<String>,
}

/// The agent's own memory of a thread, as its harness issued it: what a later
/// start reopens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentBinding {
    pub backend_id: String,
    pub session_id: String,
}

/// One conversation, as the records it already has describe it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub id: ThreadId,
    /// The thread this hangs off. None for a DM, and for a pair, which hangs
    /// off both teammates' DMs and so off no one of them.
    pub parent: Option<ThreadLink>,
    pub participants: Vec<Participant>,
    pub state: ThreadState,
    pub title: Option<String>,
    /// None until a turn has completed on a session that can be reopened.
    pub binding: Option<AgentBinding>,
    /// What the thread came to, written when it closed.
    pub note: Option<String>,
}

impl Thread {
    pub fn kind(&self) -> ThreadKind {
        self.id.kind
    }
}
