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
//! No stream moves: a thread is a view over the files that were there before
//! it. A [`ThreadId`] is on the wire, in `thread.*` and `{"threadId": …}`
//! (`docs/wire.md`).

mod link;
mod policy;
mod store;

pub use link::{Link, Opener};
pub use policy::{Answer, Computer, Idle, Lease, Policy, Restart, Resume, Tools};
#[cfg(test)]
pub(crate) use policy::{QUIET_MS, SIDE_IDLE_MS};
pub use store::ThreadStore;

use crate::contract::{End, ThreadId, ThreadKind};
use crate::log::StreamId;
use std::fmt;

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

    pub fn call(call_id: impl Into<String>) -> Self {
        Self::of(ThreadKind::Call, call_id)
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

    /// The stream the thread's events are on.
    pub fn stream(&self) -> Option<StreamId> {
        let key = self.key.clone();
        match self.kind {
            ThreadKind::Dm => Some(StreamId::Tape(key)),
            ThreadKind::Side => Some(StreamId::Side(key)),
            ThreadKind::Pair => Some(StreamId::Pair(key)),
            ThreadKind::Run => Some(StreamId::Run(key)),
            ThreadKind::Call => Some(StreamId::Call(key)),
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
