//! The turns of an agent, queued and driven.
//!
//! [`Turns`] is the queue behind one agent: the lines said while it is in the
//! middle of a turn, and whether a driver holds it. [`Room::run_queue`] is the
//! one loop over it, for every kind of thread with a live agent: the DM and a
//! work thread are each an [`Occupant`], which says what is its own at the
//! few places a kind differs (whether its agent still answers, whether a line
//! may begin a turn, what the turn does with what the agent says, what is done
//! when it ends). [`Threads::turn`] is what one turn is, for those and for a
//! run: it drives the agent, and its [`Witness`] writes what the agent does to
//! the thread. A work thread's and a run's witness is [`Told`], which writes
//! through the shared write path and refuses an agent a card nobody may answer;
//! the DM's is its own (`session/dm.rs`), which also stamps the tape, reads
//! lines on, and tells the phone.

use super::runner::{Driven, Witness, Words, drive_updates};
use super::threads::Threads;
use super::{Room, Wired, lock};
use crate::contract::{Attachment, DeliveryFrom, DeltaKind, Reach, StreamDelta, TranscriptEvent};
use crate::driver::{Driver, MessageKind, Update};
use crate::thread::ThreadId;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
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
    /// Where the line came from, when something came back for the thread: a
    /// colleague's result carries the exchange request it answers, which is
    /// checked again as the turn begins.
    pub from: Option<DeliveryFrom>,
    /// The delivery this line is, which is stamped read once its turn has
    /// begun: until then a restart finds it still to be heard.
    pub delivery: Option<String>,
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

/// What starts a turn: a line the driver is told, or the work the agent took
/// up by itself, which is already under way and only has to be read.
pub(super) enum Source {
    Say {
        text: String,
        attachments: Vec<Attachment>,
    },
    /// The turn that failed on this line, run again (`Driver::retry`).
    Retry {
        text: String,
        attachments: Vec<Attachment>,
    },
    Updates(mpsc::Receiver<Update>),
}

impl From<Line> for Source {
    fn from(line: Line) -> Self {
        Self::Say {
            text: line.text,
            attachments: line.attachments,
        }
    }
}

/// How a work thread's turn and a run's are written: through the shared write
/// path, and nothing once the thread is over. A card nobody may answer comes
/// back refused by the write, and the agent is told no, so it is not left
/// waiting on a button nobody can press.
struct Told<'a, F> {
    threads: Threads<'a>,
    seat: Seat<'a>,
    open: F,
}

impl<F: Fn() -> bool> Witness for Told<'_, F> {
    fn delta(&mut self, kind: MessageKind, message_id: &str, words: Words<'_>, muted: bool) {
        if words.shown.is_empty() || !(self.open)() {
            return;
        }
        let _ = self.threads.room.deltas.send(delta_of(
            self.seat.thread,
            kind,
            message_id,
            words.shown,
            muted,
        ));
    }

    fn write(&mut self, event: TranscriptEvent, _asked: bool) {
        if !(self.open)() {
            return;
        }
        let written = self
            .threads
            .write(self.seat.thread, self.seat.persona_id, &event);
        for refusal in written.refused {
            refusal.deliver(self.seat.driver);
        }
    }
}

impl Threads<'_> {
    /// Drives one turn of a work thread's or a run's agent to its end, writing
    /// what it does to the thread as it goes. Nothing is written or announced
    /// once `open` says the thread is over. A cancel stops the agent and still
    /// reads what the turn's own end was.
    pub(super) async fn turn(
        &self,
        seat: Seat<'_>,
        line: Line,
        reach: Reach,
        cancel: Option<&CancellationToken>,
        open: impl Fn() -> bool,
    ) -> Driven {
        let driver = seat.driver;
        let mut told = Told {
            threads: Threads { room: self.room },
            seat,
            open,
        };
        self.drive(driver, line.into(), reach, cancel, None, &mut told)
            .await
    }

    /// Drives one turn of a thread's agent to its end, telling `witness` what
    /// it does as it goes. `ready` wakes the turn when a line is queued behind
    /// it, for a witness that steers.
    pub(super) async fn drive(
        &self,
        driver: &dyn Driver,
        source: Source,
        reach: Reach,
        cancel: Option<&CancellationToken>,
        ready: Option<&tokio::sync::Notify>,
        witness: &mut impl Witness,
    ) -> Driven {
        let updates = match source {
            Source::Say { text, attachments } => driver.prompt(text, attachments, reach).await,
            Source::Retry { text, attachments } => driver.retry(text, attachments, reach).await,
            Source::Updates(updates) => updates,
        };
        drive_updates(driver, updates, cancel, ready, witness).await
    }
}

/// What a line came to at the door of its turn.
pub(super) enum Begin<H> {
    /// The line does not begin a turn: it is spent, or it is no longer wanted.
    Skip,
    /// The thread is not answering any more, and the queue is let go of.
    Stop,
    /// The turn is to be taken, with what the kind holds through it.
    Go(H),
}

/// What the loop does once a turn is over.
pub(super) enum Then {
    Next,
    Stop,
}

/// The live agent of a thread that is answered in turns, as the one loop
/// ([`Room::run_queue`]) sees it. Each kind says what is its own at the
/// points where kinds differ: the DM in `session/dm.rs`, a work thread in
/// `session/sides.rs`.
pub(super) trait Occupant: Send + Sync + 'static {
    /// What the kind queues: the DM's lines carry what a schedule, a call or a
    /// delivery stamped on them, and a work thread's carry a handoff.
    type Line: Send + 'static;
    /// What the kind keeps from the door of a turn to its end.
    type Held: Send + 'static;

    fn queue(&self) -> &Mutex<Turns<Self::Line>>;
    fn persona_id(&self) -> &str;
    fn thread(&self) -> ThreadId;
    /// What the agent holds the teammate's computer under.
    fn holder(&self) -> &crate::computer::gate::Holder;

    /// Whether this agent is still the one that answers: its authority is
    /// current and the room still holds it.
    fn answering(self: &Arc<Self>, room: &Room) -> bool;

    /// The agent is not answering, and what waited for it is let go of.
    fn abandon(self: &Arc<Self>) {}

    /// Whether the line begins a turn, and what the kind holds for it.
    fn begin(
        self: &Arc<Self>,
        room: &Arc<Room>,
        line: Self::Line,
    ) -> impl Future<Output = Begin<Self::Held>> + Send;

    /// The turn itself: one [`Threads::drive`] with the kind's witness.
    fn turn(
        self: &Arc<Self>,
        room: &Arc<Room>,
        held: &mut Self::Held,
    ) -> impl Future<Output = Driven> + Send;

    /// What the kind does when a turn is over, before the next line is taken.
    fn end(self: &Arc<Self>, room: &Arc<Room>, held: Self::Held, driven: Driven) -> Then;

    /// The queue is empty, or the loop stopped: the agent is at rest.
    fn rest(self: &Arc<Self>, room: &Arc<Room>);
}

impl Room {
    /// Drives a thread's turns, one after another, until none is waiting. The
    /// caller holds the claim on the queue ([`Turns::claim`]) and hands in the
    /// line it claimed.
    pub(super) async fn run_queue<O: Occupant>(self: Arc<Self>, occupant: Arc<O>, first: O::Line) {
        let mut next = Some(first);
        while let Some(line) = next.take() {
            if !occupant.answering(&self) {
                occupant.abandon();
                break;
            }
            match occupant.begin(&self, line).await {
                Begin::Skip => {}
                Begin::Stop => break,
                Begin::Go(mut held) => {
                    let driven = occupant.turn(&self, &mut held).await;
                    self.let_go_of_computer(occupant.persona_id(), occupant.holder());
                    if let Then::Stop = occupant.end(&self, held, driven) {
                        break;
                    }
                }
            }
            next = lock(occupant.queue()).next_line();
        }
        occupant.rest(&self);
    }
}

/// The live words of a thread, as the wire addresses them: by the thread's id,
/// for every kind. The wire turns it into the older per-kind deltas for a
/// client that does not know this one.
pub(super) fn delta_of(
    thread: &ThreadId,
    kind: MessageKind,
    message_id: &str,
    text: &str,
    muted: bool,
) -> StreamDelta {
    StreamDelta::ThreadDelta {
        thread: thread.clone(),
        message_id: message_id.to_string(),
        kind: match kind {
            MessageKind::Agent if !muted => DeltaKind::Text,
            _ => DeltaKind::Thought,
        },
        text: text.to_string(),
    }
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
    fn every_kind_is_shown_its_words_live_by_its_thread() {
        let live =
            |thread: &ThreadId, muted| delta_of(thread, MessageKind::Agent, "m1", "hel", muted);
        for thread in [
            ThreadId::side("s1"),
            ThreadId::run("r1"),
            ThreadId::dm("ada"),
        ] {
            assert_eq!(
                live(&thread, false),
                StreamDelta::ThreadDelta {
                    thread: thread.clone(),
                    message_id: "m1".to_string(),
                    kind: DeltaKind::Text,
                    text: "hel".to_string(),
                }
            );
        }
        assert!(matches!(
            live(&ThreadId::side("s1"), true),
            StreamDelta::ThreadDelta {
                kind: DeltaKind::Thought,
                ..
            }
        ));
    }
}
