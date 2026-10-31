//! Stop and Interrupt: the person cutting a turn that was doing a
//! colleague's work.
//!
//! Stop is the person's decision about that work. A handoff it reaches ends
//! for good: stopped, released, never handed over or run again, and its
//! sender is left one `note` aside saying so, heard with its next turn and
//! waking nothing, since a turn woken by it would send the work again and
//! undo the stop. An answer it cuts fails to its asker with the person's
//! sentence, never "send it again".
//!
//! Interrupt is the person redirecting the turn, and decides nothing. In a
//! work thread only the turn is cut: the request it was answering stays
//! running and begun on its record, with no turn on it, which is all that
//! says the thread carries it ([`Room::carried_into`]), and the person's
//! next line there takes it up and carries it on to its result. The main
//! conversation has no thread to carry an answer on, and the person's next
//! line must never become a colleague's answer, so there an Interrupt ends
//! the answer it cut with neutral words, to be asked again if still needed.

use super::*;
use crate::contract::{Aside, AsideLevel, AsideSource};
use crate::session::runner::Driven;
use crate::session::turns::Line;

/// The person's cut of the turn in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::session) enum Cut {
    /// Their Stop: the work the turn was doing ends as their decision.
    Stop,
    /// Their Interrupt (Esc): the turn is cut so they can say something.
    Interrupt,
}

/// Whether a turn ended because it was cut, rather than on its own: a cut
/// that lands after the turn has ended changes nothing about it.
pub(in crate::session) fn cut_short(driven: &Driven) -> bool {
    driven.cancelled
        || matches!(
            driven.stop_reason.as_deref(),
            Some("cancelled" | "canceled" | "aborted")
        )
}

/// What a request's turn that ended without its result says after whatever
/// it had said: the person's word when their cut ended it, else that its
/// sender should look before trying again.
pub(super) fn ended_without_a_result(ask: bool, cut: Option<Cut>) -> &'static str {
    match (ask, cut) {
        (true, Some(Cut::Stop)) => peers::ANSWER_STOPPED_BY_THE_PERSON,
        (true, Some(Cut::Interrupt)) => peers::ANSWER_INTERRUPTED_BY_THE_PERSON,
        (true, None) => {
            "The turn answering this question ended without a result; inspect before retrying."
        }
        (false, _) => "The handoff turn ended without a result; inspect before retrying.",
    }
}

/// The most of the cut turn's last words a stopped handoff's note carries.
const WORDS_CHARS: usize = 400;

impl Room {
    /// Ends `id`, a handoff still at work, as the person's Stop, and says
    /// whether it did. One whose turn had not `begun` is ended only if no
    /// turn of it ever began, so it is never handed over again; one waiting
    /// on the person or a colleague is ended whatever `begun` says, since only
    /// the answer the stop dropped would have carried it on. Its sender is
    /// told by one note aside of `words` (what the cut turn last said) and
    /// the person's sentence, which wakes nobody. A sender on a linked desk
    /// is sent the sentence as the stopped request's failure.
    pub(super) fn stop_handoff_for_the_person(&self, id: &str, begun: bool, words: &str) -> bool {
        let stoppable = |r: &Request| {
            r.id == id
                && r.intent == Intent::Handoff
                && match r.phase {
                    Phase::Running => begun || !r.started,
                    Phase::WaitingHuman => true,
                    _ => false,
                }
        };
        let mut ended = Vec::new();
        let stopped = {
            let _guard = lock(&self.exchange_lock);
            let Some(mut pair) = self
                .exchange_pairs()
                .into_iter()
                .find(|pair| pair.requests.iter().any(stoppable))
            else {
                return false;
            };
            let stopped = pair
                .requests
                .iter_mut()
                .find(|r| stoppable(r))
                .expect("the pair was found by it");
            if let Some((caller, target)) = lock(&self.exchange_leases).get(&stopped.id) {
                caller.revoke();
                target.revoke();
            }
            stopped.phase = Phase::Stopped;
            stopped.failed = true;
            stopped.reply = peers::HANDOFF_STOPPED_BY_THE_PERSON.into();
            stopped.release(&mut ended);
            let stopped = stopped.clone();
            if let Err(error) = self.save_pair(&pair) {
                eprintln!("save a handoff the person stopped: {error}");
                return false;
            }
            self.wake_owed(&pair);
            (pair.id, stopped)
        };
        self.mark_handoffs(&ended);
        if !linked::is_linked(&stopped.1.from) {
            self.tell_stopped(&stopped.0, &stopped.1, words);
        }
        true
    }

    /// The note a handoff the person stopped leaves its sender, once.
    fn tell_stopped(&self, key: &str, handoff: &Request, words: &str) {
        let words = crate::driver::clip(words.trim(), WORDS_CHARS);
        let sentence = peers::HANDOFF_STOPPED_BY_THE_PERSON;
        let from = handoff
            .thread
            .as_deref()
            .map_or_else(|| ThreadId::pair(key), |thread| self.work_thread(thread));
        let aside = Aside {
            id: format!("stopped:{}", handoff.id),
            ts: now_ms(),
            to: handoff.from.clone(),
            from: AsideSource::Thread(from),
            by: Some(handoff.to.clone()),
            text: if words.is_empty() {
                sentence.to_string()
            } else {
                format!("{words}\n\n{sentence}")
            },
            about: self.route_about(handoff),
            level: AsideLevel::Note,
            heard: None,
            seen: None,
            looped_in: false,
        };
        if let Err(error) = self.keep_aside(&aside) {
            eprintln!("tell {} its handoff was stopped: {error}", handoff.from);
        }
    }

    /// What the person's Stop in `side_id` reaches besides the turn it cuts:
    /// each handoff among the `dropped` lines, whose turn had not begun or
    /// which would have carried a waiting handoff on, and each request in
    /// `carried`, whose turn an Interrupt cut. Each ends as the person's
    /// stop of it; the thread goes on. A question among them fails to its
    /// asker with the person's sentence.
    pub(in crate::session) fn end_for_the_persons_stop(
        self: &Arc<Self>,
        dropped: &[Line],
        carried: &[String],
    ) {
        for request in dropped.iter().filter_map(|line| line.request.as_ref()) {
            self.stop_handoff_for_the_person(&request.request, false, "");
        }
        let stopped = Driven {
            cancelled: true,
            ..Driven::default()
        };
        for id in carried {
            if !self.stop_handoff_for_the_person(id, true, "") {
                self.finish_request(id, &stopped, Some(Cut::Stop));
            }
        }
    }

    /// The requests an Interrupt left `side_id` carrying, oldest first: begun
    /// by a turn of it and still running, and not `answering`, the one its
    /// turn in flight holds. Within a run only an Interrupt leaves a request
    /// so, since every other end of its turn finishes or stops it, and a
    /// restart settles it first; so the record alone says what a thread
    /// carries, a parked one's included.
    pub(in crate::session) fn carried_into(
        &self,
        side_id: &str,
        answering: Option<&str>,
    ) -> Vec<String> {
        self.exchange_pairs()
            .into_iter()
            .flat_map(|pair| pair.requests)
            .filter(|r| {
                r.phase == Phase::Running
                    && r.started
                    && r.thread.as_deref() == Some(side_id)
                    && answering != Some(r.id.as_str())
            })
            .map(|r| r.id)
            .collect()
    }

    /// A thread that closed before the person's next line came has no turn
    /// to carry what it carried: each ends as a turn that ended without a
    /// result. On a task of its own, since a stop ends a thread under the
    /// exchange lock this takes.
    pub(in crate::session) fn end_carried(&self, side_id: &str, answering: Option<&str>) {
        let carried = self.carried_into(side_id, answering);
        if carried.is_empty() {
            return;
        }
        let Some(room) = lock(&self.me).upgrade() else {
            return;
        };
        tokio::spawn(async move {
            let ended = Driven {
                stop_reason: Some("failed".into()),
                ..Driven::default()
            };
            for id in carried {
                room.finish_request(&id, &ended, None);
            }
        });
    }
}
