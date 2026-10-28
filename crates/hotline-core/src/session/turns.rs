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
use crate::contract::{ScheduledRun, ThreadId};
use crate::driver::{Driver, MessageKind, Update};
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
    /// Captured when accepted, so reattaching voice cannot speak an older turn.
    pub voice: Option<crate::voice::Origin>,
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
    /// Work threads retain their own lease for call-file delivery.
    pub capability: Option<&'a crate::driver::CapabilityLease>,
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
    voice: Option<crate::voice::Origin>,
    replies: Vec<serde_json::Value>,
    files: Vec<crate::contract::ChangedFile>,
    /// Replies the call already has whole: said as they streamed, or handed
    /// over with their spoken version. Their later bubbles are the same
    /// reply, so the call is never given them as replies of their own.
    with_call: Vec<String>,
}

impl<F: Fn() -> bool> Witness for Told<'_, F> {
    fn heard(
        &mut self,
        update: &Update,
        _: &mut std::collections::HashMap<String, super::prompt::PendingTool>,
    ) {
        if self.voice.is_none()
            || !(self.open)()
            || !self.threads.room.on_the_call(self.seat.thread)
        {
            return;
        }
        if let Update::Message {
            kind: MessageKind::Agent,
            id,
            text,
        } = update
        {
            let versions = crate::voice::spoken::versions(text);
            if versions.spoken.as_deref() != Some(versions.written.as_str())
                && crate::voice::spoken::carries_more(&versions.written)
            {
                // Classify the whole reply before pacing divides it into bubbles,
                // and write it before completion releases the spoken pointer.
                let post = super::Post {
                    id: format!("call-result:{}:{id}", self.seat.thread.key),
                    ..super::Post::words(&versions.written)
                };
                if let Err(error) =
                    self.threads
                        .room
                        .post(self.seat.persona_id, self.seat.thread, post)
                {
                    eprintln!("[voice] could not post call result: {error}");
                }
            }
        }
    }

    fn said_on_call(&self) -> bool {
        self.voice.is_some()
    }

    fn keep_written(&self) -> bool {
        self.voice.as_ref().is_some_and(|origin| origin.front)
            || lock(&self.threads.room.posting)
                .get(self.seat.thread)
                .is_some_and(|turn| turn.file_sent)
    }

    fn delta(&mut self, kind: MessageKind, message_id: &str, words: Words<'_>, muted: bool) {
        // Live work is reported once when the turn ends, never as speech deltas.
        if self.voice.as_ref().is_some_and(|origin| origin.front) {
            return;
        }
        if let Some(origin) = &self.voice
            && kind == MessageKind::Agent
            && !muted
            && let Some(voice) = lock(&self.threads.room.voice).upgrade()
        {
            voice.reply_delta(self.seat.persona_id, message_id, words.written, origin);
            if !self.with_call.iter().any(|id| id == message_id) {
                self.with_call.push(message_id.to_string());
            }
        }
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
        if self.seat.thread.kind == crate::contract::ThreadKind::Call
            && let TranscriptEvent::Tool {
                status: crate::contract::ToolStatus::Completed,
                files: Some(files),
                ..
            } = &event
        {
            for file in files {
                if let Some(known) = self.files.iter_mut().find(|known| known.path == file.path) {
                    known.created |= file.created;
                } else {
                    self.files.push(file.clone());
                }
            }
        }
        let live_work = self.voice.as_ref().is_some_and(|origin| origin.front);
        if live_work && matches!(event, TranscriptEvent::Tool { .. }) {
            // Only words after the last tool are the work's result.
            self.replies.clear();
        }
        let written = if live_work && matches!(event, TranscriptEvent::Agent { .. }) {
            super::threads::Written::default()
        } else {
            self.threads
                .write(self.seat.thread, self.seat.persona_id, &event)
        };
        if let TranscriptEvent::Agent {
            id, text, spoken, ..
        } = &event
        {
            let room = self.threads.room;
            let rest_of_reply = id.rsplit_once('-').is_some_and(|(reply, bubble)| {
                bubble.parse::<usize>().is_ok() && self.with_call.iter().any(|id| id == reply)
            });
            if let Some(origin) = &self.voice
                && !origin.front
                && !rest_of_reply
                && let Some(voice) = lock(&room.voice).upgrade()
            {
                if spoken.is_some() && !self.with_call.contains(id) {
                    self.with_call.push(id.clone());
                }
                let answer = spoken.as_ref().map_or_else(
                    || text.clone(),
                    |spoken| format!("<spoken>{spoken}</spoken><written>{text}</written>"),
                );
                voice.delivery(
                    self.seat.persona_id,
                    id,
                    self.seat.persona_id,
                    &answer,
                    true,
                    Some(origin),
                );
            }
            if let Ok(value) = serde_json::to_value(&event) {
                if let Some(previous) = self.replies.iter_mut().find(|reply| reply["id"] == *id) {
                    *previous = value;
                } else {
                    self.replies.push(value);
                }
            }
        }

        for refusal in written.refused {
            refusal.deliver(self.seat.driver);
        }
    }
}

impl Threads<'_> {
    /// Drives one turn of a work thread's, a job's or a run's agent to its
    /// end, writing what it does to the thread as it goes. Nothing is written
    /// or announced once `open` says the thread is over. A cancel stops the
    /// agent and still reads what the turn's own end was.
    ///
    /// The turn is open for `post_to_chat` while it runs; `job` is the job a
    /// job's thread posts as, so its post, file or image is stamped with it.
    /// What the turn came to says whether it had its one post.
    pub(super) async fn turn(
        &self,
        seat: Seat<'_>,
        line: Line,
        reach: Reach,
        cancel: Option<&CancellationToken>,
        job: Option<ScheduledRun>,
        open: impl Fn() -> bool,
    ) -> Driven {
        let driver = seat.driver;
        let thread = seat.thread.clone();
        let _posting = self.room.open_posting(&thread, job);
        let voice = line
            .voice
            .clone()
            .or_else(|| self.room.attached_voice(&thread));
        let subject = line.text.clone();
        let mut told = Told {
            replies: Vec::new(),
            files: Vec::new(),
            with_call: Vec::new(),
            voice: voice.clone(),
            threads: Threads { room: self.room },
            seat,
            open,
        };
        let mut driven = self
            .drive(driver, line.into(), reach, cancel, None, &mut told)
            .await;
        if thread.kind == crate::contract::ThreadKind::Call
            && (told.open)()
            && let Some(capability) = told.seat.capability
        {
            self.room
                .post_call_files(told.seat.persona_id, &thread, &told.files, capability)
                .await;
        }
        if voice.as_ref().is_some_and(|origin| origin.front) && (told.open)() {
            let result_id = super::new_id();
            let text = told
                .replies
                .iter()
                .filter_map(|line| line["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let failed = driven.cancelled || driven.failure.is_some();
            let text = if text.is_empty() {
                driven.failure.clone().unwrap_or_else(|| {
                    if driven.cancelled {
                        "Work stopped; unfinished actions remain unfinished."
                    } else {
                        "Work ended without a written result."
                    }
                    .into()
                })
            } else {
                text
            };
            self.write(
                &thread,
                told.seat.persona_id,
                &TranscriptEvent::Tool {
                    id: result_id.clone(),
                    ts: super::now_ms(),
                    tool_call_id: result_id.clone(),
                    title: "Work result".into(),
                    tool_kind: Some("work_result".into()),
                    status: if failed {
                        crate::contract::ToolStatus::Failed
                    } else {
                        crate::contract::ToolStatus::Completed
                    },
                    locations: None,
                    files: None,
                    output: Some(vec![crate::contract::ToolOutput::Text {
                        text: text.clone(),
                    }]),
                    said: Some(crate::glance::step_said("work_result", "work_result", None)),
                },
            );
            if !failed
                && let Some(origin) = &voice
                && let Some(calls) = lock(&self.room.voice).upgrade()
                && !told.replies.is_empty()
            {
                let spoken = told
                    .replies
                    .iter()
                    .filter_map(|line| line["spoken"].as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let answer = if spoken.is_empty() {
                    text
                } else {
                    format!("<spoken>{spoken}</spoken><written>{text}</written>")
                };
                calls.delivery(
                    told.seat.persona_id,
                    &result_id,
                    told.seat.persona_id,
                    &answer,
                    true,
                    Some(origin),
                );
            }
        }
        if let Some(origin) = voice
            && let Some(voice) = lock(&self.room.voice).upgrade()
        {
            voice.turn_ended(told.seat.persona_id, &origin);
        }
        if (told.open)() {
            self.room
                .post_detached_reply(told.seat.persona_id, &thread, &subject, &told.replies)
                .await;
        }
        driven.posted = self.room.has_posted(&thread);
        driven
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
/// `session/sides.rs`, a job in `session/job_thread.rs`.
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
    fn holder(&self) -> crate::computer::gate::Holder;

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
                    self.let_go_of_computer(occupant.persona_id(), &occupant.holder());
                    if matches!(occupant.end(&self, held, driven), Then::Stop) {
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
