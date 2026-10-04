//! The turns of an agent, queued and driven.
//!
//! [`Turns`] is the queue behind one agent: the lines said while it is in the
//! middle of a turn, and whether a driver holds it. [`Threads::turn`] is what
//! one turn of a thread's agent is, for every kind that is built by
//! [`Room::thread_agent`]: it drives the agent, writes what it does to the
//! thread through the shared write path, and refuses an agent a card nobody
//! may answer. A side thread loops over its queue with it, and a run is one
//! turn of it.
//!
//! The main conversation still runs `run_turns`, on the same [`Turns`] and
//! the same driver loop ([`super::runner::drive_with`]), until it is ported.

use super::Wired;
use super::runner::{Driven, drive_with};
use super::threads::Threads;
use crate::contract::{Attachment, Reach, StreamDelta};
use crate::driver::{Driver, MessageKind};
use crate::thread::{ThreadId, ThreadKind};
use std::collections::VecDeque;
use tokio_util::sync::CancellationToken;

/// The lines waiting for an agent, and whether a driver is already taking
/// them. The main conversation queues [`Wired`] lines and a thread queues
/// [`Line`]s; the rule is the same.
///
/// The two are one fact and so they are one lock. Held apart, there was a
/// moment in which the driver had looked at an empty queue and not yet let go
/// of the turn: a line dispatched into it was filed behind a turn that was
/// already over, and the teammate then sat on it until the next thing said
/// shook it loose.
pub(super) struct Turns<T = Wired> {
    pub(super) waiting: VecDeque<T>,
    pub(super) running: bool,
}

impl<T> Turns<T> {
    /// Queues the line behind the turn in flight, or claims the driver for it.
    ///
    /// `Some` is the caller's to run: it holds the claim from here until
    /// [`Turns::next_line`] gives it back.
    pub(super) fn claim(&mut self, wire: T) -> Option<T> {
        if self.running {
            self.waiting.push_back(wire);
            return None;
        }
        self.running = true;
        Some(wire)
    }

    /// The next line for whoever holds the claim — or, when there is none, the
    /// release of that claim, in the same breath as the look.
    pub(super) fn next_line(&mut self) -> Option<T> {
        let next = self.waiting.pop_front();
        self.running = next.is_some();
        next
    }
}

impl<T> Default for Turns<T> {
    fn default() -> Self {
        Self {
            waiting: VecDeque::new(),
            running: false,
        }
    }
}

impl<T> Turns<T> {
    /// Drops every line waiting behind the turn in flight.
    pub(super) fn clear(&mut self) {
        self.waiting.clear();
    }
}

/// One line for a thread's agent: the words as the driver is to hear them,
/// already stamped, with what came attached.
pub(super) struct Line {
    pub text: String,
    pub attachments: Vec<Attachment>,
    /// Set when the turn this line starts is a handoff's: the exchange's
    /// request it answers. Said by whoever produced the line, never read back
    /// off an id.
    pub handoff: Option<HandoffLine>,
}

/// The handoff a turn of a work thread belongs to.
pub(super) struct HandoffLine {
    /// The exchange request that opened the thread.
    pub request: String,
    /// The card this line is the person's answer to, when the handoff had
    /// stopped to wait on one.
    pub answer: Option<String>,
}

/// The agent a turn is for, and the thread and teammate it answers as.
pub(super) struct Seat<'a> {
    pub thread: &'a ThreadId,
    pub persona_id: &'a str,
    pub driver: &'a dyn Driver,
}

impl Threads<'_> {
    /// Drives one turn of a thread's agent to its end, writing what it does
    /// to `thread` as it goes. Nothing is written or announced once `open`
    /// says the thread is over.
    ///
    /// A card the thread's policy says nobody answers comes back refused by
    /// the write, and the agent is told no, so it is not left waiting on a
    /// button nobody can press. A cancel stops the agent and still reads
    /// what the turn's own end was.
    pub(super) async fn turn(
        &self,
        seat: Seat<'_>,
        line: Line,
        reach: Reach,
        cancel: Option<&CancellationToken>,
        open: impl Fn() -> bool,
    ) -> Driven {
        let Seat {
            thread,
            persona_id,
            driver,
        } = seat;
        drive_with(
            driver,
            line.text,
            line.attachments,
            reach,
            cancel,
            |kind, message_id, text, muted| {
                if !open() {
                    return;
                }
                if let Some(delta) = delta_of(thread, kind, message_id, text, muted) {
                    let _ = self.room.deltas.send(delta);
                }
            },
            |event, _| {
                if !open() {
                    return;
                }
                for refusal in self.write(thread, persona_id, &event).refused {
                    refusal.deliver(driver);
                }
            },
        )
        .await
    }
}

/// The live words of a thread, as the wire addresses them. Only a side thread
/// is shown live: the window has no view of a run's words as they come.
fn delta_of(
    thread: &ThreadId,
    kind: MessageKind,
    message_id: &str,
    text: &str,
    muted: bool,
) -> Option<StreamDelta> {
    if thread.kind != ThreadKind::Side {
        return None;
    }
    let (side_id, message_id, text) =
        (thread.key.clone(), message_id.to_string(), text.to_string());
    Some(match kind {
        MessageKind::Agent if !muted => StreamDelta::SideAgentDelta {
            side_id,
            message_id,
            text,
        },
        _ => StreamDelta::SideThoughtDelta {
            side_id,
            message_id,
            text,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_waits_behind_the_turn_in_flight_and_the_claim_ends_with_the_queue() {
        let mut turns: Turns<&str> = Turns::default();
        assert_eq!(turns.claim("first"), Some("first"));
        assert_eq!(turns.claim("second"), None);
        assert_eq!(turns.claim("third"), None);
        assert_eq!(turns.next_line(), Some("second"));
        assert!(turns.running);
        turns.clear();
        assert_eq!(turns.next_line(), None);
        assert!(
            !turns.running,
            "looking at an empty queue lets go of the turn"
        );
        assert_eq!(turns.claim("again"), Some("again"));
    }

    #[test]
    fn only_a_side_thread_is_shown_its_words_live() {
        let live =
            |thread: &ThreadId, muted| delta_of(thread, MessageKind::Agent, "m1", "hel", muted);
        assert!(matches!(
            live(&ThreadId::side("s1"), false),
            Some(StreamDelta::SideAgentDelta { .. })
        ));
        assert!(matches!(
            live(&ThreadId::side("s1"), true),
            Some(StreamDelta::SideThoughtDelta { .. })
        ));
        assert!(live(&ThreadId::run("r1"), false).is_none());
    }
}
