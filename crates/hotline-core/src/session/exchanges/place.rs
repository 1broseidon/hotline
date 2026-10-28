//! Where a request that works in a thread does its work: the thread a
//! handoff opens or continues, found by the name its sender passes.

use super::*;

/// Where a handoff's work is done, or where an ask that names a thread is
/// answered.
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
    /// id of any handoff that ran there (or its side-thread id): the id of the
    /// one that opened it.
    pub(super) fn thread_named(&self, named: &str, from: &str, to: &str) -> Option<String> {
        let clean = named.strip_prefix("side:").unwrap_or(named);
        self.requests
            .iter()
            .rev()
            .find(|r| {
                r.from == from
                    && r.to == to
                    && r.intent == Intent::Handoff
                    && (r.id == named || r.id == clean || r.thread.as_deref() == Some(clean))
            })
            .map(|r| r.continues.clone().unwrap_or_else(|| r.id.clone()))
    }
    /// Where a request that works in a thread is done. One that names a thread
    /// follows the handoffs before it into that thread, as its sender sent
    /// them; it is refused when no handoff of its sender's to its recipient
    /// opened one by that name, whatever another desk says. An ask never
    /// opens a thread: it waits for the handoff that will, and is refused
    /// when none will.
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
            return Err(match request.intent {
                Intent::Ask => "That work thread was not opened by this teammate's handoff. Ask without a thread.",
                Intent::Handoff => "That work thread was not opened by this teammate's handoff. Send without a thread to open a new one.",
            }
            .into());
        }
        if let Some(thread) = before.iter().rev().find_map(|r| r.thread.clone()) {
            return Ok(Place::In(thread));
        }
        // None of them has a thread yet: one still to be handed over opens
        // it, and this follows; if none ever will, a handoff opens it.
        if before
            .iter()
            .any(|r| matches!(r.phase, Phase::Queued | Phase::Running))
        {
            Ok(Place::Later)
        } else if request.intent == Intent::Ask {
            Err("That work thread has not been opened yet. Ask without a thread, or wait for its handoff to start.".into())
        } else {
            Ok(Place::New)
        }
    }
}

impl Room {
    /// The work threads `from`'s handoffs to `to` run in and still serve, oldest
    /// first, each by the name a later handoff passes as `thread`.
    pub(crate) fn handoff_threads_for(&self, from: &str, to: &str) -> Vec<serde_json::Value> {
        let Some(key) = thread_key(from, to) else {
            return Vec::new();
        };
        let Some(pair) = self.exchange_pair(&key) else {
            return Vec::new();
        };
        let mut seen = Vec::<String>::new();
        let mut out = Vec::new();
        for r in pair.requests.iter().rev() {
            if r.from != from
                || r.to != to
                || r.intent != Intent::Handoff
                || r.released
                || r.phase == Phase::Stopped
            {
                continue;
            }
            let thread_id = r.continues.clone().unwrap_or_else(|| r.id.clone());
            if seen.contains(&thread_id) {
                continue;
            }
            seen.push(thread_id.clone());
            out.push(serde_json::json!({
                "thread": thread_id,
                "sideThreadId": r.thread,
                "phase": r.phase,
                "lastRequestId": r.id,
            }));
        }
        out.reverse();
        out
    }
}
