//! Whether the handoff a work thread serves still stands.

use super::*;

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
