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
//!   exchange has since stopped never; a colleague's question is answered
//!   only while its request still runs, in a turn that takes in nothing the
//!   person says, and its reply goes back to the asker; work the agent took
//!   up by itself is read, not prompted.
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
//! - **A quiet turn.** A turn nobody started is read by nobody: one the
//!   inbox woke (`Wired::quiet`), one a line of Hotline's that work came back
//!   began (`Wired::back`: a command that ended, the asides' wake, a thread's
//!   post), one the agent began by itself for its own background work
//!   (`Wired::unprompted`), and the rest of any turn once it has replied and
//!   parked for its helpers (`Update::Parked`), when what wakes it again is a
//!   helper's or a command's result. While it stays quiet the witness writes
//!   what the agent says as thinking marked `said`, the path under the turn,
//!   streams it as thinking, and tells no call and no phone; the turn may
//!   post once, as a quiet job's may. The person's own line steered into it
//!   ends the quiet, and the rest of the turn is a reply to them. A turn said
//!   on a call is never made quiet: the person is on the line.
//! - **The pick-up.** A turn's end arms one quiet turn offering what the
//!   teammate set aside in its inbox, once the conversation is idle
//!   ([`super::inbox::PickUp`]); the beginning of any turn spends it, and the
//!   end of a pick-up's own turn, or of one the person stopped, arms nothing.
//!
//! The chapters the DM keeps are in [`super::chapters`], and its idle clock is
//! the Dm row of [`Room::sweep`].

use super::asides::result_of;
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
    /// The colleague's question this turn answers.
    request: Option<String>,
    /// Nobody started the turn (the inbox, or work that came back): nobody
    /// reads it until the person speaks.
    quiet: bool,
    /// The inbox items that woke a quiet turn: those its line offered, or
    /// those its index offered for the first time.
    woke: Vec<String>,
    offered: Vec<String>,
    inline: Option<String>,
    /// The turn is a pick-up of what the teammate set aside, which no other
    /// pick-up follows.
    pick_up: bool,
    /// The person's emoji alone began the turn: an emoji alone said back is
    /// a reaction to it, not a message.
    reaction: bool,
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
        // consumed once its turn is certain to run, and one the exchange has
        // since stopped is never heard. A turn refused below leaves it to be
        // heard by the one that is not.
        let result = wired
            .from
            .as_ref()
            .filter(|from| matches!(from.kind, ThreadKind::Pair | ThreadKind::Side))
            .and_then(|from| from.request.clone())
            .filter(|request| room.is_exchange_request(request));
        if result
            .as_deref()
            .is_some_and(|request| !room.exchange_result_waits(request))
        {
            return Begin::Skip;
        }
        // A colleague's question is answered only while its request still
        // runs: one stopped while it waited here is never taken up.
        let request = wired.request.as_ref().map(|line| line.request.clone());
        if let Some(request) = &request
            && let Err(error) = room.begin_request(request)
        {
            eprintln!("question not taken up: {error}");
            return Begin::Skip;
        }
        room.set_state(session, SessionState::Thinking);
        let reach = room.reach_of(&session.persona_id);
        if !session.answering(room) {
            session.abandon();
            // A question begun is ended with the turn that would have
            // answered it, not left running as begun.
            if let Some(request) = &request {
                room.refuse_request(request, super::peers::NOT_TAKEN_UP);
            }
            return Begin::Stop;
        }
        if let Some(said) = wired.said.clone() {
            lock(&session.unread).push(said);
        }
        let from_voice = wired.spoken;
        let reaction = wired.person == Some(super::marks::Person::Reaction);
        // Work that came back, or the agent's own work between turns, begins
        // a turn nobody started: quiet as the inbox's is, though no inbox
        // limit counts it. One said on a call is heard there.
        let returning = (wired.back || wired.unprompted.is_some()) && wired.voice.is_none();
        let pick_up = wired.pick_up;
        let mut woke = wired.woke.clone();
        let mut offered = Vec::new();
        let mut inline = None;
        // A pick-up whose items were all sorted meanwhile has nothing to offer.
        if pick_up && room.set_aside(&session.persona_id).is_empty() {
            return Begin::Skip;
        }
        // A quiet turn past the teammate's hourly limit is not begun: it is
        // paused with a card (`inbox/limit.rs`).
        if wired.quiet && !room.quiet_turn_may_begin(&session.persona_id) {
            return Begin::Skip;
        }
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
            // Results that came back together are one turn: the ones queued
            // behind this result are heard with it.
            let is_result = result_of(&wired).is_some();
            // An inbox notice its driver did not take mid-turn begins a turn
            // of its own: the quiet one the idle wake begins, whose line says
            // who reads it. The items it named are in the index above.
            let mut text = if wired.quiet && wired.steer {
                super::timed(super::now_ms(), super::inbox::QUIET_WAKE_LINE)
            } else {
                wired.text
            };
            if is_result {
                for behind in room.results_behind(session) {
                    if let Some(said) = behind.said {
                        lock(&session.unread).push(said);
                    }
                    text = format!("{text}\n\n{}", behind.text);
                }
            }
            // Where the teammate's other threads stand, what came into its
            // inbox and what it was left, open the line. A line that only woke
            // it is spent once a turn before it has heard what woke it.
            let asides = room.hear_asides(&session.persona_id, &session.thread());
            let inbox = room.inbox_told(
                &session.persona_id,
                &session.thread(),
                wired.quiet || returning,
                // The loud wake is a finished ask; an aside's wake is also a return.
                wired.wake
                    && !wired.steer
                    && (!wired.back || text.ends_with(super::inbox::QUIET_WAKE_LINE)),
            );
            if asides.is_empty() && !inbox.news && wired.wake {
                return Begin::Skip;
            }
            woke.extend(inbox.offered.iter().cloned());
            offered = inbox.offered;
            inline = inbox.inline;
            // Which sites it holds the person's cookies for, when that changed.
            text = room.with_cookie_news(&session.told_cookies, &session.persona_id, text);
            if let Some(index) = room.thread_index(
                &session.persona_id,
                &session.thread(),
                &asides,
                inbox.text.as_deref(),
                Some(&session.indexed),
            ) {
                text = format!("{index}\n\n{text}");
            }
            // Work that came back, or a line that only wakes the teammate:
            // nobody waits on its reply, and Hotline says so with the facts
            // of the moment.
            if wired.back {
                text = format!(
                    "{text}\n\n{}",
                    room.return_note(&session.persona_id, &session.thread())
                );
            }
            // The person's emoji alone is a reaction, said so, and waits on
            // no reply ([`super::marks`]).
            Source::Say {
                text: super::marks::at_the_door(text, wired.person),
                attachments: wired.attachments,
                asked: !wired.back && !reaction,
            }
        };
        // Nothing refuses the turn from here: the result it hears is spent.
        if let Some(result) = &result
            && !room.begin_exchange_result(result)
        {
            return Begin::Skip;
        }
        // The person's Stop or Interrupt from here on is this turn's.
        lock(&session.cut).take();
        // This turn hears the inbox in its index: a pick-up the last one
        // armed is spent.
        lock(&session.pick_up).began();
        Begin::Go(Hold {
            source: Some(source),
            reach,
            from_voice,
            steered: Vec::new(),
            request,
            quiet: wired.quiet || returning,
            woke,
            offered,
            inline,
            pick_up,
            reaction,
        })
    }

    async fn turn(self: &Arc<Self>, room: &Arc<Room>, held: &mut Hold) -> Driven {
        let source = held.source.take().expect("a turn is taken once");
        let _posting = room.open_posting(&self.thread(), None);
        if held.quiet {
            let thread = self.thread();
            room.quiet_turn(&thread, &held.woke);
            if matches!(&source, Source::Updates(_)) {
                self.driver
                    .note_unprompted(room.return_note(&self.persona_id, &thread))
                    .await;
            }
        }
        let mut heard = Heard {
            room,
            session: self,
            steered: &mut held.steered,
            card: None,
            answering_a_question: held.request.is_some(),
            reaction: held.reaction,
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
        let finished = !driven.cancelled && driven.stop_reason.as_deref() == Some("end_turn");
        room.send_glance(session, held.from_voice, finished);
        room.turn_ended(session);
        room.read_plan_after_turn(session);
        if driven.asked {
            // A permission the turn left open is a button nobody is behind.
            room.threads()
                .expire_asked(&session.thread(), &session.persona_id);
        }
        let mut steered = held.steered;
        let mut handed_back = Vec::new();
        for (text, attachments) in session.driver.take_unconsumed().into_iter().rev() {
            // Drivers return the input's contents, not its tape identity.
            // Match from the end because replay is requeued in reverse;
            // identical inputs must keep their original order and origin.
            let wire = if let Some(index) = steered.iter().rposition(|wire| {
                super::marks::steered(&wire.text, wire.person) == text
                    && wire.attachments == attachments
            }) {
                steered.remove(index)
            } else {
                let mut wire = Wired::words(text);
                wire.attachments = attachments;
                wire
            };
            handed_back.push(wire);
        }
        {
            let mut turns = lock(&session.turns);
            if room.closing() {
                drop(turns);
                // `handed_back` is last line first.
                handed_back.reverse();
                room.keep_late(&session.persona_id, handed_back);
            } else {
                for wire in handed_back {
                    turns.waiting.push_front(wire);
                }
            }
        }
        let cut = lock(&session.cut).take();
        let failed_wake = room.settle_inbox_offer(
            &session.persona_id,
            &held.offered,
            held.inline.as_deref(),
            &driven,
            cut,
        );
        // Whether the conversation, once idle, may be offered what it set
        // aside: never after a pick-up's own turn, so pick-ups cannot chain,
        // and never after a turn the person stopped or that did not finish.
        let may_pick_up = !failed_wake
            && !held.pick_up
            && cut.is_none()
            && !super::exchanges::cut_short(&driven)
            && driven.failure.is_none();
        lock(&session.pick_up).ended(may_pick_up);
        if let Some(request) = &held.request {
            room.finish_request(request, &driven, cut);
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
        room.rebuild_for_sign_in_when_idle(self);
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
    /// Whether the turn answers a colleague's question, whose whole reply
    /// goes to them: nothing the person says is steered into it.
    answering_a_question: bool,
    /// The person's emoji alone began the turn ([`Hold::reaction`]).
    reaction: bool,
}

impl Heard<'_> {
    /// The call to this teammate the turn was said on, when it was: its
    /// replies are said there as well as shown here.
    fn on_call(&self) -> Option<crate::voice::Origin> {
        lock(&self.session.voice_origin)
            .clone()
            .filter(|origin| origin.direct)
    }

    /// Whether nobody reads what the agent says now: the inbox woke the
    /// turn and the person has not spoken into it.
    fn quiet(&self) -> bool {
        self.room.is_quiet(&self.session.thread())
    }

    /// Whether `text` is an emoji said back to the person's emoji alone:
    /// a reaction on their line, never a bubble. Claude Code makes its model
    /// write something after an empty answer, and to a thumbs up that is
    /// often a thumbs up.
    fn reacts_back(&self, text: &str) -> bool {
        self.reaction && super::marks::emoji_only(text)
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
                    let _ =
                        room::clear_checkpoint(&room.log, &session.persona_id, &session.backend_id);
                } else {
                    room.checkpoint(session);
                }
            }
            // A phone hears a turn's reply once the turn is over
            // ([`Room::send_glance`]), and a question the agent cannot go on
            // without the moment it is asked.
            // A quiet turn's words are its path: no call says them and no
            // phone is told them.
            Update::Message {
                kind: MessageKind::Agent,
                id,
                text,
            } if !text.trim().is_empty() && !self.quiet() && !self.reacts_back(text) => {
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
                    before_wait: false,
                });
            }
            // The agent has said what it has to say and the turn stays open
            // only for subagents it started: it reads as done. A call hears
            // the reply now, where a turn's end would have said it; the phone
            // hears the turn once, at its true end, so a job that waits on
            // four helpers is one notification and not one per report.
            // Hotline Agent hands its commands to the session before it
            // parks, so a tool still open here is a harness's own, which is
            // work, and the turn goes on reading as it does.
            Update::Parked
                if in_flight
                    .values()
                    .all(|tool| tool.kind == super::jobs::SUBAGENT) =>
            {
                room.hold_glance(session);
                room.turn_ended(session);
                room.await_subagents(session, true);
                // It has replied: what wakes it next is a helper or a command
                // coming back, which nobody waits on, so the rest of the turn
                // is quiet until the person speaks into it. A colleague's
                // question is answered to them, and a call hears its turn.
                if !self.answering_a_question && self.on_call().is_none() {
                    room.set_quiet(&session.thread(), true);
                }
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
        // A quiet turn's words stream as the thinking they are written as,
        // and a turn the person's emoji began streams none as a bubble that
        // may yet be a reaction.
        let muted = muted || self.quiet() || self.reaction;
        // Words the window types as they come are said as they come, from
        // what the agent wrote, tags and all; a message that may yet turn out
        // to be narration waits to be whole.
        if let Some(origin) = self.on_call()
            && let Some(voice) = lock(&self.room.voice).upgrade()
        {
            if kind == MessageKind::Thought {
                voice.thinking_delta(&self.session.persona_id, &origin);
            } else if kind == MessageKind::Agent && !muted {
                voice.reply_delta(&self.session.persona_id, message_id, words.written, &origin);
            }
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
        // An emoji said back to the person's emoji is a reaction on their
        // line; one that cannot be put there is written as it was said.
        if let TranscriptEvent::Agent { text, .. } = &event
            && self.reacts_back(text)
            && self
                .room
                .react(&self.session.persona_id, text.trim())
                .is_ok()
        {
            return;
        }
        // A quiet turn's words are no reply: they are kept as thinking, the
        // path under the turn, which no rail, phone or call takes as one.
        let event = match event {
            TranscriptEvent::Agent { id, ts, text, .. } if self.quiet() => {
                TranscriptEvent::Thought {
                    id,
                    ts,
                    text,
                    said: Some(true),
                }
            }
            // A card the agent raises in a quiet turn one item woke names it:
            // a colleague's message, not the person, is behind it.
            TranscriptEvent::Permission {
                from,
                id,
                ts,
                request_id,
                title,
                options,
                decision: None,
                decided_option_name,
                from_item: None,
            } if self.quiet() => TranscriptEvent::Permission {
                from,
                id,
                ts,
                request_id,
                title,
                options,
                decision: None,
                decided_option_name,
                from_item: self
                    .room
                    .prompted_by(&ThreadId::dm(&self.session.persona_id)),
            },
            // A turn quiet to its end is one the person was not part of.
            TranscriptEvent::Turn {
                id,
                ts,
                stop_reason,
                usage,
                ..
            } if self.quiet() => TranscriptEvent::Turn {
                id,
                ts,
                stop_reason,
                usage,
                quiet: Some(true),
            },
            event => event,
        };
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

    fn steer(&mut self) -> bool {
        !self.answering_a_question && self.room.steer_waiting(self.session, self.steered)
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

    /// Hands the driver the lines waiting that may join the turn in flight,
    /// the person's with the mark of the moment ([`super::marks`]), and says
    /// whether one of them was the person's.
    fn steer_waiting(&self, session: &Arc<Session>, steered_inputs: &mut Vec<Wired>) -> bool {
        if !session.capability.is_current() || !self.current_session(session) {
            return false;
        }
        let taken = lock(&session.turns).steer_into(
            session.driver.as_ref(),
            |wire| wire.steer,
            |wire| {
                (
                    super::marks::steered(&wire.text, wire.person),
                    wire.attachments.clone(),
                )
            },
        );
        if taken.is_empty() {
            return false;
        }
        let person = taken.iter().any(|wire| wire.person.is_some());
        for wire in &taken {
            *lock(&session.voice_origin) = call_origin(wire);
            if let Some(said) = wire.said.clone() {
                lock(&session.unread).push(said);
            }
        }
        // A line that is more than an inbox notice is the person's: a quiet
        // turn they have spoken into answers them out loud from here on.
        if taken.iter().any(|wire| !wire.wake) {
            self.set_quiet(&session.thread(), false);
        }
        steered_inputs.extend(taken);
        // A turn that was only waiting on its subagents has a line to
        // answer now, so it is working again.
        self.await_subagents(session, false);
        person
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
