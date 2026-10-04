//! The DM's part of the shared turn loop.
//!
//! The main conversation is a thread like any other, and its turns are taken by
//! [`Room::run_queue`] and [`Threads::drive`] like a work thread's. What is its
//! own is here, because it is what a person talking to a teammate has that no
//! other thread does:
//!
//! - **What a line is.** A [`Wired`] line carries what a schedule, a call, a
//!   delivery or a person stamped on it: whether it may steer a turn in flight,
//!   the user line it was written as (so the tape can be told when the agent has
//!   read it), the thread it came from, and a firing's authority, which travels
//!   with the queued turn.
//! - **The door of a turn.** A colleague's result is heard once, and one the
//!   exchange has since stopped never; a firing that was cancelled while it
//!   waited is not run; work the agent took up by itself is read, not prompted.
//! - **The witness.** [`Heard`] writes what the agent does onto the tape,
//!   through the stamps a prompt left and the quiet window of a schedule, marks
//!   the person's lines read, keeps the session's checkpoint, says a reply to a
//!   call and to the phone, and steers: a line said to a driver that takes
//!   input mid-turn is handed to it when it arrives.
//! - **The end of a turn.** What the driver did not take is queued again in its
//!   order, and a quiet run that found something is escalated.
//!
//! The chapters the DM keeps are in [`super::chapters`], and its idle clock is
//! the Dm row of [`Room::sweep`].

use super::runner::{Driven, Witness};
use super::turns::{Begin, Occupant, Source, Then, Turns};
use super::{
    Glance, PendingTool, Room, Session, Wired, call_origin, escalation, lock, mark, new_id, now_ms,
    quiet, schedule, timed,
};
use crate::contract::{Reach, Receipt, ScheduledRun, SessionState, StreamDelta, TranscriptEvent};
use crate::driver::{MessageKind, Update};
use crate::room;
use crate::thread::{ThreadId, ThreadKind};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// What the DM keeps from the door of a turn to its end.
pub(super) struct Hold {
    source: Option<Source>,
    reach: Reach,
    /// Whether the line came from a voice, which decides how the reply is said.
    from_voice: bool,
    /// A quiet run's replies are thinking; it is heard only by asking. Armed
    /// for this turn alone.
    quiet_run: Option<ScheduledRun>,
    escalation: Option<Arc<escalation::Escalation>>,
    /// The lines the driver took mid-turn, in the order it took them.
    steered: Vec<Wired>,
}

impl Occupant for Session {
    type Line = Wired;
    type Held = Hold;

    fn queue(&self) -> &Mutex<Turns> {
        &self.turns
    }

    fn persona_id(&self) -> &str {
        &self.persona_id
    }

    fn thread(&self) -> ThreadId {
        ThreadId::dm(&self.persona_id)
    }

    fn answering(self: &Arc<Self>, room: &Room) -> bool {
        self.capability.is_current() && room.current_session(self)
    }

    fn abandon(self: &Arc<Self>) {
        let mut turns = lock(&self.turns);
        turns.waiting.clear();
        turns.running = false;
    }

    async fn begin(self: &Arc<Self>, room: &Arc<Room>, wired: Wired) -> Begin<Hold> {
        let session = self;
        // What a line is, and where it came from, is a field of it. The
        // desk's own calls, which are no thread, are the one origin still
        // read off the id they were written under.
        *lock(&session.voice_origin) = call_origin(&wired);
        // A result that came back from a colleague is heard once: it is
        // consumed as its turn begins, and one the exchange has since
        // stopped is never heard.
        if wired
            .from
            .as_ref()
            .filter(|from| matches!(from.kind, ThreadKind::Pair | ThreadKind::Side))
            .and_then(|from| from.request.as_deref())
            .is_some_and(|request| {
                room.is_exchange_request(request) && !room.begin_exchange_result(request)
            })
        {
            return Begin::Skip;
        }
        room.set_state(session, SessionState::Thinking);
        let reach = room.reach_of(&session.persona_id);
        if !session.answering(room) {
            session.abandon();
            return Begin::Stop;
        }
        if wired.scheduled.as_ref().is_some_and(|run| {
            !schedule::scheduled_run_allowed(&room.log, &session.persona_id, run)
        }) {
            return Begin::Skip;
        }
        if let Some(said) = wired.said.clone() {
            lock(&session.unread).push(said);
        }
        // A quiet run's replies are thinking; it is heard only by asking.
        // Armed for this turn alone, and cleared for every other one.
        let quiet_run = wired
            .scheduled
            .clone()
            .filter(|run| run.quiet == Some(true));
        let escalation = quiet_run
            .as_ref()
            .map(|_| Arc::new(escalation::Escalation::new()));
        let from_voice = wired
            .said
            .as_deref()
            .is_some_and(|id| id.starts_with("voice:"));
        let source = if let Some(unprompted) = &wired.unprompted {
            let Some(updates) = lock(&unprompted.0).take() else {
                return Begin::Skip;
            };
            Source::Updates(updates)
        } else {
            session.driver.escalate_next(
                escalation
                    .clone()
                    .map(|armed| armed as Arc<dyn crate::driver::Escalate>),
            );
            Source::Say {
                text: wired.text,
                attachments: wired.attachments,
            }
        };
        Begin::Go(Hold {
            source: Some(source),
            reach,
            from_voice,
            quiet_run,
            escalation,
            steered: Vec::new(),
        })
    }

    async fn turn(self: &Arc<Self>, room: &Arc<Room>, held: &mut Hold) -> Driven {
        let source = held.source.take().expect("a turn is taken once");
        let mut heard = Heard {
            room,
            session: self,
            steered: &mut held.steered,
            card: None,
        };
        room.threads()
            .drive(
                self.driver.as_ref(),
                source,
                held.reach,
                None,
                Some(&self.input_ready),
                &mut heard,
            )
            .await
    }

    fn end(self: &Arc<Self>, room: &Arc<Room>, held: Hold, driven: Driven) -> Then {
        let session = self;
        room.send_glance(session, held.from_voice);
        if driven.asked {
            // A permission the turn left open is a button nobody is
            // behind. A `request_human` wait is not: the tool is still
            // parked on it, and only the person, the deadline, or a
            // session stop settles that card.
            room.threads()
                .expire_asked(&session.thread(), &session.persona_id);
        }
        let mut steered = held.steered;
        for (text, attachments) in session.driver.take_unconsumed().into_iter().rev() {
            // Drivers return the input's contents, not its tape identity.
            // Match from the end because replay is requeued in reverse;
            // identical inputs must keep their original order and origin.
            let wire = if let Some(index) = steered
                .iter()
                .rposition(|wire| wire.text == text && wire.attachments == attachments)
            {
                steered.remove(index)
            } else {
                let mut wire = Wired::words(text);
                wire.attachments = attachments;
                wire
            };
            lock(&session.turns).waiting.push_front(wire);
        }
        if let (Some(run), Some(note)) = (
            held.quiet_run,
            held.escalation.as_ref().and_then(|armed| armed.take()),
        ) {
            room.escalate(session, run, &note);
        }
        Then::Next
    }

    fn rest(self: &Arc<Self>, room: &Arc<Room>) {
        if !self.answering(room) {
            lock(&self.turns).running = false;
            return;
        }
        room.set_state(self, SessionState::Ready);
        room.attach_computer_when_idle(&self.persona_id);
        room.swap_computer_when_idle(&self.persona_id);
    }
}

/// What the DM does about each thing its agent does in a turn: writes it on the
/// tape as the person's own conversation, and tells whoever is listening.
struct Heard<'a> {
    room: &'a Room,
    session: &'a Arc<Session>,
    steered: &'a mut Vec<Wired>,
    /// The card the update in hand raises, until its event is written.
    card: Option<(String, crate::push::Waiting)>,
}

impl Witness for Heard<'_> {
    fn heard(&mut self, update: &Update, in_flight: &mut HashMap<String, PendingTool>) {
        let (room, session) = (self.room, self.session);
        // A full context mid-turn is the driver's to carry on from: it has
        // rebuilt its history from what this turn committed, under the same
        // preamble and wake. A chapter is the person's unit of work, not the
        // model's window, so it stays open, and with it the collaboration it
        // holds: a handoff in flight keeps its authority (BRO-150).
        if matches!(update, Update::Chapter { .. }) {
            return;
        }
        // Anything the agent produces proves it has what it was handed; a
        // notice can be an error raised before the prompt reached the model.
        if !matches!(update, Update::Notice { .. }) {
            room.mark_read(session);
        }
        match update {
            // The words are told to `delta`; the message that follows makes
            // them durable.
            Update::Delta { .. } => {}
            Update::Turn { stop_reason, .. } => {
                // A cancelled turn leaves tools running; they are marked before
                // the turn is closed, so the transcript never shows a finished
                // turn above a tool still in progress.
                room.fail_in_flight(session, in_flight);
                if !session.driver.checkpoint_valid() || stop_reason == "failed" {
                    lock(&session.pending_checkpoint).take();
                    let _ =
                        room::clear_checkpoint(&room.log, &session.persona_id, &session.backend_id);
                } else {
                    room.checkpoint(session);
                }
            }
            // A phone hears a turn's reply once the driver is done with the
            // line ([`Room::send_glance`]), and a question the agent cannot go
            // on without the moment it is asked. A quiet schedule's reply is
            // demoted to a thought and says nothing anywhere.
            Update::Message {
                kind: MessageKind::Agent,
                id,
                text,
            } if !quiet::mutes_deltas(lock(&session.quiet).as_ref(), now_ms())
                && !text.trim().is_empty() =>
            {
                // narration::Voice has committed this as an acknowledgement or
                // report. Direct callers hear it now rather than waiting for
                // tool work to end, unless the call has a voice of its own that
                // already acknowledged it and answers how it is going; then
                // only the turn's reply is said.
                if let Some(origin) = lock(&session.voice_origin).clone()
                    && origin.direct
                    && let Some(voice) = lock(&room.voice).upgrade()
                    && !voice.fronted(&origin)
                {
                    let name = room
                        .persona(&session.persona_id)
                        .map(|p| p.name)
                        .unwrap_or_else(|_| "Hotline".into());
                    voice.delivery(&session.persona_id, id, &name, text, true, Some(&origin));
                }
                *lock(&session.glance) = Some(Glance {
                    event_id: id.clone(),
                    text: text.trim().to_string(),
                });
            }
            Update::Permission {
                request_id,
                title,
                options,
            } => {
                self.card = Some((
                    title.clone(),
                    crate::push::Waiting::Permission {
                        request_id: request_id.clone(),
                        options: options.clone(),
                    },
                ));
            }
            _ => {}
        }
    }

    fn delta(&mut self, kind: MessageKind, message_id: &str, text: &str, muted: bool) {
        // A muted turn must not run the writing indicator for a message
        // that will never land, so the delta is demoted with the event it
        // is building; and after the acknowledgement a message may turn
        // out to be narration, so it streams as thinking too.
        let muted = muted
            || (kind == MessageKind::Agent
                && quiet::mutes_deltas(lock(&self.session.quiet).as_ref(), now_ms()));
        let persona_id = self.session.persona_id.clone();
        let (message_id, text) = (message_id.to_string(), text.to_string());
        let _ = self.room.deltas.send(match kind {
            MessageKind::Agent if !muted => StreamDelta::AgentDelta {
                persona_id,
                message_id,
                text,
            },
            _ => StreamDelta::ThoughtDelta {
                persona_id,
                message_id,
                text,
            },
        });
    }

    fn write(&mut self, event: TranscriptEvent, asked: bool) {
        let card = if asked { self.card.take() } else { None };
        self.room.append(self.session, event);
        if let Some((title, waiting)) = card {
            self.room.push.notify(
                &self.room.needs_you(&self.session.persona_id),
                &title,
                &self.session.persona_id,
                Some(waiting),
            );
        }
    }

    fn steer(&mut self) {
        self.room.steer_waiting(self.session, self.steered);
    }
}

impl Room {
    /// A quiet run found something: the teammate is prompted with it in the
    /// open, next, so it answers the person as any reply is answered. The
    /// line is stamped with the job but not quiet, so no window opens over
    /// the answer.
    fn escalate(&self, session: &Session, run: ScheduledRun, note: &str) {
        let run = ScheduledRun { quiet: None, ..run };
        let ts = now_ms();
        let mut wire = Wired::words(timed(ts, &escalation::follow_up(&run, note)));
        wire.scheduled = Some(run.clone());
        mark(&session.pending_scheduled, run);
        let id = new_id();
        self.append(
            session,
            TranscriptEvent::User {
                id: id.clone(),
                ts,
                text: note.to_string(),
                attachments: None,
                reactions: None,
                reply_to: None,
                scheduled: None,
                ring: None,
                receipt: Some(Receipt::Sent),
                client: None,
            },
        );
        wire.said = Some(id);
        lock(&session.turns).waiting.push_front(wire);
    }

    fn steer_waiting(&self, session: &Arc<Session>, steered_inputs: &mut Vec<Wired>) {
        if !session.capability.is_current() || !self.current_session(session) {
            return;
        }
        let mut turns = lock(&session.turns);
        let mut steered = false;
        while let Some(index) = turns.waiting.iter().position(|wire| wire.steer) {
            let wire = &turns.waiting[index];
            if !session
                .driver
                .steer(wire.text.clone(), wire.attachments.clone())
            {
                break;
            }
            *lock(&session.voice_origin) = call_origin(wire);
            if let Some(said) = wire.said.clone() {
                lock(&session.unread).push(said);
            }
            steered_inputs.push(
                turns
                    .waiting
                    .remove(index)
                    .expect("steered input is queued"),
            );
            steered = true;
        }
        drop(turns);
        if steered {
            // A new instruction is neither an answer nor permission. Release
            // an obsolete human wait so the model can reconsider the request.
            self.release_human_waits(&session.persona_id);
        }
    }
}
