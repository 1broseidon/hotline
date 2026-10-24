//! Side threads: the same teammate, borrowed for a second conversation beside
//! the main one.
//!
//! The person starts one beside a teammate's main conversation, and it runs
//! in parallel, in a context of its own, about a topic they give it. It is a
//! working session, not an errand: it may run through many requests over an
//! afternoon, and what makes it a side thread is that nothing it says reaches
//! the main conversation's context. They talk with the teammate in it as they
//! would anywhere.
//!
//! A thread is in one of three states. **Live**: an agent is running it.
//! **Parked**: still open, its agent let go of because nobody spoke in it for a
//! few hours, the desk restarted, or a newer thread needed its place; saying
//! something in it brings it back by itself. **Archived**: ended on purpose, by
//! the person pressing Archive or the teammate saying the person is done, and
//! read-only until the person presses Continue. Neither parking nor archiving
//! loses anything: the thread's stream is kept whole, and the agent is reopened
//! from it.
//!
//! Four records come out of one:
//!
//! - **The stream.** [`StreamId::Side`], `sides/<id>.jsonl`: the task, what the
//!   teammate and the person said, its tool calls and cards. Never on the
//!   teammate's tape, so the main conversation is not interrupted by it and
//!   the teammate's main context never reads it unasked: what was said in it
//!   is indexed under the teammate, so `search_thread` can find it, naming the
//!   thread. Written through [`super::threads::Threads::write`], so a card
//!   raised in it reaches the phone and the roster's `waiting`. Live words arrive as
//!   [`StreamDelta::ThreadDelta`], addressed by the thread's id.
//! - **The marker.** One [`TranscriptEvent::Side`] line on the teammate's tape,
//!   written again under the same id as the thread goes: "started a side
//!   thread", then a title and one-line result with Open once it is archived,
//!   with the closing handoff note when a model wrote one. The same line heads
//!   the stream, so the thread says what it is to whoever opens it, and it
//!   holds the agent's session id once a turn has completed. Archived, it is
//!   also what `search_thread` finds of the thread.
//! - **The roster entry.** [`RunningSide`], while it is live, which is what the
//!   conversation header draws its chip from. Like a subagent, nothing about
//!   it survives a restart; the marker does.
//! - **The driver.** A second agent for the teammate, built by
//!   [`Room::thread_agent`] like any thread's: no checkpoint of the teammate's
//!   reopened, so it never lands in the main conversation, on either harness.
//!   Its turns run on the shared [`super::turns::Turns`]. A new thread is told the
//!   person's task, the main conversation's last handoff note and its last few
//!   lines, and that another thread of itself is working in the same folder.
//!   One that is brought back reopens its own saved session when the harness
//!   can; otherwise Hotline Agent is seeded with the thread's stream as its
//!   history and an ACP child is given a compact transcript of it.
//!
//! Authority is the thread's own lease, not the teammate's current session's:
//! a chapter rotating in the main conversation restarts that session without
//! ending the thread. What ends it is what ends the teammate's authority —
//! the person stopping the teammate, a policy change, its removal — and each
//! of those archives its threads here, as `stopped`; whatever grants the
//! teammate has when a thread is brought back are the ones it gets.
//!
//! The computer goes to the main session. Two agents driving one desktop is a
//! fight nobody wins, so a thread never has it and is told so; the working
//! folder is shared, and it is told that too.

use super::agent::{Opening, lease_of};
use super::exchanges::Finished;
use super::runner::Driven;
use super::turns::{Begin, Line, Occupant, RequestLine, Seat, Then, Turns};
use super::{Room, lock, new_id, now_ms, timed_from};
use crate::contract::{
    Attachment, DeliveryCause, DeliveryFrom, NoticeLevel, PermissionOption, Persona, Receipt,
    RunningSide, SharedSecret, SideEnd, SideStatus, SideThreadSummary, TranscriptEvent,
};
use crate::contract::{End, ThreadId, ThreadKind};
use crate::driver::{CapabilityLease, Driver};
use crate::log::StreamId;
use crate::thread::{AgentBinding, Link, Opener, ThreadState};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// The most work threads one teammate may have live at once, whoever opened
/// them: the person beside the DM or a colleague handing work over. It caps
/// agents, not threads: parked and archived ones cost nothing and are not
/// counted, and the DM's own session is not one of them.
pub const MAX_LIVE: usize = 3;

/// The label a task is cut to for the chip and the marker.
pub(super) const TITLE_CHARS: usize = 60;

/// The longest one-line result a thread keeps.
const RESULT_CHARS: usize = 200;

/// One live side thread.
pub(super) struct LiveSide {
    roster: Mutex<Option<crate::store::previews::live::Live>>,
    pub(super) id: String,
    pub(super) thread: ThreadId,
    persona_id: String,
    /// Empty until the person first says something in a thread that was
    /// opened without a task; that line names it.
    title: Mutex<String>,
    started: i64,
    /// The harness the agent runs on, which is whose session ids `saved` holds.
    backend_id: String,
    /// The teammate that handed this work over, when one did.
    opener: Option<Opener>,
    /// The request the turn in flight is answering, until its result is
    /// saved. A work thread opened by the person has none.
    request: Mutex<Option<String>>,
    /// The agent, once it is up. The thread is published before its agent is
    /// built, because starting one takes seconds, so until this is set the
    /// thread is starting: lines said in it wait in `turns`, and there is
    /// nothing yet to cancel or answer a card.
    up: OnceLock<Up>,
    /// The thread's own authority. Revoking it ends every tool handle the
    /// agent holds.
    capability: CapabilityLease,
    turns: Mutex<Turns<Line>>,
    /// When the person last said something, or the teammate last finished.
    last_used: Mutex<i64>,
    /// The deliveries this agent has been handed, so that handing one over
    /// again is heard once. A restart or a wake is a new agent, with none.
    dispatched: Mutex<std::collections::HashSet<String>>,
    /// Set once the thread is archived; nothing more is written after it.
    closed: AtomicBool,
    /// The person pressed Stop on the turn in flight.
    stopped_by_person: AtomicBool,
    /// What `archive_thread` said, held until the turn it was said in ends.
    archive_note: Mutex<Option<String>>,
    /// The agent's own id for this conversation, as the driver reports it.
    reported: Mutex<Option<String>>,
    /// The id the marker holds, which is what a later start reopens. Written
    /// only once a turn has completed on the session: some agents issue an id
    /// they cannot reopen until a prompt has committed.
    saved: Mutex<Option<String>>,
}

/// What a thread holds once its agent is up.
struct Up {
    driver: Arc<dyn Driver>,
    /// What the agent holds the teammate's computer under.
    holder: crate::computer::gate::Holder,
}

impl LiveSide {
    /// The agent, or nothing while the thread is still starting.
    fn driver(&self) -> Option<&Arc<dyn Driver>> {
        self.up.get().map(|up| &up.driver)
    }

    /// A delivery's turn has begun: it is read, and a restart does not hand it
    /// over again.
    fn heard(&self, room: &Room, id: &str) {
        let stream = StreamId::of_thread(&self.thread).expect("work has a stream");
        let Some(event) = room
            .log
            .load(&stream)
            .into_iter()
            .rev()
            .find(|event| event["id"] == id)
            .and_then(|event| serde_json::from_value::<TranscriptEvent>(event).ok())
        else {
            return;
        };
        self.say(room, &super::peers::stamped(event, Receipt::Read));
    }

    pub(super) fn working(&self) -> bool {
        lock(&self.turns).running
    }

    pub(super) fn last_used(&self) -> i64 {
        *lock(&self.last_used)
    }

    /// The request the turn in flight answers, if it is one.
    pub(super) fn request(&self) -> Option<String> {
        lock(&self.request).clone()
    }

    /// One event onto the thread, through the room's write path. Once the
    /// thread is archived nothing more lands.
    fn say(&self, room: &Room, event: &impl serde::Serialize) {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        room.threads()
            .write(&self.thread.clone(), &self.persona_id, event);
    }
}

/// What a work thread keeps from the door of a turn to its end: the line, until
/// it is handed to the driver.
pub(super) struct WorkTurn {
    line: Option<Line>,
}

impl Occupant for LiveSide {
    type Line = Line;
    type Held = WorkTurn;

    fn queue(&self) -> &Mutex<Turns<Line>> {
        &self.turns
    }

    fn persona_id(&self) -> &str {
        &self.persona_id
    }

    fn thread(&self) -> ThreadId {
        self.thread.clone()
    }

    fn holder(&self) -> crate::computer::gate::Holder {
        self.up
            .get()
            .expect("turns run only once the agent is up")
            .holder
            .clone()
    }

    fn answering(self: &Arc<Self>, _room: &Room) -> bool {
        !self.closed.load(Ordering::SeqCst) && self.capability.check().is_ok()
    }

    /// A request's turn is the exchange's: it begins only while the request is
    /// still running, and its result is saved when it ends.
    async fn begin(self: &Arc<Self>, room: &Arc<Room>, mut line: Line) -> Begin<WorkTurn> {
        // A result that came back from a colleague is heard once: it is
        // consumed as its turn begins, and one the exchange has since stopped,
        // or whose authority has been revoked, is never heard, however long
        // it waited behind the turn in flight.
        if line
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
        if let Some(RequestLine {
            request,
            answer: Some(answer),
        }) = &line.request
            && let Err(error) = room.resume_handoff_answer(request, answer).await
        {
            // A colleague's answer is still heard once its handoff has
            // stopped waiting on it, as an ordinary turn; a card's is not.
            if !room.is_exchange_request(answer) {
                eprintln!("handoff answer not started: {error}");
                return Begin::Skip;
            }
            line.request = None;
        }
        if let Some(request_line) = &line.request {
            if let Err(error) = room.begin_request(&request_line.request) {
                eprintln!("handoff not started: {error}");
                return Begin::Skip;
            }
            *lock(&self.request) = Some(request_line.request.clone());
        }
        if let Some(id) = &line.delivery {
            self.heard(room, id);
        }
        if let Some(heard) = room.take_heard(&self.thread) {
            line.text = format!("{heard}\n\n{}", line.text);
        } else if line.wake {
            return Begin::Skip;
        }
        self.stopped_by_person.store(false, Ordering::SeqCst);
        Begin::Go(WorkTurn { line: Some(line) })
    }

    async fn turn(self: &Arc<Self>, room: &Arc<Room>, held: &mut WorkTurn) -> Driven {
        let thread = self.thread();
        let line = held.line.take().expect("a turn is taken once");
        let driver = self.driver().expect("turns run only once the agent is up");
        room.threads()
            .turn(
                Seat {
                    capability: Some(&self.capability),
                    thread: &thread,
                    persona_id: &self.persona_id,
                    driver: driver.as_ref(),
                },
                line,
                room.reach_of(&self.persona_id),
                None,
                None,
                || !self.closed.load(Ordering::SeqCst),
            )
            .await
    }

    fn end(self: &Arc<Self>, room: &Arc<Room>, _held: WorkTurn, driven: Driven) -> Then {
        let side = self;
        *lock(&side.last_used) = now_ms();
        if side.closed.load(Ordering::SeqCst) {
            // A thread ended under a turn still settles the ask it answered.
            if let Some(request) = lock(&side.request).take() {
                room.answer_question(&request, &driven);
            }
            return Then::Stop;
        }
        if let Some(driver) = side.driver() {
            let unconsumed = driver.take_unconsumed();
            let cancelled = driven.stop_reason.as_deref().is_some_and(|reason| {
                matches!(reason, "cancelled" | "canceled" | "aborted" | "revoked")
            });
            for (text, attachments) in unconsumed.into_iter().rev().filter(|_| !cancelled) {
                lock(&side.turns).waiting.push_front(Line {
                    voice: room.attached_voice(&side.thread),
                    text,
                    attachments,
                    request: None,
                    from: None,
                    delivery: None,
                    wake: false,
                });
            }
        }
        room.remember_session(side, &driven);
        // A card the turn left open is a button nobody is behind.
        if driven.asked {
            room.threads()
                .expire_asked(&side.thread(), &side.persona_id);
        }
        let said_so = lock(&side.archive_note).take();
        let request = lock(&side.request).take();
        if let Some(request) = request {
            // The handoff's result is what the teammate said when it was
            // done. The thread closes with it unless the turn stopped to
            // wait for the person or a colleague, in which case it is the
            // answer that carries it on, or another handoff is already
            // waiting in it, which carries it on now.
            let failed = driven.stop_reason.as_deref().is_none_or(|reason| {
                matches!(
                    reason,
                    "failed" | "cancelled" | "canceled" | "aborted" | "revoked"
                )
            });
            let reply = if failed && driven.replies.is_empty() {
                room.ended_without_result(&request).to_string()
            } else {
                driven.replies.join("\n\n")
            };
            let stopped = side.stopped_by_person.swap(false, Ordering::SeqCst)
                && room.end_stopped_handoff(&request, &driven);
            let finished = if stopped {
                Finished::Gone
            } else {
                room.finish_request(&request, reply, failed)
            };
            let answered = Some((request.as_str(), finished));
            room.report_turn(&side.id, side.opener.as_ref(), answered, &driven);
            // Answering an ask never closes the thread: only a handoff's
            // result does.
            if matches!(finished, Finished::Result | Finished::Gone)
                && lock(&side.turns).waiting.is_empty()
            {
                let by = match (stopped, failed) {
                    (true, _) => End::Stopped,
                    (false, true) => End::Failed,
                    (false, false) => End::Agent,
                };
                room.finish_side(side, by, None);
                return Then::Stop;
            }
        } else {
            room.report_turn(&side.id, side.opener.as_ref(), None, &driven);
            if let Some(summary) = said_so {
                room.finish_side(side, End::Agent, Some(summary));
                return Then::Stop;
            }
        }
        Then::Next
    }

    fn rest(self: &Arc<Self>, room: &Arc<Room>) {
        lock(&self.turns).running = false;
        room.wake_when_rested(&self.id, self.opener.as_ref());
        let _ = room.info_changes.send(room.info(&self.persona_id));
    }
}

#[derive(Default)]
struct Inner {
    live: HashMap<String, Arc<LiveSide>>,
    /// Starts in flight, so two at once cannot both take the last place.
    starting: HashMap<String, usize>,
}

/// Every live side thread the room holds.
#[derive(Default)]
pub(super) struct Sides {
    inner: Mutex<Inner>,
    /// One thread is brought back at a time, so two lines said to the same
    /// parked thread at once start one agent between them.
    pub(super) waking: tokio::sync::Mutex<()>,
}

impl Sides {
    /// Takes one of a teammate's places for a start that is under way. When
    /// they are all taken, the thread that has waited longest for the person
    /// is handed back to be parked: parking loses nothing, so it is the
    /// kinder answer than a refusal. Only when every place is mid-turn, with
    /// nothing to let go of, is there no place to be had.
    fn reserve(&self, persona_id: &str, kind: ThreadKind) -> Result<Option<Arc<LiveSide>>, String> {
        let mut inner = lock(&self.inner);
        let limit = if kind == ThreadKind::Call {
            1
        } else {
            MAX_LIVE
        };
        let reservation = format!("{}:{persona_id}", kind.name());
        let live: Vec<Arc<LiveSide>> = inner
            .live
            .values()
            .filter(|side| side.persona_id == persona_id && side.thread.kind == kind)
            .cloned()
            .collect();
        let starting = inner.starting.get(&reservation).copied().unwrap_or(0);
        let freed = if live.len() + starting >= limit {
            let idlest = live
                .iter()
                .filter(|side| !side.working())
                .min_by_key(|side| (*lock(&side.last_used), side.started))
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "That teammate already has {limit} threads working. Let one finish, then try again."
                    )
                })?;
            inner.live.remove(&idlest.id);
            Some(idlest)
        } else {
            None
        };
        *inner.starting.entry(reservation).or_default() += 1;
        Ok(freed)
    }

    /// Whether a thread could be opened for this teammate now: a place is free,
    /// or one is held by a thread that is not mid-turn and can be parked. A
    /// handoff waits in its queue until this is true, rather than being
    /// refused.
    pub(super) fn has_room(&self, persona_id: &str) -> bool {
        let inner = lock(&self.inner);
        let live: Vec<&Arc<LiveSide>> = inner
            .live
            .values()
            .filter(|side| side.persona_id == persona_id && side.thread.kind == ThreadKind::Side)
            .collect();
        let starting = inner
            .starting
            .get(&format!("side:{persona_id}"))
            .copied()
            .unwrap_or(0);
        live.len() + starting < MAX_LIVE || live.iter().any(|side| !side.working())
    }

    fn release(&self, persona_id: &str) {
        let mut inner = lock(&self.inner);
        if let Some(count) = inner.starting.get_mut(persona_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                inner.starting.remove(persona_id);
            }
        }
    }

    pub(super) fn get(&self, side_id: &str) -> Option<Arc<LiveSide>> {
        lock(&self.inner).live.get(side_id).cloned()
    }

    fn remove(&self, side_id: &str) {
        lock(&self.inner).live.remove(side_id);
    }

    fn of(&self, persona_id: &str) -> Vec<Arc<LiveSide>> {
        let mut sides: Vec<Arc<LiveSide>> = lock(&self.inner)
            .live
            .values()
            .filter(|side| side.persona_id == persona_id)
            .cloned()
            .collect();
        sides.sort_by_key(|side| side.started);
        sides
    }

    pub(super) fn all(&self) -> Vec<Arc<LiveSide>> {
        lock(&self.inner).live.values().cloned().collect()
    }
}

/// Gives back the place a start reserved, however the start ends.
struct Reserved<'a> {
    sides: &'a Sides,
    persona_id: &'a str,
    kind: ThreadKind,
}

impl Drop for Reserved<'_> {
    fn drop(&mut self) {
        self.sides
            .release(&format!("{}:{}", self.kind.name(), self.persona_id));
    }
}

/// A task as a label: its first line, flattened and cut.
pub(super) fn title_of(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    cut(line, TITLE_CHARS)
}

pub(super) fn cut(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let kept: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

/// What a thread is brought up from: a task that has never run, or a thread
/// that ran before and is being reopened. What it said before, and the session
/// it saved, are on its own stream and marker, and the agent's builder reads
/// them there.
pub(super) struct Start {
    pub thread: ThreadId,
    pub title: String,
    pub started: i64,
    /// The teammate that handed this work over, when one did.
    pub opener: Option<Opener>,
    /// Whether the thread has never run. A start that fails closes a new
    /// thread, which has nothing to come back to, and parks one that ran
    /// before, so that saying something in it tries again.
    pub fresh: bool,
    /// The session the thread's marker holds, for a thread being brought
    /// back. It stays on the marker while the agent starts, and is dropped if
    /// the harness will not reopen it.
    pub saved: Option<AgentBinding>,
    /// The exchange a handoff opened this thread for, whose result is a
    /// failure if the thread cannot start.
    pub handoff: Option<HandoffStart>,
}

/// The exchange request a work thread was opened to answer.
pub(super) struct HandoffStart {
    pub key: String,
    pub request: String,
}

/// A published thread whose agent has not been built yet. [`Room::launch`]
/// builds it, once whoever published the thread has put in it the lines that
/// are to be heard first.
#[must_use = "the thread's agent is not started until it is launched"]
pub(super) struct Launch {
    side: Arc<LiveSide>,
    persona: Persona,
    /// When the thread was published: what is said in it from then on is for
    /// the agent to hear, not to be seeded with.
    requested: i64,
    fresh: bool,
    handoff: Option<HandoffStart>,
}

/// How a live thread comes to an end.
pub(super) enum Ending {
    /// The agent is let go of and the thread stays open.
    Park,
    /// The thread is over, `by` whom or what, with `outcome` in a line if it
    /// was said.
    Close(End, Option<String>),
}

impl Room {
    /// Calls and work threads share the same occupant and restoration path.
    pub(super) fn work_thread(&self, id: &str) -> ThreadId {
        self.sides.get(id).map_or_else(
            || {
                let call = ThreadId::call(id);
                if self.link_of(&call).is_some() {
                    call
                } else {
                    ThreadId::side(id)
                }
            },
            |live| live.thread.clone(),
        )
    }

    /// Starts a side thread with this teammate about `text`, and answers its
    /// summary at once. The thread is published, with the task as its first
    /// line, before its agent exists: the agent is brought up on its own task,
    /// which takes seconds, and the first turn begins the moment it is ready.
    /// Without `text` the thread opens untitled and waits for the person's
    /// first line, which names it.
    pub async fn start_side(
        self: &Arc<Self>,
        persona_id: &str,
        text: &str,
    ) -> Result<SideThreadSummary, String> {
        let _working = self.working()?;
        let text = text.trim();
        if text.len() > super::TEAMMATE_MESSAGE_MAX {
            return Err("That task is too long for a side thread.".to_string());
        }
        let (live, launch) = self.bring_up(
            persona_id,
            Start {
                thread: ThreadId::side(new_id()),
                title: title_of(text),
                started: now_ms(),
                opener: None,
                fresh: true,
                saved: None,
                handoff: None,
            },
        )?;
        if !text.is_empty() {
            self.say_in_side(&live, text, None, None);
        }
        self.launch(launch);
        Ok(self.side_summary(&live))
    }

    /// Brings a parked or archived thread back, and answers its summary. A
    /// thread that is live already is answered as it is.
    pub async fn continue_side(
        self: &Arc<Self>,
        side_id: &str,
    ) -> Result<SideThreadSummary, String> {
        let _working = self.working()?;
        let (side, launch) = self.wake_side(side_id, true, None).await?;
        if let Some(launch) = launch {
            self.launch(launch);
        }
        Ok(self.side_summary(&side))
    }

    /// A thread a teammate handed over ends with its result, not with the
    /// person's Archive.
    fn refuse_archiving_handed_over(&self, side_id: &str) -> Result<(), String> {
        match self.opener_of(side_id) {
            Some(opener) => Err(format!(
                "{name} handed this thread over, so it ends with its result. Stop its turn, or talk to {name} about it.",
                name = opener.name
            )),
            None => Ok(()),
        }
    }

    /// The teammate that handed this thread over, when one did: as the live
    /// thread holds it, else as its link records it.
    pub(super) fn opener_of(&self, side_id: &str) -> Option<Opener> {
        match self.sides.get(side_id) {
            Some(side) => side.opener.clone(),
            None => self
                .link_of(&self.work_thread(side_id))
                .and_then(|link| link.opener),
        }
    }

    /// A thread's place, published: its link and its marker say live, and it
    /// can be said to, before any agent exists for it.
    ///
    /// Building the agent is the slow part (the computer, the skills, reading
    /// the conversation it was started beside, and then the harness's own
    /// start-up), so it is not waited for here: [`Room::launch`] does it on
    /// its own task. The thread holds the claim on its queue until then, so a
    /// line said meanwhile waits behind the start, and the first turn runs
    /// when the agent is up.
    ///
    /// A new thread has a fresh context. One that ran before reopens its own
    /// saved session when the harness can, and when it cannot (or never saved
    /// one) is given the thread's own stream: see [`Room::thread_agent`]. The
    /// main conversation's session is never touched, so it can never land
    /// there.
    pub(super) fn bring_up(
        self: &Arc<Self>,
        persona_id: &str,
        start: Start,
    ) -> Result<(Arc<LiveSide>, Launch), String> {
        let persona = self.persona(persona_id)?;
        let persona = if start.thread.kind == ThreadKind::Call {
            super::agent::clone_of(persona, &self.info(persona_id), &[])
        } else {
            persona
        };
        let requested = now_ms();
        let persona_lease = self.capability_lease(persona_id);
        persona_lease.check()?;
        if let Some(parked) = self.sides.reserve(persona_id, start.thread.kind)? {
            self.park_side(&parked);
        }
        let _reserved = Reserved {
            sides: &self.sides,
            persona_id,
            kind: start.thread.kind,
        };

        // The thread is its own authority, not the session's: whatever the
        // teammate is granted now is what it gets, however long ago it began.
        let lease = lease_of(start.thread.kind, &persona_lease);
        // What the marker held stays on it while the agent starts, so that
        // the start can still reopen it, but only if it is one this harness
        // issued.
        let kept = start
            .saved
            .filter(|saved| {
                persona.backend_id != crate::driver::HOTLINE_BACKEND_ID
                    && saved.backend_id == persona.backend_id
            })
            .map(|saved| saved.session_id);
        let live = Arc::new(LiveSide {
            roster: Mutex::default(),
            id: start.thread.key.clone(),
            thread: start.thread,
            persona_id: persona_id.to_string(),
            title: Mutex::new(start.title),
            started: start.started,
            opener: start.opener,
            request: Mutex::new(None),
            backend_id: persona.backend_id.clone(),
            up: OnceLock::new(),
            capability: lease,
            // Claimed for the start: a line said now is queued, not run.
            turns: Mutex::new(Turns {
                waiting: VecDeque::default(),
                running: true,
            }),
            last_used: Mutex::new(now_ms()),
            dispatched: Mutex::new(HashSet::default()),
            closed: AtomicBool::new(false),
            stopped_by_person: AtomicBool::new(false),
            archive_note: Mutex::new(None),
            reported: Mutex::new(None),
            saved: Mutex::new(kept),
        });
        {
            // Publication and revocation share this lock, so a stop or policy
            // change that lands during the start either refuses it here or
            // finds the thread and archives it.
            let _lifecycle = lock(&self.lifecycle);
            persona_lease.check()?;
            let mut inner = lock(&self.sides.inner);
            inner.live.insert(live.id.clone(), live.clone());
        }

        self.mark_side(&live, ThreadState::Live, None);
        let _ = self.info_changes.send(self.info(persona_id));
        let launch = Launch {
            side: live.clone(),
            persona,
            requested,
            fresh: start.fresh,
            handoff: start.handoff,
        };
        Ok((live, launch))
    }

    /// Builds the agent of a published thread on a task of its own, and runs
    /// the lines that waited for it. A start that fails says so in the thread
    /// and puts it away: see [`Room::start_failed`].
    pub(super) fn launch(self: &Arc<Self>, launch: Launch) {
        let room = self.clone();
        let Ok(working) = self.lease() else {
            // The desk is stopping for an update: the thread keeps its place
            // as a parked one, and saying something in it starts it after.
            room.end_side(&launch.side, Ending::Park);
            return;
        };
        tokio::spawn(async move {
            let _working = working;
            let side = launch.side.clone();
            match room
                .start_agent(&side, launch.persona.clone(), launch.requested)
                .await
            {
                Ok(()) => {
                    let first = lock(&side.turns).next_line();
                    match first {
                        Some(line) => room.clone().run_queue(side, line).await,
                        None => {
                            let _ = room.info_changes.send(room.info(&side.persona_id));
                        }
                    }
                }
                Err(error) => room.start_failed(&launch, &error),
            }
        });
    }

    /// The slow half of a start: the agent is built and started for a
    /// published thread, and handed to it. Nothing here is waited for by
    /// whoever opened the thread.
    async fn start_agent(
        self: &Arc<Self>,
        side: &Arc<LiveSide>,
        persona: Persona,
        heard_from: i64,
    ) -> Result<(), String> {
        let title = lock(&side.title).clone();
        let mut agent = self
            .thread_agent(Opening {
                thread: side.thread.clone(),
                persona,
                title,
                opener: side.opener.clone(),
                lease: side.capability.clone(),
                heard_from: Some(heard_from),
            })
            .await?;
        let up = Up {
            driver: agent.driver.clone(),
            holder: agent.holder.clone(),
        };
        let _ = side.up.set(up);
        // The thread may have been ended while its agent was starting. The
        // agent is set before this looks, and `end_side` marks the thread
        // closed before it looks for the agent, so one of the two stops it.
        if side.closed.load(Ordering::SeqCst) {
            self.let_go_of_computer(&side.persona_id, &agent.holder);
            return Err("This thread ended before its agent was ready.".to_string());
        }
        self.watch_side_activity(side, &mut agent);
        agent.keep();
        lock(&side.reported).clone_from(&agent.reported);
        let changed = {
            let mut saved = lock(&side.saved);
            let changed = *saved != agent.resumed;
            saved.clone_from(&agent.resumed);
            changed
        };
        if changed {
            self.mark_side(side, ThreadState::Live, None);
        }
        Ok(())
    }

    /// An agent that could not be started: the thread says so, and is put
    /// away rather than left waiting for one. A thread that had never run is
    /// closed as failed; one that had is parked, since its stream is whole and
    /// the next line said in it tries again. A handoff's sender is told too,
    /// as a handoff that could not start a thread would be.
    fn start_failed(&self, launch: &Launch, error: &str) {
        let side = &launch.side;
        if side.closed.load(Ordering::SeqCst) {
            // Ended on purpose while it started, and already put away.
            return;
        }
        eprintln!("{} could not start a thread: {error}", launch.persona.name);
        let reason = format!(
            "{} could not start this thread: {error}",
            launch.persona.name
        );
        side.say(
            self,
            &TranscriptEvent::Notice {
                id: new_id(),
                ts: now_ms(),
                level: NoticeLevel::Error,
                text: reason.clone(),
            },
        );
        if launch.fresh {
            self.end_side(side, Ending::Close(End::Failed, Some(reason)));
        } else {
            self.end_side(side, Ending::Park);
        }
        if let Some(handoff) = &launch.handoff {
            let _ = self.set_exchange_reply(
                &handoff.key,
                &handoff.request,
                format!(
                    "{} could not start a thread for this: {error}",
                    launch.persona.name
                ),
                true,
            );
        }
    }

    /// Brings a thread whose agent is gone back from its marker. A parked
    /// thread is the one a line said to it wakes; an archived one only comes
    /// back when the person asks to continue it.
    ///
    /// The thread is answered as soon as it is published, still starting, with
    /// the [`Launch`] to run once the caller has said what it came back for.
    /// A thread that was live already has none.
    ///
    /// `handoff` is the exchange a colleague's handoff brings it back for,
    /// whose sender is told if the thread cannot start.
    pub(super) async fn wake_side(
        self: &Arc<Self>,
        side_id: &str,
        continuing: bool,
        handoff: Option<HandoffStart>,
    ) -> Result<(Arc<LiveSide>, Option<Launch>), String> {
        let _one_at_a_time = self.sides.waking.lock().await;
        self.wake_work(side_id, continuing, handoff)
    }

    fn wake_work(
        self: &Arc<Self>,
        side_id: &str,
        continuing: bool,
        handoff: Option<HandoffStart>,
    ) -> Result<(Arc<LiveSide>, Option<Launch>), String> {
        if let Ok(side) = self.live_side(side_id) {
            return Ok((side, None));
        }
        let link = self
            .link_of(&self.work_thread(side_id))
            .ok_or_else(|| "There is no such side thread.".to_string())?;
        match link.state {
            ThreadState::Parked => {}
            ThreadState::Closed(_) if continuing => {}
            ThreadState::Closed(_) => {
                return Err(
                    "That side thread is archived. Continue it to talk in it again.".to_string(),
                );
            }
            ThreadState::Live if link.thread.kind == ThreadKind::Call => {}
            ThreadState::Live => return Err("That side thread is not live.".to_string()),
        }
        let Some(persona_id) = link.persona_id else {
            return Err("That side thread could not be read.".to_string());
        };
        let (side, launch) = self.bring_up(
            &persona_id,
            Start {
                thread: link.thread,
                title: link.title,
                started: link.ts,
                opener: link.opener,
                fresh: false,
                saved: link.binding,
                handoff,
            },
        )?;
        Ok((side, Some(launch)))
    }

    /// Archives a side thread, live or parked. Archiving one that is already
    /// archived is answered as done, so a second press is not an error. The
    /// person's Archive is refused on a thread a colleague handed over,
    /// whatever its state: it ends with its result.
    pub fn archive_side(
        &self,
        side_id: &str,
        by: SideEnd,
        result: Option<String>,
    ) -> Result<(), String> {
        if by == SideEnd::Person {
            self.refuse_archiving_handed_over(side_id)?;
        }
        if let Some(side) = self.sides.get(side_id) {
            self.finish_side(&side, by.into(), result);
            return Ok(());
        }
        let Some(link) = self.link_of(&self.work_thread(side_id)) else {
            return Err("There is no such side thread.".to_string());
        };
        match link.state {
            ThreadState::Closed(_) => Ok(()),
            ThreadState::Parked => {
                self.archive_parked(link, by.into(), result);
                Ok(())
            }
            ThreadState::Live => Err("There is no such side thread.".to_string()),
        }
    }

    /// Lets go of a side thread's agent and keeps the thread open, as the
    /// sweep does for one nobody has spoken in: saying something in it brings
    /// an agent back. Parking one that is parked already is answered as done,
    /// and an archived one is refused, since it is not open.
    pub fn park_side_thread(&self, side_id: &str) -> Result<(), String> {
        if let Some(side) = self.sides.get(side_id) {
            self.park_side(&side);
            return Ok(());
        }
        match self
            .link_of(&self.work_thread(side_id))
            .map(|link| link.state)
        {
            Some(ThreadState::Parked) => Ok(()),
            Some(ThreadState::Closed(_)) => {
                Err("That side thread is archived, so it cannot be parked.".to_string())
            }
            Some(ThreadState::Live) | None => Err("There is no such side thread.".to_string()),
        }
    }

    /// The teammate saying it is done. The thread is archived when the turn
    /// it said so in ends, so its last message lands first.
    pub(crate) fn request_side_archive(&self, side_id: &str, summary: &str) -> Result<(), String> {
        let side = self.live_side(side_id)?;
        *lock(&side.archive_note) = Some(cut(summary, RESULT_CHARS));
        Ok(())
    }

    /// Answers a permission card raised inside a side thread.
    pub async fn answer_side_permission(
        &self,
        side_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), String> {
        let side = self.live_side(side_id)?;
        if !side
            .driver()
            .is_some_and(|driver| driver.answer_permission(request_id, option_id))
        {
            return Err("That request is no longer waiting for an answer.".to_string());
        }
        let id = Value::from(format!("perm:{request_id}"));
        let card = self
            .log
            .load(&StreamId::of_thread(&self.work_thread(side_id)).expect("work has a stream"))
            .into_iter()
            .find(|event| event.get("id") == Some(&id))
            .and_then(|event| serde_json::from_value::<TranscriptEvent>(event).ok());
        let Some(TranscriptEvent::Permission {
            id,
            request_id,
            title,
            options,
            ..
        }) = card
        else {
            return Ok(());
        };
        let decided_option_name = options
            .iter()
            .find(|option: &&PermissionOption| option.option_id == option_id)
            .map(|option| option.name.clone());
        side.say(
            self,
            &TranscriptEvent::Permission {
                from: None,
                id,
                ts: now_ms(),
                request_id,
                title,
                options,
                decision: Some(option_id.to_string()),
                decided_option_name,
            },
        );
        Ok(())
    }

    /// The teammate's reaction to what the person last said in this thread.
    pub(crate) fn react_side(&self, side_id: &str, emoji: &str) -> Result<(), String> {
        let emoji = emoji.trim();
        if emoji.is_empty() || emoji.chars().count() > 4 || emoji.chars().any(char::is_alphanumeric)
        {
            return Err("react needs one emoji.".to_string());
        }
        let side = self.live_side(side_id)?;
        let stream = self
            .log
            .load(&StreamId::of_thread(&self.work_thread(side_id)).expect("work has a stream"));
        let line = stream
            .iter()
            .rev()
            .find(|event| event["kind"] == "user")
            .ok_or_else(|| "There is no message from the person to react to.".to_string())?;
        let TranscriptEvent::User {
            id,
            ts,
            text,
            attachments,
            reactions,
            reply_to,
            scheduled,
            ring,
            receipt,
            client,
        } = serde_json::from_value::<TranscriptEvent>(line.clone())
            .map_err(|error| format!("The last message could not be read: {error}"))?
        else {
            return Err("There is no message from the person to react to.".to_string());
        };
        let mut reactions = reactions.unwrap_or_default();
        if reactions.iter().any(|had| had == emoji) {
            return Ok(());
        }
        reactions.push(emoji.to_string());
        side.say(
            self,
            &TranscriptEvent::User {
                id,
                ts,
                text,
                attachments,
                reactions: Some(reactions),
                reply_to,
                scheduled,
                ring,
                receipt,
                client,
            },
        );
        Ok(())
    }

    /// Opens a link in the person's browser, only while they are at the
    /// desktop app: their last line in this thread came from it.
    pub(crate) async fn open_link_side(&self, side_id: &str, link: &str) -> Result<String, String> {
        let link = url::Url::parse(link.trim())
            .ok()
            .filter(|link| matches!(link.scheme(), "http" | "https"))
            .ok_or("open_link needs a full http or https link.")?;
        self.live_side(side_id)?;
        let here = self
            .log
            .load(&StreamId::of_thread(&self.work_thread(side_id)).expect("work has a stream"))
            .iter()
            .rev()
            .find(|event| event["kind"] == "user")
            .is_some_and(|event| event.get("client").and_then(Value::as_str) == Some("desktop"));
        if !here {
            return Err("The person is not at this computer, so nothing was opened. Put the link in your reply instead.".into());
        }
        super::open_in_browser(link.as_str()).await?;
        Ok("Opened in the person's browser on this computer.".into())
    }

    /// Whether this work thread has a turn in flight, without listing its history.
    pub(crate) fn work_thread_working(&self, thread: &ThreadId) -> bool {
        self.sides
            .get(&thread.key)
            .is_some_and(|live| live.thread == *thread && live.working())
    }

    /// Call threads with a turn in flight, separate from their saved lifetime.
    pub fn working_calls(&self, persona_id: &str) -> Vec<String> {
        self.sides
            .of(persona_id)
            .iter()
            .filter(|side| side.thread.kind == ThreadKind::Call && side.working())
            .map(|side| side.id.clone())
            .collect()
    }

    pub fn sides(&self, persona_id: &str) -> Vec<RunningSide> {
        self.sides
            .of(persona_id)
            .iter()
            .filter(|side| side.thread.kind == ThreadKind::Side)
            .map(|side| RunningSide {
                side_id: side.id.clone(),
                title: lock(&side.title).clone(),
                started_at: side.started,
                working: side.working(),
            })
            .collect()
    }

    /// Every side thread this teammate has had: the live ones, oldest first,
    /// then the parked, then the archived, each newest first.
    pub fn side_threads(&self, persona_id: &str) -> Vec<SideThreadSummary> {
        let mut live: Vec<SideThreadSummary> = self
            .sides
            .of(persona_id)
            .iter()
            .filter(|side| side.thread.kind == ThreadKind::Side)
            .map(|side| self.side_summary(side))
            .collect();
        let mut kept: Vec<SideThreadSummary> = self
            .stored_side_ids()
            .into_iter()
            .filter(|id| self.sides.get(id).is_none())
            .filter_map(|id| self.stored_summary(&id, persona_id))
            .collect();
        kept.sort_by_key(|summary| {
            (
                summary.status == SideStatus::Archived,
                std::cmp::Reverse(summary.archived_at.unwrap_or(summary.last_at)),
            )
        });
        live.extend(kept);
        live
    }

    /// Whether a live side thread of this teammate is stopped on a card, for
    /// the roster row.
    pub(super) fn side_cards_waiting(&self, persona_id: &str) -> bool {
        self.sides
            .of(persona_id)
            .iter()
            .any(|side| super::threads::cards_waiting(&self.log, &side.thread, &side.roster))
    }

    /// Archives every thread this teammate has: its authority is gone, so
    /// their agents are. The person can continue them under what it has next.
    pub(super) fn drop_sides(&self, persona_id: &str) {
        for side in self.sides.of(persona_id) {
            self.finish_side(&side, End::Stopped, None);
        }
    }

    pub(super) fn drop_all_sides(&self) {
        for side in self.sides.all() {
            self.finish_side(&side, End::Stopped, None);
        }
    }

    /// What the room does with a thread's turn when it is stopping for a
    /// restart and the drain ran out: stop it, and say so in the thread.
    pub(super) fn interrupt_sides(&self) {
        for side in self.sides.all() {
            if !side.working() {
                continue;
            }
            lock(&side.turns).clear();
            if let Some(driver) = side.driver() {
                driver.cancel();
            }
            side.say(self, &TranscriptEvent::Notice {
                    id: new_id(),
                    ts: now_ms(),
                    level: NoticeLevel::Warn,
                    text: "Hotline restarted while this turn was running, so it was stopped. It was not run again: send it again if it still matters.".to_string(),
                },
            );
        }
    }

    fn live_side(&self, side_id: &str) -> Result<Arc<LiveSide>, String> {
        let side = self
            .sides
            .get(side_id)
            .filter(|side| !side.closed.load(Ordering::SeqCst))
            .ok_or_else(|| "That side thread is not live.".to_string())?;
        side.capability.check()?;
        Ok(side)
    }

    /// The person's line, written to the thread and handed to its agent.
    fn say_in_side(
        self: &Arc<Self>,
        side: &Arc<LiveSide>,
        text: &str,
        reply_to: Option<String>,
        attachments: Option<Vec<Attachment>>,
    ) {
        let ts = now_ms();
        let client = crate::wire::commands::prompt_client();
        *lock(&side.last_used) = ts;
        self.name_side(side, text);
        side.say(
            self,
            &TranscriptEvent::User {
                id: new_id(),
                ts,
                text: text.to_string(),
                attachments: attachments.clone(),
                reactions: None,
                reply_to,
                scheduled: None,
                ring: None,
                receipt: None,
                client,
            },
        );
        let voice = self.attached_voice(&side.thread);
        let interruption = voice
            .as_ref()
            .and_then(|origin| {
                lock(&self.voice)
                    .upgrade()?
                    .take_interruption(&origin.call_id)
            })
            .unwrap_or_default();
        self.queue_in_side(
            side,
            Line {
                voice: voice.clone(),
                text: if voice.is_some() {
                    format!(
                        "{}\n\n{}\n\n{}",
                        timed_from(ts, client, text),
                        interruption,
                        self.call_instruction(&side.thread)
                    )
                } else {
                    timed_from(ts, client, text)
                },
                attachments: attachments.unwrap_or_default(),
                request: None,
                from: None,
                delivery: None,
                wake: false,
            },
        );
    }

    /// Hands a line to the thread's agent now, or behind the turn in flight.
    pub(super) fn queue_in_side(self: &Arc<Self>, side: &Arc<LiveSide>, line: Line) {
        let Some(queued) = lock(&side.turns).claim(line) else {
            return;
        };
        let Ok(working) = self.lease() else {
            lock(&side.turns).running = false;
            return;
        };
        let _ = self.info_changes.send(self.info(&side.persona_id));
        let room = self.clone();
        let side = side.clone();
        tokio::spawn(async move {
            let _working = working;
            room.run_queue(side, queued).await;
        });
    }

    /// Something that came back for this thread, in the thread: a colleague's
    /// answer, the person's answer to a card, the handoff that opened it. It is
    /// written to the thread's stream first, so it survives a restart and is
    /// heard once, and then handed to the agent behind the turn in flight.
    /// Delivering the same id twice delivers it once: one the thread's turns
    /// have begun on is not delivered again, but one that was saved and never
    /// reached a turn, because the desk stopped between the two, is handed to
    /// the agent that finds it, as the main conversation's is.
    ///
    /// A parked thread is brought back for it, and so is a closed one: the
    /// person answering a card in it is continuing it. `request` says the turn
    /// it starts answers an exchange request, so its result is saved when the
    /// turn ends.
    pub(super) async fn deliver_into_work(
        self: &Arc<Self>,
        side_id: &str,
        id: &str,
        cause: DeliveryCause,
        from: DeliveryFrom,
        text: String,
        request: Option<RequestLine>,
    ) -> Result<(), String> {
        let _working = self.working()?;
        let _admission = self.sides.waking.lock().await;
        let (side, launch) = match self.live_side(side_id) {
            Ok(side) => (side, None),
            Err(_) => self.wake_work(side_id, true, None)?,
        };
        // What came back is queued before the agent is started, so it is the
        // first thing the agent hears however fast the start goes.
        let delivered = (|| {
            let stream =
                StreamId::of_thread(&self.work_thread(side_id)).expect("work has a stream");
            let existing = self
                .log
                .load(&stream)
                .into_iter()
                .find(|event| event["id"] == id);
            if existing
                .as_ref()
                .is_some_and(|event| event["receipt"] == "read")
            {
                return Ok(());
            }
            if !lock(&side.dispatched).insert(id.to_string()) {
                return Ok(());
            }
            let ts = existing
                .as_ref()
                .and_then(|event| event["ts"].as_i64())
                .unwrap_or_else(now_ms);
            let wire = super::peers::delivery_wire(&cause, &text);
            *lock(&side.last_used) = now_ms();
            if existing.is_none() {
                side.say(
                    self,
                    &TranscriptEvent::Delivery {
                        id: id.to_string(),
                        ts,
                        from: Some(from.clone()),
                        cause,
                        text,
                        receipt: Some(Receipt::Sent),
                    },
                );
            }
            self.queue_in_side(
                &side,
                Line {
                    voice: None,
                    text: super::timed(ts, &wire),
                    attachments: Vec::new(),
                    request,
                    from: Some(from),
                    delivery: Some(id.to_string()),
                    wake: false,
                },
            );
            Ok(())
        })();
        if let Some(launch) = launch {
            self.launch(launch);
        }
        delivered
    }

    /// Whether this thread can be talked in without the person pressing
    /// Continue: it is live or parked.
    pub(super) fn work_is_open(&self, side_id: &str) -> bool {
        self.sides.get(side_id).is_some()
            || self
                .link_of(&self.work_thread(side_id))
                .is_some_and(|link| !matches!(link.state, ThreadState::Closed(_)))
    }
}

/// What the teammate last said in a thread, as a line.
fn last_words(events: &[Value]) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|event| event["kind"] == "agent")
        .and_then(|event| event.get("text").and_then(Value::as_str))
        .map(|text| cut(text, RESULT_CHARS))
        .filter(|text| !text.is_empty())
}

/// The newest thing said in a thread, as a line for a list: the teammate's
/// last words, else the person's latest.
pub(crate) fn preview_line(events: &[Value]) -> Option<String> {
    last_words(events).or_else(|| {
        events
            .iter()
            .rev()
            .filter(|event| event["kind"] == "user")
            .find_map(|event| event.get("text").and_then(Value::as_str))
            .map(|text| cut(text, RESULT_CHARS))
            .filter(|text| !text.is_empty())
    })
}

/// The one line a closing note says about how it came out.
pub(super) fn outcome_line(note: &str) -> Option<String> {
    note.lines()
        .find_map(|line| line.strip_prefix("Outcome:"))
        .map(|outcome| cut(outcome, RESULT_CHARS))
        .filter(|outcome| !outcome.is_empty())
}

fn last_at(events: &[Value]) -> i64 {
    events
        .iter()
        .filter_map(|event| event.get("ts").and_then(Value::as_i64))
        .max()
        .unwrap_or_default()
}

/// Whether a card in the thread is waiting on the person: a permission, or a
/// request the teammate parked on them and went on.
pub(super) fn waiting_on(events: &[Value]) -> bool {
    events
        .iter()
        .any(|event| match event.get("kind").and_then(Value::as_str) {
            Some("permission") => event.get("decision").is_none(),
            Some("human_action" | "credential_ask") => {
                event.get("status").and_then(Value::as_str) == Some("pending")
            }
            _ => false,
        })
}

mod activity;
mod brief;
mod ending;
mod input;
pub(super) use brief::work_preamble;

#[cfg(test)]
mod tests;
