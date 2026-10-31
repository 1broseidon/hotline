//! What a handed-over thread tells the teammate that handed it over, as
//! asides, and the brake a nudge counts on.
//!
//! A thread a teammate handed over informs its opener and does not report
//! home: each of its turns that is not the handoff's result leaves the opener
//! a note, in the colleague's own words (what the turn last said, and that it
//! waits on the person when it does), heard with the opener's next line in the
//! conversation the result will go to. The result itself still comes back as a
//! delivery, and leaves a nudge beside it: the delivery is what wakes the
//! opener, and results that come back together are heard in one turn
//! (`Room::results_behind`). The reply is counted on the pair's brake as it
//! always was.
//!
//! These ride on the handoff, as its result does: the opener asked for the
//! work, so what the work says back needs no grant of its own. A handoff that
//! no longer stands (stopped, revoked, unlinked) leaves nothing, and neither
//! does one whose opener is on a linked desk, which has no conversation here
//! to hear it in.

use super::*;
use crate::contract::{Aside, AsideLevel, AsideSource};
use crate::session::asides::result_aside;
use crate::session::runner::Driven;

/// The most of a turn's last words a note carries.
const NOTE_CHARS: usize = 400;

/// What a note adds when the turn stopped to wait on the person.
const WAITING: &str = "Waiting on the person.";

/// A turn's note, in the colleague's own words: what it said once its work
/// was done, and that it waits when it does. Nothing for a turn that said
/// nothing and is not waiting.
fn turn_note(driven: &Driven, waiting: bool) -> Option<String> {
    let words = if driven.last_words.is_empty() {
        driven.replies.last().cloned().unwrap_or_default()
    } else {
        driven.last_words.join("\n\n")
    };
    let words = crate::driver::clip(words.trim(), NOTE_CHARS);
    match (words.is_empty(), waiting) {
        (true, false) => None,
        (true, true) => Some(WAITING.to_string()),
        (false, false) => Some(words),
        (false, true) => Some(format!("{words}\n\n{WAITING}")),
    }
}

impl Room {
    /// A turn of a work thread ended that is not a handoff's result: the
    /// teammate that handed the thread over is left a note, if one did and
    /// its handoff still stands. `waiting` says the turn stopped to wait on
    /// the person.
    pub(in crate::session) fn note_turn(&self, side_id: &str, waiting: bool, driven: &Driven) {
        let Some(request) = self.handoff_standing(side_id) else {
            return;
        };
        let Some(text) = turn_note(driven, waiting) else {
            return;
        };
        let aside = Aside {
            id: new_id(),
            ts: now_ms(),
            to: request.from.clone(),
            from: AsideSource::Thread(self.work_thread(side_id)),
            by: Some(request.to.clone()),
            text,
            about: self.route_about(&request),
            level: AsideLevel::Note,
            heard: None,
            seen: None,
            looped_in: false,
        };
        if let Err(error) = self.keep_aside(&aside) {
            eprintln!("a note from thread {side_id}: {error}");
        }
    }

    /// The nudge a handoff's result leaves its sender beside the delivery,
    /// once however often the result is handed over.
    fn nudge_result(&self, key: &str, request: &Request) {
        let id = result_aside(&request.id);
        if self.aside_kept(&request.from, &id) {
            return;
        }
        let from = request
            .thread
            .as_deref()
            .map_or_else(|| ThreadId::pair(key), |thread| self.work_thread(thread));
        let aside = Aside {
            id,
            ts: now_ms(),
            to: request.from.clone(),
            from: AsideSource::Thread(from),
            by: Some(request.to.clone()),
            text: crate::driver::clip(request.reply.trim(), NOTE_CHARS),
            about: self.route_about(request),
            level: AsideLevel::Nudge,
            heard: None,
            seen: None,
            looped_in: false,
        };
        if let Err(error) = self.keep_aside(&aside) {
            eprintln!("the nudge for result {}: {error}", request.id);
        }
    }

    /// Leaves the nudge of a handoff's result, for a sender on this desk that
    /// hears it later.
    pub(super) fn nudge_handoff_result(&self, key: &str, request: &Request) {
        if request.intent == Intent::Handoff && !request.inline && !linked::is_linked(&request.from)
        {
            self.nudge_result(key, request);
        }
    }

    /// The latest handoff into a work thread while it stands, with a sender on
    /// this desk: the one whose opener its turns inform.
    pub(super) fn handoff_standing(&self, side_id: &str) -> Option<Request> {
        self.exchange_pairs()
            .into_iter()
            .flat_map(|pair| pair.requests)
            .rfind(|r| r.intent == Intent::Handoff && r.thread.as_deref() == Some(side_id))
            .filter(|r| {
                !r.released
                    && r.phase != Phase::Stopped
                    && !linked::is_linked(&r.from)
                    && self.exchange_lease_current(&r.id)
            })
    }

    /// The sender's thread a request's answer goes back to, when it is not
    /// the sender's main conversation: the job or the work thread it was sent
    /// from, while that is still open.
    pub(super) fn route_about(&self, request: &Request) -> Option<ThreadId> {
        if let Some(job) = request
            .reply_job
            .as_deref()
            .filter(|job| self.job_lives(job))
        {
            return Some(ThreadId::job(job));
        }
        request
            .reply_thread
            .as_deref()
            .filter(|thread| self.work_is_open(thread))
            .map(|thread| self.work_thread(thread))
    }

    /// Whether two teammates are at their brake: their exchange is paused
    /// until the person says to keep going.
    pub(in crate::session) fn brake_holds(&self, key: &str) -> bool {
        self.exchange_pair(key).is_some_and(|pair| pair.paused)
    }

    /// Counts a nudge's wake on two teammates' brake, as a reply is counted,
    /// and pauses them at the cap.
    pub(in crate::session) fn count_wake(&self, key: &str, by: &str, to: &str) {
        let _guard = lock(&self.exchange_lock);
        let mut pair = self.exchange_pair(key).unwrap_or(Pair {
            id: key.to_string(),
            a: by.to_string(),
            b: to.to_string(),
            exchanges: 0,
            paused: false,
            requests: vec![],
        });
        if pair.paused {
            return;
        }
        pair.exchanges += 1;
        pair.paused = pair.exchanges >= EXCHANGE_CAP;
        if let Err(error) = self.save_pair(&pair) {
            eprintln!("count a nudge's wake: {error}");
            return;
        }
        if pair.paused {
            self.exchange_card(&pair, ExchangePauseStatus::Pending);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driven(replies: &[&str], last_words: &[&str]) -> Driven {
        Driven {
            replies: replies.iter().map(ToString::to_string).collect(),
            last_words: last_words.iter().map(ToString::to_string).collect(),
            ..Driven::default()
        }
    }

    #[test]
    fn a_note_is_what_the_turn_last_said_and_says_when_it_waits() {
        let said = driven(
            &["Looking.", "The crane is fixed."],
            &["The crane is fixed."],
        );
        assert_eq!(
            turn_note(&said, false).as_deref(),
            Some("The crane is fixed.")
        );
        assert_eq!(
            turn_note(&said, true).as_deref(),
            Some("The crane is fixed.\n\nWaiting on the person.")
        );
        let quiet = driven(&[], &[]);
        assert_eq!(turn_note(&quiet, false), None);
        assert_eq!(turn_note(&quiet, true).as_deref(), Some(WAITING));
        let long = "x".repeat(NOTE_CHARS * 2);
        let clipped = turn_note(&driven(&[&long], &[]), false).unwrap();
        assert!(clipped.chars().count() <= NOTE_CHARS + 1, "{clipped}");
    }
}
