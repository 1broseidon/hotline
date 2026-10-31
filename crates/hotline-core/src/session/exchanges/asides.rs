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

/// What a note adds when the turn stopped to wait on the person and the
/// card it waits on cannot be found.
const WAITING: &str = "Waiting on the person.";

/// What a note adds when the turn waits on a colleague's answer.
const WAITING_ON_A_COLLEAGUE: &str = "Waiting on a colleague's answer.";

/// What a turn said once its work was done: its words after the last tool
/// card, else its last line.
pub(super) fn said_last(driven: &Driven) -> String {
    if driven.last_words.is_empty() {
        driven.replies.last().cloned().unwrap_or_default()
    } else {
        driven.last_words.join("\n\n")
    }
}

/// A turn's note, in the colleague's own words: what it said once its work
/// was done, and then what it waits on when it does. Nothing for a turn that
/// said nothing and is not waiting.
fn turn_note(driven: &Driven, waiting: Option<&str>) -> Option<String> {
    let words = crate::driver::clip(said_last(driven).trim(), NOTE_CHARS);
    match (words.is_empty(), waiting) {
        (true, None) => None,
        (true, Some(waiting)) => Some(waiting.to_string()),
        (false, None) => Some(words),
        (false, Some(waiting)) => Some(format!("{words}\n\n{waiting}")),
    }
}

impl Room {
    /// A turn of a work thread ended that is not a handoff's result: the
    /// teammate that handed the thread over is left a note, if one did and
    /// its handoff still stands. `waiting` says the turn stopped to wait, and
    /// the note says on what; a turn that waits only on questions to that
    /// teammate leaves none, since those reach it themselves.
    pub(in crate::session) fn note_turn(&self, side_id: &str, waiting: bool, driven: &Driven) {
        let Some(request) = self.handoff_standing(side_id) else {
            return;
        };
        let waiting = if waiting {
            match self.waiting_on(&request) {
                Some(waiting) => Some(waiting),
                None => return,
            }
        } else {
            None
        };
        let Some(text) = turn_note(driven, waiting.as_deref()) else {
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

    /// What a handoff's turn waits on, as its sender is told it: the card up
    /// for the person, said in its own words, or that a colleague is
    /// answering. Nothing when it waits only on questions to its sender.
    fn waiting_on(&self, handoff: &Request) -> Option<String> {
        let pairs = self.exchange_pairs();
        let asked = |id: &str| {
            pairs.iter().find_map(|pair| {
                pair.requests
                    .iter()
                    .find(|r| r.id == id && r.intent == Intent::Ask)
            })
        };
        let open: Vec<&HumanGate> = handoff
            .human_actions
            .iter()
            .filter(|gate| !gate.consumed)
            .collect();
        if !open.is_empty()
            && open
                .iter()
                .all(|gate| asked(&gate.id).is_some_and(|ask| ask.to == handoff.from))
        {
            return None;
        }
        let card = open
            .iter()
            .find_map(|gate| match self.human_card(&handoff.to, &gate.id) {
                Some((TranscriptEvent::HumanAction { reason, .. }, _)) => Some(reason),
                _ => self.card_before_asking(&pairs, &gate.id),
            });
        Some(match card {
            Some(card) => format!("Waiting on the person: {card}"),
            None if open.iter().any(|gate| asked(&gate.id).is_some()) => {
                WAITING_ON_A_COLLEAGUE.to_string()
            }
            None => WAITING.to_string(),
        })
    }

    /// The card a question waits behind before it can be asked at all, in
    /// its asker's main conversation: Keep going while the two have paused
    /// at the cap, which holds it whatever the asker may do, and only then
    /// the person allowing its asker to ask that colleague. A question to
    /// another desk is allowed there.
    fn card_before_asking(&self, pairs: &[Pair], ask: &str) -> Option<String> {
        let (pair, ask) = pairs.iter().find_map(|pair| {
            pair.requests
                .iter()
                .find(|r| r.id == ask && r.intent == Intent::Ask && r.phase == Phase::Queued)
                .map(|ask| (pair, ask))
        })?;
        let asker = self.participant(&ask.from).ok()?;
        let asker_name = crate::session::links::shown(&asker);
        if pair.paused {
            let colleague = self
                .participant(&ask.to)
                .map_or_else(|_| ask.to.clone(), |p| crate::session::links::shown(&p));
            return Some(format!(
                "Let {asker_name} and {colleague} keep going? They paused at {EXCHANGE_CAP} messages. The card is in {asker_name}'s main conversation."
            ));
        }
        if linked::is_linked(&ask.to) || ask.looped_in || self.about_live(ask).is_some() {
            return None;
        }
        let colleague = self.persona(&ask.to).ok()?;
        if self.collaboration_granted(&asker, &colleague, false) {
            return None;
        }
        Some(format!(
            "Allow {asker_name} to ask {} to work? The card is in {asker_name}'s main conversation.",
            colleague.name
        ))
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
            turn_note(&said, None).as_deref(),
            Some("The crane is fixed.")
        );
        assert_eq!(
            turn_note(&said, Some(WAITING)).as_deref(),
            Some("The crane is fixed.\n\nWaiting on the person.")
        );
        let quiet = driven(&[], &[]);
        assert_eq!(turn_note(&quiet, None), None);
        assert_eq!(turn_note(&quiet, Some(WAITING)).as_deref(), Some(WAITING));
        let long = "x".repeat(NOTE_CHARS * 2);
        let clipped = turn_note(&driven(&[&long], &[]), None).unwrap();
        assert!(clipped.chars().count() <= NOTE_CHARS + 1, "{clipped}");
    }
}
