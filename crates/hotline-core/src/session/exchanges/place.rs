//! Where a handoff does its work: the thread it opens or continues, found by
//! the name its sender passes.

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
    /// The name a handoff from `from` to `to` gives its thread, found by the
    /// id of any handoff that ran there, or by the work thread's own id: the
    /// id of the one that opened it.
    pub(super) fn thread_named(&self, named: &str, from: &str, to: &str) -> Option<String> {
        let side = named.strip_prefix("side:").unwrap_or(named);
        self.requests
            .iter()
            .rev()
            .find(|r| {
                r.from == from
                    && r.to == to
                    && r.intent == Intent::Handoff
                    && (r.id == named || r.thread.as_deref() == Some(side))
            })
            .map(|r| r.continues.clone().unwrap_or_else(|| r.id.clone()))
    }
    /// Where a handoff's work is done. One that names a thread follows the
    /// handoffs before it into that thread, as its sender sent them; it is
    /// refused when no handoff of its sender's to its recipient opened one by
    /// that name, whatever another desk says.
    pub(super) fn place(&self, request: &Request) -> Result<Place, String> {
        if let Some(thread) = &request.thread {
            return Ok(Place::In(thread.clone()));
        }
        let Some(opener) = &request.continues else {
            return Ok(Place::New);
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
        if !before.iter().any(|r| r.id == *opener) {
            return Err(
                "That work thread was not opened by this teammate's handoff. Send without a thread to open a new one."
                    .into(),
            );
        }
        if let Some(thread) = before.iter().rev().find_map(|r| r.thread.clone()) {
            return Ok(Place::In(thread));
        }
        // None of them has a thread yet: one still to be handed over opens
        // it, and this follows; if none ever will, this one opens it.
        if before
            .iter()
            .any(|r| matches!(r.phase, Phase::Queued | Phase::Running))
        {
            Ok(Place::Later)
        } else {
            Ok(Place::New)
        }
    }
}

impl Room {
    /// The work threads `from`'s handoffs to `to` run in and that were not
    /// stopped, oldest first, each by the name a later handoff passes as
    /// `thread`: what `list_teammates` tells `from` it can continue.
    pub(crate) fn handoff_threads_for(&self, from: &str, to: &str) -> Vec<serde_json::Value> {
        let Some(pair) = thread_key(from, to).and_then(|key| self.exchange_pair(&key)) else {
            return Vec::new();
        };
        let mut named: Vec<String> = Vec::new();
        let mut threads = Vec::new();
        for r in pair.requests.iter().rev() {
            if r.from != from || r.to != to || r.intent != Intent::Handoff {
                continue;
            }
            let name = r.continues.clone().unwrap_or_else(|| r.id.clone());
            if named.contains(&name) {
                continue;
            }
            named.push(name.clone());
            if r.phase == Phase::Stopped {
                continue;
            }
            threads.push(json!({
                "thread": name,
                "sideThreadId": r.thread,
                "phase": r.phase,
                "lastRequestId": r.id,
            }));
        }
        threads.reverse();
        threads
    }
}
