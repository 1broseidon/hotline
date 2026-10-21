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
//! `Room::threads().write`, beside the room it touches. The line a thread
//! leaves on its parent ([`Link`]) and the [`ThreadState`] it records are in
//! `hotline-store`, because the search index reads them, and are re-exported
//! here.
//!
//! No stream moves: a thread is a view over the files that were there before
//! it. A [`ThreadId`] is on the wire, in `thread.*` and `{"threadId": …}`
//! (`docs/wire.md`).

mod policy;
mod store;

pub use hotline_store::link::{AgentBinding, Link, Opener, ThreadState};
pub(crate) use policy::QUIET_MS;
#[cfg(test)]
pub(crate) use policy::SIDE_IDLE_MS;
pub use policy::{Answer, Computer, Idle, Lease, Policy, Restart, Resume, Tools};
pub use store::ThreadStore;

use crate::contract::{ThreadId, ThreadKind};

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

/// A thread's place under another: the parent, and the line on it the child
/// hangs off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadLink {
    pub thread: ThreadId,
    /// The id of the marker on the parent that stands for the child, if the
    /// kind writes one.
    pub event: Option<String>,
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
