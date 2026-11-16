//! Where a handoff does its work: the thread it opens, or for a follow-up
//! saved before the inbox, the thread it continues; and whether the handoff a
//! work thread serves still stands.

use super::*;

/// Where a handoff's work is done.
pub(super) enum Place {
    /// A thread of its own, opened for it.
    New,
    /// This thread: the one it was dispatched into, or the one it continues.
    In(String),
    /// Not known yet: the thread it continues has not been opened.
    Later,
}

impl Pair {
    /// Where a handoff is done. One from before the inbox that continues an
    /// earlier handoff (`continues`, the opener's id) follows the handoffs
    /// before it into that thread, as its sender sent them, and waits while
    /// none of them has a thread yet and one still will. One whose opener is
    /// not among them opens a thread of its own.
    pub(super) fn place(&self, request: &Request) -> Place {
        if let Some(thread) = &request.thread {
            return Place::In(thread.clone());
        }
        let Some(opener) = &request.continues else {
            return Place::New;
        };
        let before: Vec<&Request> = self
            .requests
            .iter()
            .take_while(|r| r.id != request.id)
            .filter(|r| {
                r.from == request.from
                    && r.to == request.to
                    && r.intent == Intent::Handoff
                    && (r.id == *opener || r.continues.as_ref() == Some(opener))
            })
            .collect();
        if let Some(thread) = before.iter().rev().find_map(|r| r.thread.clone()) {
            return Place::In(thread);
        }
        if before
            .iter()
            .any(|r| matches!(r.phase(), Phase::Queued | Phase::Running))
        {
            Place::Later
        } else {
            Place::New
        }
    }
}

impl Room {
    /// Whether the handoff a thread serves no longer stands: the latest one
    /// handed into it was released or stopped. What the thread's link says,
    /// and so what the window shows.
    pub(in crate::session) fn handoff_ended(&self, side_id: &str) -> bool {
        self.exchange_pairs()
            .iter()
            .flat_map(|pair| &pair.requests)
            .rev()
            .find(|r| r.intent == Intent::Handoff && r.thread.as_deref() == Some(side_id))
            .is_some_and(|r| !r.stands())
    }
}
