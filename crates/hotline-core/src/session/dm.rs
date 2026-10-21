//! The DM's part of the shared turn loop.
//!
//! The main conversation is a thread like any other, and its turns are taken by
//! [`Room::run_queue`] and [`Threads::drive`] like a work thread's. What is its
//! own is here, because it is what a person talking to a teammate has that no
//! other thread does:
//!
//! - **What a line is.** A [`Wired`] line carries what a call, a delivery or
//!   a person stamped on it: whether it may steer a turn in flight, the user
//!   line it was written as (so the tape can be told when the agent has read
//!   it), and the thread it came from.
//! - **The door of a turn.** A colleague's result is heard once, and one the
//!   exchange has since stopped never; work the agent took up by itself is
//!   read, not prompted.
//! - **The witness.** [`Heard`] writes what the agent does onto the tape,
//!   through the stamps a prompt left, marks
//!   the person's lines read, keeps the session's checkpoint, says a reply to a
//!   call and to the phone, and steers: a line said to a driver that takes
//!   input mid-turn is handed to it when it arrives. On a turn said on a call
//!   to the teammate, the reply's words stream to the call as they arrive,
//!   and the reply is written with the version that was said beside it. The
//!   chat and the phone show every reply's written version without a tag, on
//!   a call or not (see [`crate::voice::spoken`] and
//!   [`super::runner::drive_updates`]).
//! - **The end of a turn.** What the driver did not take is queued again in its
//!   order.
//! - **Posting.** Its turn is open for `post_to_chat`, which refuses it: its
//!   reply is already the chat ([`super::post`]). A job's firing is a turn of
//!   its own thread, never the DM's ([`super::job_thread`]).
//!
//! The chapters the DM keeps are in [`super::chapters`], and its idle clock is
//! the Dm row of [`Room::sweep`].

use super::prompt::PendingTool;
use super::runner::{Driven, Witness, Words};
use super::turns::{Begin, Occupant, Source, Then, Turns};
use super::{Glance, Room, Session, Wired, call_origin, lock};
use crate::contract::{Reach, SessionState, TranscriptEvent};
use crate::contract::{ThreadId, ThreadKind};
use crate::driver::{MessageKind, Update};
use crate::room;
use crate::voice::spoken::versions;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// What the DM keeps from the door of a turn to its end.
pub(super) struct Hold {
    source: Option<Source>,
    reach: Reach,
    /// Whether the line came from a voice, which decides how the reply is said.
    from_voice: bool,
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

    fn holder(&self) -> crate::computer::gate::Holder {
        self.holder.clone()
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
        if let Some(said) = wired.said.clone() {
            lock(&session.unread).push(said);
        }
        let from_voice = wired.spoken;
        let source = if let Some(unprompted) = &wired.unprompted {
            let Some(updates) = lock(&unprompted.0).take() else {
                return Begin::Skip;
            };
            Source::Updates(updates)
        } else if wired.retry {
            Source::Retry {
                text: wired.text,
                attachments: wired.attachments,
            }
        } else {
            Source::Say {
                text: wired.text,
                attachments: wired.attachments,
            }
        };
        Begin::Go(Hold {
            source: Some(source),
            reach,
            from_voice,
            steered: Vec::new(),
        })
    }

    async fn turn(self: &Arc<Self>, room: &Arc<Room>, held: &mut Hold) -> Driven {
        let source = held.source.take().expect("a turn is taken once");
        let _posting = room.open_posting(&self.thread(), None);
        let mut heard = Heard {
            room,
            session: self,
            steered: &mut held.steered,
            from_voice: held.from_voice,
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
        room.turn_ended(session);
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
    /// Whether the line came from a voice, which decides how the reply is said.
    from_voice: bool,
    /// The card the update in hand raises, until its event is written.
    card: Option<(String, crate::push::Waiting)>,
}

impl Heard<'_> {
    /// The call to this teammate the turn was said on, when it was: its
    /// replies are said there as well as shown here.
    fn on_call(&self) -> Option<crate::voice::Origin> {
        lock(&self.session.voice_origin)
            .clone()
            .filter(|origin| origin.direct)
    }
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
        // A turn waiting on its subagents reads as done until the agent works
        // again: a reply is a reply, but a tool, a thought or a card is work.
        if works(update) {
            room.await_subagents(session, false);
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
                    let _ = room::clear_checkpoint(
                        &room.log,
                        &session.persona_id,
                        &session.backend_id,
                        session.account_id.as_deref(),
                    );
                } else {
                    room.checkpoint(session);
                }
            }
            // A phone hears a turn's reply once the driver is done with the
            // line ([`Room::send_glance`]), and a question the agent cannot go
            // on without the moment it is asked.
            Update::Message {
                kind: MessageKind::Agent,
                id,
                text,
            } if !text.trim().is_empty() => {
                // narration::Voice has committed this as an acknowledgement or
                // report. On a call to this teammate it is said now rather than
                // when tool work ends: the words already said as it streamed
                // are its start, and the rest of its spoken version follows.
                let on_call = self.on_call();
                if let Some(origin) = &on_call
                    && let Some(voice) = lock(&room.voice).upgrade()
                {
                    let name = room
                        .persona(&session.persona_id)
                        .map_or_else(|_| "Hotline".into(), |p| p.name);
                    voice.delivery(&session.persona_id, id, &name, text, true, Some(origin));
                }
                // The phone is shown the reply as the chat is, its written
                // version, whether or not this turn was said on a call: the
                // agent may write the tags in any reply once it has been asked
                // to.
                *lock(&session.glance) = Some(Glance {
                    event_id: id.clone(),
                    text: versions(text).written.trim().to_string(),
                });
            }
            // The agent has said what it has to say and the turn stays open
            // only for subagents it started: it reads as done. The reply is
            // heard now, where a turn's end would have said it. Hotline
            // Agent hands its commands to the session before it parks, so a
            // tool still open here is a harness's own, which is work, and the
            // turn goes on reading as it does.
            Update::Parked
                if in_flight
                    .values()
                    .all(|tool| tool.kind == super::jobs::SUBAGENT) =>
            {
                room.send_glance(session, self.from_voice);
                room.turn_ended(session);
                room.await_subagents(session, true);
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

    fn delta(&mut self, kind: MessageKind, message_id: &str, words: Words<'_>, muted: bool) {
        // Words the window types as they come are said as they come, from
        // what the agent wrote, tags and all; a message that may yet turn out
        // to be narration waits to be whole.
        if let Some(origin) = self.on_call()
            && kind == MessageKind::Agent
            && !muted
            && let Some(voice) = lock(&self.room.voice).upgrade()
        {
            voice.reply_delta(&self.session.persona_id, message_id, words.written, &origin);
        }
        if words.shown.is_empty() {
            return;
        }
        let _ = self.room.deltas.send(super::turns::delta_of(
            &ThreadId::dm(&self.session.persona_id),
            kind,
            message_id,
            words.shown,
            muted,
        ));
    }

    fn said_on_call(&self) -> bool {
        self.on_call().is_some()
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
    /// The turn is over, or open only for its subagents: a call to this
    /// teammate that it was said on listens once its reply is said.
    fn turn_ended(&self, session: &Session) {
        if let Some(origin) = lock(&session.voice_origin).clone()
            && let Some(voice) = lock(&self.voice).upgrade()
        {
            voice.turn_ended(&session.persona_id, &origin);
        }
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
            // A turn that was only waiting on its subagents has a line to
            // answer now, so it is working again.
            self.await_subagents(session, false);
        }
    }
}

impl Room {
    /// Whether the teammate's record still names the account this session
    /// started on, signed in as it was then (spec §8.4). A session that
    /// outlived a switch, or a sign-in replaced under it, must not leave a
    /// checkpoint the account's next session would reopen.
    pub(super) fn checkpoint_still_ours(&self, session: &Session) -> bool {
        let Ok(persona) = self.persona(&session.persona_id) else {
            return false;
        };
        if crate::accounts::account_of(&persona) != session.account_id.as_deref() {
            return false;
        }
        let Some(account_id) = &session.account_id else {
            return true;
        };
        room::agent_accounts(&self.log).iter().any(|account| {
            account.id == *account_id && account.login_at == session.account_login_at
        })
    }
}

/// Whether an update is the agent at work. Its words, a notice, its waiting
/// and the end of its turn are not, so a turn waiting on its subagents reads
/// as done through them; the end of the turn clears the wait with the state.
fn works(update: &Update) -> bool {
    !matches!(
        update,
        Update::Parked
            | Update::Turn { .. }
            | Update::Notice { .. }
            | Update::Delta {
                kind: MessageKind::Agent,
                ..
            }
            | Update::Message {
                kind: MessageKind::Agent,
                ..
            }
    )
}
