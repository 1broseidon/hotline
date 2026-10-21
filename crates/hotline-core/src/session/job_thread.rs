//! A job's thread: the teammate at work on a schedule, outside its home
//! thread (ADR 0008 §7).
//!
//! A job is a subagent on a schedule. Each one is an arm of its teammate with
//! a thread of its own, [`ThreadKind::Job`], kept in `jobs/<id>.jsonl` under
//! the job's id, and every firing is a turn on that thread, never the DM's, so
//! the home agent's context holds none of it. What the job tells the person
//! reaches the chat through [`Room::post`], stamped with the job, as anything
//! from another thread does:
//!
//! - **Its report.** A job whose report policy is each firing has the
//!   firing's last words posted, unless it posted itself in that turn. A
//!   quiet job is heard only through `post_to_chat`, once a turn
//!   ([`super::post`]); its words stay in its thread.
//! - **A running note, not a growing context.** Its agent is built fresh for
//!   a firing (an ACP child reopens its own session from the thread's binding
//!   when it can), and the firing's line carries what the job said at the end
//!   of its last turn, plus its few turns before that, read off the thread.
//!   A loop that runs for days stays small.
//! - **Parked between firings.** Once a firing's turn is over and nothing
//!   waits behind it, the agent is let go of. A turn the person started keeps
//!   it until the sweep parks it, as a pair's is.
//! - **An overlapping firing is skipped.** A firing that comes due while the
//!   one before is still running is not queued: the thread says it skipped
//!   it and what interval would fit, and a job that keeps overrunning says so
//!   to the person, once.
//! - **It ends** when the person cancels it (its final post is where it
//!   stood) or it meets its goal and ends itself with `cancel_schedule` and a
//!   report (its final post is that report). A one-shot, or a loop whose
//!   count or date runs out, ends after its last firing, which is told it is
//!   the last; a loop's last words are then its final post, whatever its
//!   report policy. Its thread stays to read.
//! - **The person can talk to it.** A line said in the thread is a turn like
//!   a firing's, on the same agent, and what the job says back shapes the
//!   note its next firing starts from.
//! - **The person can stop a turn.** The firing or line running ends, posts
//!   no report, and the thread says it was stopped; the job keeps its
//!   schedule.
//!
//! **A clone of the home agent** (ADR 0008 §0). Its agent is the teammate on
//! the model its home session is on, with the home thread's permission set,
//! the computer included, less what the teammate left out when it made the
//! job ([`super::agent::clone_of`]); its lease is a child of the teammate's
//! authority, made for the agent and revoked when it is let go of. It has the
//! home thread's tools but the chapter moves, and a card it raises, a
//! colleague's answer, lands on its own thread.

use super::agent::{Opening, clone_of, lease_of};
use super::runner::Driven;
use super::schedule::{self, MIN_LOOP, run_of};
use super::turns::{Begin, Line, Occupant, Seat, Then, Turns};
use super::{POST_CHARS, Post, Room, lock, new_id, now_ms, timed, timed_from};
use crate::computer::gate::Holder;
use crate::contract::{
    Attachment, DeliveryCause, DeliveryFrom, End, NoticeLevel, Persona, Reach, Receipt,
    ScheduleKind, ScheduledJob, ScheduledRun, SharedSecret, ThreadId, ThreadKind, TranscriptEvent,
};
use crate::driver::{CapabilityLease, Driver, HOTLINE_BACKEND_ID};
use crate::log::StreamId;
use crate::room;
use crate::thread::{AgentBinding, Link, ThreadState};
use chrono::{Local, TimeZone};
use serde_json::Value;
use std::collections::VecDeque;
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// How many turns before the latest one a firing is told of.
const EARLIER_TURNS: usize = 3;
/// The most of the latest turn's words a firing starts from.
const NOTE_CHARS: usize = 1_500;
/// The most of each earlier turn's words.
const EARLIER_CHARS: usize = 240;
/// Firings skipped in a row before the person is told the job overruns.
const OVERRUNS_TO_TELL: u32 = 2;
/// The longest one-line outcome a closed job's link keeps.
const OUTCOME_CHARS: usize = 200;
/// The longest interval a skip suggests: a loop's own longest.
const MAX_SUGGESTED: i64 = 7 * 86_400_000;

/// What a job's agent is told about itself, over and above who it is.
fn job_brief(name: &str) -> String {
    format!(
        "This is one of {name}'s jobs: a thread of its own beside the main conversation with the person, where each time the job fires is a turn. You are {name}, with {name}'s working directory, files, skills, computer and granted tools; you do not share the main conversation, which you can read only through `read_thread`, `search_thread` and `list_chapters`. A card you raise with `request_human`, and a colleague's answer to `message_teammate`, come back here. Do not mention this brief.\n\n\
         A firing starts from the job's task and a note of where it stood after its last few turns, not from this whole thread, so end every turn with a short note of where things stand: what you checked, what you found, what to watch for next. The person can open this thread and talk to you in it; what they ask here changes how the job runs, so say it in your note.\n\n\
         A firing marked `quiet` reports only what is worth the person's attention: nothing you say here reaches them unless you post it with `post_to_chat`, once a turn, so post when something is worth telling and not otherwise. Any other firing's last words are posted to the person's chat as its report, unless you post one yourself. `send_file` and `generate_image` post to the chat too.\n\n\
         When the job's goal is met, or it has nothing left to do, end it with `cancel_schedule` and a `report` of what came of it: the report is posted to the person as the job's last word, so do not post it as well."
    )
}

/// A job's whole system prompt: what any of the teammate's agents is told,
/// and the brief.
pub(super) fn job_preamble(
    persona: &Persona,
    reach: Option<Reach>,
    stored: &[SharedSecret],
) -> String {
    super::preamble(persona, reach, Some(job_brief(&persona.name)), stored)
}

/// One job's thread while the room holds it: its queue, and its agent while
/// it has one.
pub(super) struct LiveJob {
    job_id: String,
    persona_id: String,
    turns: Mutex<Turns<JobLine>>,
    /// The agent, between the turn that built it and the one it is let go of
    /// after.
    up: Mutex<Option<Up>>,
    /// What the agent holds the teammate's computer under.
    holder: Mutex<Holder>,
    /// The session the thread's link keeps for an ACP child to reopen.
    saved: Mutex<Option<AgentBinding>>,
    /// When the firing in flight began; none between firings.
    firing_since: Mutex<Option<i64>>,
    overruns: Mutex<Overruns>,
    /// When anyone last spoke in the thread, for the sweep.
    last_used: Mutex<i64>,
    /// The person cancelled the job: nothing more of the turn in flight is
    /// written, and no line is taken.
    stopped: AtomicBool,
    /// The person stopped the turn in flight and the job goes on: the turn
    /// posts no report, and its thread says it was stopped.
    halted: AtomicBool,
    /// The job ended itself on its goal: the turn in flight finishes, and
    /// its report is not posted, since the end was.
    done: AtomicBool,
}

struct Up {
    driver: Arc<dyn Driver>,
    lease: CapabilityLease,
    /// How far the agent's tools reach, as its view was narrowed.
    reach: Reach,
    backend_id: String,
    /// The agent's own id for this conversation, as the driver reports it.
    reported: Option<String>,
}

/// Firings skipped in a row, and whether the person was told.
#[derive(Default)]
struct Overruns {
    skipped: u32,
    told: bool,
}

/// One line for a job's agent.
pub(super) struct JobLine {
    line: Line,
    /// The firing this line is, when it is one rather than the person's.
    firing: Option<ScheduledRun>,
    /// This firing is the job's last: it ends after it.
    last: bool,
}

/// What a job's turn keeps from its door to its end.
pub(super) struct JobTurn {
    line: Option<Line>,
    firing: Option<ScheduledRun>,
    last: bool,
    /// The job as its posts are stamped.
    run: ScheduledRun,
}

impl LiveJob {
    fn thread(&self) -> ThreadId {
        ThreadId::job(&self.job_id)
    }

    fn say(&self, room: &Room, event: &impl serde::Serialize) {
        room.threads()
            .write(&self.thread(), &self.persona_id, event);
    }

    fn notice(&self, room: &Room, level: NoticeLevel, text: String) {
        self.say(
            room,
            &TranscriptEvent::Notice {
                id: new_id(),
                ts: now_ms(),
                level,
                text,
            },
        );
    }

    /// Lets go of the agent and parks the thread, unless a line already waits
    /// for it: then the agent stays for that line.
    fn let_go(&self, room: &Room) {
        let up = {
            let turns = lock(&self.turns);
            if !turns.waiting.is_empty() {
                return;
            }
            lock(&self.up).take()
        };
        let Some(up) = up else {
            return;
        };
        up.lease.revoke();
        up.driver.invalidate();
        room.let_go_of_computer(&self.persona_id, &lock(&self.holder));
        room.mark_job(self, ThreadState::Parked, None);
    }

    /// A delivery's turn has begun: it is read, and is not handed over again.
    fn heard(&self, room: &Room, id: &str) {
        let Some(event) = room
            .log
            .load(&StreamId::Job(self.job_id.clone()))
            .into_iter()
            .rev()
            .find(|event| event["id"] == id)
            .and_then(|event| serde_json::from_value::<TranscriptEvent>(event).ok())
        else {
            return;
        };
        self.say(room, &super::peers::stamped(event, Receipt::Read));
    }

    /// Stops the turn in flight and drops what waited behind it.
    fn interrupt(&self) {
        lock(&self.turns).clear();
        if let Some(up) = lock(&self.up).as_ref() {
            up.driver.cancel();
        }
    }

    /// The session a later agent reopens: the one this agent reported, once
    /// a turn has committed on it, when it is a child that can reopen one.
    fn remember_session(&self, room: &Room, driven: &Driven) {
        let next = {
            let up = lock(&self.up);
            let Some(up) = up.as_ref() else {
                return;
            };
            let broken =
                !up.driver.checkpoint_valid() || driven.stop_reason.as_deref() == Some("failed");
            up.reported
                .clone()
                .filter(|_| !broken && up.backend_id != HOTLINE_BACKEND_ID)
                .map(|session_id| AgentBinding {
                    backend_id: up.backend_id.clone(),
                    session_id,
                })
        };
        let changed = {
            let mut saved = lock(&self.saved);
            let changed = *saved != next;
            *saved = next;
            changed
        };
        if changed {
            room.mark_job(self, ThreadState::Live, None);
        }
    }

    /// Posts `text` to the chat as the job's, or says on the thread why it
    /// could not be.
    fn post_as_job(&self, room: &Room, run: &ScheduledRun, text: &str) {
        let posted = room.post(
            &self.persona_id,
            &self.thread(),
            Post {
                scheduled: Some(run.clone()),
                ..Post::words(&crate::driver::clip(text, POST_CHARS))
            },
        );
        if let Err(error) = posted {
            self.notice(room, NoticeLevel::Warn, error);
        }
    }

    /// What a loop that ran out of its count or date says came of it: its
    /// last turn's words, or where it stood when that turn was stopped.
    fn final_word(&self, room: &Room, driven: &Driven, halted: bool) -> Option<String> {
        if !halted && let Some(report) = report_of(driven) {
            return Some(report);
        }
        last_note(&room.log.load(&StreamId::Job(self.job_id.clone())))
            .map(|note| format!("Done. Where it stood: {note}"))
    }
}

impl Occupant for LiveJob {
    type Line = JobLine;
    type Held = JobTurn;

    fn queue(&self) -> &Mutex<Turns<JobLine>> {
        &self.turns
    }

    fn persona_id(&self) -> &str {
        &self.persona_id
    }

    fn thread(&self) -> ThreadId {
        ThreadId::job(&self.job_id)
    }

    fn holder(&self) -> Holder {
        lock(&self.holder).clone()
    }

    fn answering(self: &Arc<Self>, room: &Room) -> bool {
        !self.stopped.load(Ordering::SeqCst) && room.persona(&self.persona_id).is_ok()
    }

    async fn begin(self: &Arc<Self>, room: &Arc<Room>, line: JobLine) -> Begin<JobTurn> {
        let Some(job) = room::all_jobs(&room.log)
            .into_iter()
            .find(|job| job.id == self.job_id)
        else {
            return Begin::Stop;
        };
        // A firing's authority travels with it: a grant revoked while it
        // waited stops it here.
        if let Some(firing) = &line.firing
            && !schedule::scheduled_run_allowed(&room.log, &self.persona_id, firing)
        {
            return Begin::Skip;
        }
        // A firing is running from here, its agent's start included: the
        // next one due meanwhile is skipped, not queued behind it.
        let now = now_ms();
        *lock(&self.last_used) = now;
        if line.firing.is_some() {
            *lock(&self.firing_since) = Some(now);
            lock(&self.overruns).skipped = 0;
        }
        let stale = lock(&self.up)
            .as_ref()
            .is_some_and(|up| !up.lease.is_current());
        if stale {
            self.let_go(room);
        }
        if lock(&self.up).is_none() {
            room.mark_job(self, ThreadState::Live, None);
            match room.job_agent(self, &job).await {
                Ok(up) => *lock(&self.up) = Some(up),
                Err(error) => {
                    lock(&self.firing_since).take();
                    self.notice(
                        room,
                        NoticeLevel::Error,
                        format!("This job could not start its agent: {error}"),
                    );
                    room.mark_job(self, ThreadState::Parked, None);
                    return Begin::Skip;
                }
            }
        }
        if let Some(id) = &line.line.delivery {
            self.heard(room, id);
        }
        Begin::Go(JobTurn {
            line: Some(line.line),
            firing: line.firing,
            last: line.last,
            run: run_of(&job),
        })
    }

    async fn turn(self: &Arc<Self>, room: &Arc<Room>, held: &mut JobTurn) -> Driven {
        let thread = Occupant::thread(&**self);
        let line = held.line.take().expect("a turn is taken once");
        // Stopped while its agent was starting: there is nothing to drive.
        if self.halted.load(Ordering::SeqCst) {
            return Driven::default();
        }
        let (driver, reach) = lock(&self.up)
            .as_ref()
            .map(|up| (up.driver.clone(), up.reach))
            .expect("a job's turn runs once its agent is up");
        room.threads()
            .turn(
                Seat {
                    capability: None,
                    thread: &thread,
                    persona_id: &self.persona_id,
                    driver: driver.as_ref(),
                },
                line,
                reach,
                None,
                Some(held.run.clone()),
                || !self.stopped.load(Ordering::SeqCst),
            )
            .await
    }

    fn end(self: &Arc<Self>, room: &Arc<Room>, held: JobTurn, driven: Driven) -> Then {
        *lock(&self.last_used) = now_ms();
        lock(&self.firing_since).take();
        if self.stopped.load(Ordering::SeqCst) {
            return Then::Stop;
        }
        self.remember_session(room, &driven);
        if driven.asked {
            room.threads()
                .expire_asked(&Occupant::thread(&**self), &self.persona_id);
        }
        let done = self.done.load(Ordering::SeqCst);
        // The job ran out of its count or date with this turn, or while it
        // ran; a firing still waiting is its last, and ends it instead.
        let ended = held.last
            || room::all_jobs(&room.log)
                .into_iter()
                .find(|job| job.id == self.job_id)
                .is_none_or(|job| job.ended_at.is_some());
        let firing_waits = lock(&self.turns)
            .waiting
            .iter()
            .any(|line| line.firing.is_some());
        let ran_out = !done && ended && !firing_waits;
        // A loop that ran out says what came of it, as an end the person or
        // the job made does; a one-shot's one report is its end.
        let closing = ran_out && held.run.kind == ScheduleKind::Loop;
        let halted = self.halted.swap(false, Ordering::SeqCst);
        if halted {
            let said = if ran_out {
                "Stopped. That was the job's last run, so it has ended."
            } else {
                "Stopped. The job keeps its schedule and runs again at its next time."
            };
            self.notice(room, NoticeLevel::Info, said.to_string());
        }
        if let Some(firing) = &held.firing
            && !done
            && !halted
            && !closing
            && !driven.posted
            && firing.quiet != Some(true)
            && let Some(report) = report_of(&driven)
        {
            self.post_as_job(room, firing, &report);
        }
        if done || ran_out {
            // Its last firing is over, or it ended itself: nothing more is
            // taken, and the thread is kept to read.
            let final_word = closing
                .then(|| self.final_word(room, &driven, halted))
                .flatten();
            if let Some(final_word) = &final_word {
                self.post_as_job(room, &held.run, final_word);
            }
            lock(&self.turns).clear();
            room.close_job(self, End::Done, final_word);
            return Then::Stop;
        }
        if held.firing.is_some() {
            self.let_go(room);
        }
        Then::Next
    }

    fn rest(self: &Arc<Self>, room: &Arc<Room>) {
        lock(&self.turns).running = false;
        // A stop that landed after the last turn's end has no turn to stop.
        self.halted.store(false, Ordering::SeqCst);
        let _ = room.info_changes.send(room.info(&self.persona_id));
    }
}

/// What a firing came to, for a job that reports every firing: its last
/// words, or the failure it ended on.
fn report_of(driven: &Driven) -> Option<String> {
    let words = if driven.last_words.is_empty() {
        &driven.replies
    } else {
        &driven.last_words
    };
    let said = words.join("\n\n").trim().to_string();
    if !said.is_empty() {
        return Some(said);
    }
    driven
        .failure
        .as_ref()
        .map(|failure| format!("This run failed: {failure}"))
}

/// One turn of a job's thread, as the next firing is told of it.
struct Turned {
    ts: i64,
    /// What the person said, when the person started it.
    asked: Option<String>,
    /// What the job said after its last tool call: its note.
    said: String,
}

/// The turns of a job's thread that came to something, oldest first.
fn turns_of(events: &[Value]) -> Vec<Turned> {
    let mut turns: Vec<Turned> = Vec::new();
    for event in events {
        let text = event["text"].as_str().unwrap_or_default();
        match event["kind"].as_str() {
            Some("user") => turns.push(Turned {
                ts: event["ts"].as_i64().unwrap_or_default(),
                asked: event.get("scheduled").is_none().then(|| text.to_string()),
                said: String::new(),
            }),
            // What it said before a tool call was on the way, not the note.
            Some("tool") => {
                if let Some(turn) = turns.last_mut() {
                    turn.said.clear();
                }
            }
            Some("agent") => {
                if let Some(turn) = turns.last_mut() {
                    if !turn.said.is_empty() {
                        turn.said.push_str("\n\n");
                    }
                    turn.said.push_str(text.trim());
                }
            }
            _ => {}
        }
    }
    turns.retain(|turn| !turn.said.trim().is_empty());
    turns
}

/// What the job said at the end of its latest turn: its running note.
fn last_note(events: &[Value]) -> Option<String> {
    turns_of(events)
        .pop()
        .map(|turn| crate::driver::clip(&turn.said, NOTE_CHARS))
}

/// Where the job stands, from its own thread: its running note, and its few
/// turns before that. None before its first turn.
fn standing(events: &[Value]) -> Option<String> {
    let turns = turns_of(events);
    let (latest, before) = turns.split_last()?;
    let mut standing = format!(
        "Where this job stood after its last turn, in your own words:\n{}",
        crate::driver::clip(&latest.said, NOTE_CHARS)
    );
    let earlier = &before[before.len().saturating_sub(EARLIER_TURNS)..];
    if !earlier.is_empty() {
        standing.push_str("\n\nIts turns before that, oldest first:");
        for turn in earlier {
            let what = match &turn.asked {
                Some(asked) => format!(
                    "the person said \"{}\"",
                    crate::driver::clip(asked, EARLIER_CHARS)
                ),
                None => "a firing".to_string(),
            };
            let _ = write!(
                standing,
                "\n- {} · {what}: {}",
                clock(turn.ts),
                crate::driver::clip(&turn.said, EARLIER_CHARS)
            );
        }
    }
    Some(standing)
}

/// A moment as a firing line names it: `Mon 14:05`.
fn clock(ts: i64) -> String {
    Local
        .timestamp_millis_opt(ts)
        .single()
        .map(|at| at.format("%a %H:%M").to_string())
        .unwrap_or_default()
}

/// What the job's agent hears when it fires: what woke it and the task, the
/// goal it ends on, whether this is its last firing, and where it stood. A
/// quiet firing says `quiet` and asks nothing of it: the preamble says what
/// that means. The last firing is not asked to end the job, which ends after
/// it anyway; a loop's last is told its words are the job's final post.
fn firing_text(
    job: &ScheduledJob,
    run: &ScheduledRun,
    last: bool,
    standing: Option<String>,
) -> String {
    let waking = match run.kind {
        ScheduleKind::Loop => "loop",
        ScheduleKind::Schedule => "scheduled",
    };
    let mut text = if run.quiet == Some(true) {
        format!("quiet {waking} · {}", job.prompt)
    } else {
        format!("{waking} · {}", job.prompt)
    };
    if let Some(goal) = &job.goal {
        let _ = write!(text, "\n\nThe job's goal: {goal}.");
        if !last {
            text.push_str(" Once it is met, end the job with `cancel_schedule` and a `report`.");
        }
    }
    if last {
        text.push_str(match run.kind {
            ScheduleKind::Loop => "\n\nThis is the job's last run: it ends after this turn, so do not end it with `cancel_schedule`. End your turn with what came of the job across its runs: your last words are posted to the person as its final post, so do not post them with `post_to_chat` as well.",
            ScheduleKind::Schedule => "\n\nThis is the job's only run: it ends after this turn, so do not end it with `cancel_schedule`.",
        });
    }
    if let Some(standing) = standing {
        text.push_str("\n\n");
        text.push_str(&standing);
    }
    text
}

/// An interval that fits work which takes `took` milliseconds: half as long
/// again, at least twice the one it had, in whole minutes.
fn suggested_every(took: i64, every: i64) -> i64 {
    let wanted = (took.saturating_mul(3) / 2).max(every.saturating_mul(2));
    let minutes = (wanted + 59_999) / 60_000;
    (minutes * 60_000).clamp(MIN_LOOP, MAX_SUGGESTED)
}

/// What a job's thread says of a firing it skipped at `at`: how long the one
/// before had been running, or that it was still waiting its turn, and the
/// interval that `fits`.
fn skipped_text(at: &str, took: Option<i64>, fits: &str) -> String {
    let before = match took {
        Some(took) => format!("was still going after {}", spoken(took)),
        None => "was still waiting its turn".to_string(),
    };
    format!(
        "Skipped the {at} run: the one before it {before}. Every {fits} would fit the time this takes."
    )
}

/// A span as a person says it: `45s`, `45m`, `2h`, `1h 30m`. It says what has
/// passed, so a part that has not passed in whole is left out: a minute and a
/// second is `1m`.
fn spoken(ms: i64) -> String {
    let seconds = ms / 1_000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    match (minutes / 60, minutes % 60) {
        (hours, 0) => format!("{hours}h"),
        (hours, rest) => format!("{hours}h {rest}m"),
    }
}

impl Room {
    /// The job's thread as the room holds it, made the first time it is
    /// needed.
    fn live_job(&self, job: &ScheduledJob) -> Arc<LiveJob> {
        let mut held = lock(&self.job_threads);
        held.entry(job.id.clone())
            .or_insert_with(|| {
                let thread = ThreadId::job(&job.id);
                Arc::new(LiveJob {
                    job_id: job.id.clone(),
                    persona_id: job.persona_id.clone(),
                    turns: Mutex::new(Turns {
                        waiting: VecDeque::new(),
                        running: false,
                    }),
                    up: Mutex::new(None),
                    holder: Mutex::new(Holder::new(
                        super::agent::lease_key(&thread),
                        schedule::entry(job.clone()).name,
                    )),
                    saved: Mutex::new(self.link_of(&thread).and_then(|link| link.binding)),
                    firing_since: Mutex::new(None),
                    overruns: Mutex::new(Overruns::default()),
                    last_used: Mutex::new(now_ms()),
                    stopped: AtomicBool::new(false),
                    halted: AtomicBool::new(false),
                    done: AtomicBool::new(false),
                })
            })
            .clone()
    }

    /// The agent for a job's turn: a clone of the home agent, built under the
    /// job kind's policy on a child of the teammate's authority, revoked when
    /// the agent is let go of.
    async fn job_agent(self: &Arc<Self>, live: &LiveJob, job: &ScheduledJob) -> Result<Up, String> {
        let persona = clone_of(
            self.persona(&job.persona_id)?,
            &self.info(&job.persona_id),
            job.without.as_deref().unwrap_or_default(),
        );
        let lease = lease_of(ThreadKind::Job, &self.capability_lease(&job.persona_id));
        lease.check()?;
        let mut agent = self
            .thread_agent(Opening {
                thread: ThreadId::job(&job.id),
                persona,
                title: schedule::entry(job.clone()).name,
                opener: None,
                lease: lease.clone(),
                heard_from: None,
            })
            .await?;
        agent.keep();
        *lock(&live.holder) = agent.holder.clone();
        Ok(Up {
            driver: agent.driver.clone(),
            lease,
            reach: agent.view.reach.unwrap_or_default(),
            backend_id: agent.view.backend_id.clone(),
            reported: agent.reported.clone(),
        })
    }

    /// Hands a firing of `job` to its thread, or skips it while the firing
    /// before is still running, and says whether it ran. The firing's prompt
    /// is written to the thread as it is, stamped with the job; the agent
    /// hears it framed, with where the job stood.
    pub(super) fn fire_job(self: &Arc<Self>, job: &ScheduledJob) -> Result<bool, String> {
        let live = self.live_job(job);
        let run = run_of(job);
        let now = now_ms();
        let busy_since = *lock(&live.firing_since);
        let firing_waits = lock(&live.turns)
            .waiting
            .iter()
            .any(|line| line.firing.is_some());
        if busy_since.is_some() || firing_waits {
            self.skip_firing(&live, job, &run, busy_since.map(|since| now - since));
            return Ok(false);
        }
        let standing = standing(&self.log.load(&StreamId::Job(job.id.clone())));
        let last = schedule::fires_last(job, now);
        live.say(
            self,
            &TranscriptEvent::User {
                id: new_id(),
                ts: now,
                text: job.prompt.clone(),
                attachments: None,
                reactions: None,
                reply_to: None,
                scheduled: Some(run.clone()),
                ring: None,
                receipt: None,
                client: None,
            },
        );
        self.queue_job(
            &live,
            JobLine {
                line: Line {
                    voice: None,
                    text: timed(now, &firing_text(job, &run, last, standing)),
                    attachments: Vec::new(),
                    handoff: None,
                    from: None,
                    delivery: None,
                },
                firing: Some(run),
                last,
            },
        )
        .map(|()| true)
    }

    /// A firing due while the one before is still running: said on the
    /// thread with an interval that fits, and told to the person once when
    /// it keeps happening.
    fn skip_firing(
        &self,
        live: &LiveJob,
        job: &ScheduledJob,
        run: &ScheduledRun,
        took: Option<i64>,
    ) {
        let every = job.every.unwrap_or(MIN_LOOP);
        let fits = spoken(suggested_every(took.unwrap_or(every), every));
        live.notice(
            self,
            NoticeLevel::Warn,
            skipped_text(&clock(now_ms()), took, &fits),
        );
        let tell = {
            let mut overruns = lock(&live.overruns);
            overruns.skipped += 1;
            let tell = overruns.skipped >= OVERRUNS_TO_TELL && !overruns.told;
            overruns.told |= tell;
            tell
        };
        if tell {
            let said = format!(
                "\"{}\" keeps running past its {}: each run takes longer than that, so I have been skipping runs. Every {fits} would fit; ask me to change it.",
                run.name,
                spoken(every)
            );
            if let Err(error) = self.post(
                &job.persona_id,
                &ThreadId::job(&job.id),
                Post {
                    scheduled: Some(run.clone()),
                    ..Post::words(&said)
                },
            ) {
                eprintln!("a job's overrun could not be told: {error}");
            }
        }
    }

    /// Hands a line to the job's agent now, or behind the turn in flight.
    fn queue_job(self: &Arc<Self>, live: &Arc<LiveJob>, line: JobLine) -> Result<(), String> {
        let Some(queued) = lock(&live.turns).claim(line) else {
            return Ok(());
        };
        let working = match self.lease() {
            Ok(working) => working,
            Err(error) => {
                lock(&live.turns).running = false;
                return Err(error);
            }
        };
        let _ = self.info_changes.send(self.info(&live.persona_id));
        let room = self.clone();
        let live = live.clone();
        tokio::spawn(async move {
            let _working = working;
            room.run_queue(live, queued).await;
        });
        Ok(())
    }

    /// The person says something in a job's thread: a turn like a firing's,
    /// on the same agent. An agent the job let go of is built afresh, and is
    /// told where the job stands.
    pub async fn prompt_job(
        self: &Arc<Self>,
        job_id: &str,
        text: &str,
        attachments: Option<Vec<Attachment>>,
    ) -> Result<(), String> {
        let _working = self.working()?;
        let job = room::all_jobs(&self.log)
            .into_iter()
            .find(|job| job.id == job_id)
            .ok_or_else(|| "There is no such job.".to_string())?;
        if job.ended_at.is_some() {
            return Err("This job has ended. Its thread is kept to read.".to_string());
        }
        self.capability_lease(&job.persona_id).check()?;
        let live = self.live_job(&job);
        let fresh = lock(&live.up).is_none();
        let standing = fresh
            .then(|| standing(&self.log.load(&StreamId::Job(job.id.clone()))))
            .flatten();
        let ts = now_ms();
        let client = crate::wire::commands::prompt_client();
        *lock(&live.last_used) = ts;
        live.say(
            self,
            &TranscriptEvent::User {
                id: new_id(),
                ts,
                text: text.to_string(),
                attachments: attachments.clone(),
                reactions: None,
                reply_to: None,
                scheduled: None,
                ring: None,
                receipt: None,
                client,
            },
        );
        let heard = match standing {
            Some(standing) => format!("({standing})\n\n{text}"),
            None => text.to_string(),
        };
        self.queue_job(
            &live,
            JobLine {
                line: Line {
                    voice: None,
                    text: timed_from(ts, client, &heard),
                    attachments: attachments.unwrap_or_default(),
                    handoff: None,
                    from: None,
                    delivery: None,
                },
                firing: None,
                last: false,
            },
        )
    }

    /// Whether a job is still on the room stream and has not ended: what it
    /// asked can still come back to its thread.
    pub(super) fn job_lives(&self, job_id: &str) -> bool {
        room::all_jobs(&self.log)
            .iter()
            .any(|job| job.id == job_id && job.ended_at.is_none())
    }

    /// Something the job asked for came back (the person's answer to its
    /// card, a colleague's answer to its message): written to its thread
    /// and handed to its agent as a turn, as a line the person says there
    /// is. Handing the same delivery over twice hands it over once.
    pub(super) fn deliver_into_job(
        self: &Arc<Self>,
        job_id: &str,
        id: &str,
        cause: DeliveryCause,
        from: DeliveryFrom,
        text: String,
    ) -> Result<(), String> {
        let job = room::all_jobs(&self.log)
            .into_iter()
            .find(|job| job.id == job_id && job.ended_at.is_none())
            .ok_or_else(|| "This job has ended.".to_string())?;
        let live = self.live_job(&job);
        let existing = self
            .log
            .load(&StreamId::Job(job_id.to_string()))
            .into_iter()
            .find(|event| event["id"] == id);
        let queued = lock(&live.turns)
            .waiting
            .iter()
            .any(|waiting| waiting.line.delivery.as_deref() == Some(id));
        if queued
            || existing
                .as_ref()
                .is_some_and(|event| event["receipt"] == "read")
        {
            return Ok(());
        }
        let ts = existing
            .as_ref()
            .and_then(|event| event["ts"].as_i64())
            .unwrap_or_else(now_ms);
        let wire = super::peers::delivery_wire(&cause, &text);
        *lock(&live.last_used) = now_ms();
        if existing.is_none() {
            live.say(
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
        self.queue_job(
            &live,
            JobLine {
                line: Line {
                    voice: None,
                    text: timed(ts, &wire),
                    attachments: Vec::new(),
                    handoff: None,
                    from: Some(from),
                    delivery: Some(id.to_string()),
                },
                firing: None,
                last: false,
            },
        )
    }

    /// What came back to a job and was written to its thread, but whose turn
    /// had not begun when the last process stopped: the queue it waited in
    /// died with that process, so it is handed over again, once. A job that
    /// has ended since can hear nothing more, so what it was owed goes to its
    /// teammate's DM instead.
    pub(super) async fn resume_job_deliveries(self: &Arc<Self>) {
        for job in room::all_jobs(&self.log) {
            let unread: Vec<(String, DeliveryCause, Option<DeliveryFrom>, String)> = self
                .log
                .load(&StreamId::Job(job.id.clone()))
                .into_iter()
                .filter(|event| event["kind"] == "delivery" && event["receipt"] != "read")
                .filter_map(|event| match serde_json::from_value(event) {
                    Ok(TranscriptEvent::Delivery {
                        id,
                        cause,
                        from,
                        text,
                        ..
                    }) => Some((id, cause, from, text)),
                    _ => None,
                })
                .collect();
            for (id, cause, from, text) in unread {
                let handed = match from {
                    Some(from) if job.ended_at.is_none() => {
                        self.deliver_into_job(&job.id, &id, cause, from, text)
                    }
                    _ => {
                        self.deliver_identified(&job.persona_id, &id, cause, text)
                            .await
                    }
                };
                if let Err(error) = handed {
                    eprintln!(
                        "[restart] what came back to job {} was not handed over: {error}",
                        job.id
                    );
                }
            }
        }
    }

    /// The human-action cards raised in this teammate's jobs, with the
    /// thread each is on.
    pub(super) fn cards_in_jobs(&self, persona_id: &str) -> Vec<(Value, ThreadId)> {
        room::all_jobs(&self.log)
            .into_iter()
            .filter(|job| job.persona_id == persona_id)
            .flat_map(|job| {
                let thread = ThreadId::job(&job.id);
                self.log
                    .load(&StreamId::Job(job.id))
                    .into_iter()
                    .filter(|event| event["kind"] == "human_action")
                    .map(move |event| (event, thread.clone()))
            })
            .collect()
    }

    /// Answers a permission card raised inside a job's thread, and writes
    /// the card as decided.
    pub fn answer_job_permission(
        &self,
        job_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), String> {
        let live = lock(&self.job_threads).get(job_id).cloned();
        let answered = live.as_ref().is_some_and(|live| {
            lock(&live.up)
                .as_ref()
                .is_some_and(|up| up.driver.answer_permission(request_id, option_id))
        });
        let Some(live) = live.filter(|_| answered) else {
            return Err("That request is no longer waiting for an answer.".to_string());
        };
        let id = Value::from(format!("perm:{request_id}"));
        let card = self
            .log
            .load(&StreamId::Job(job_id.to_string()))
            .into_iter()
            .find(|event| event.get("id") == Some(&id))
            .and_then(|event| serde_json::from_value::<TranscriptEvent>(event).ok());
        if let Some(TranscriptEvent::Permission {
            id,
            request_id,
            title,
            options,
            ..
        }) = card
        {
            let decided_option_name = options
                .iter()
                .find(|option| option.option_id == option_id)
                .map(|option| option.name.clone());
            live.say(
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
        }
        Ok(())
    }

    /// Stops the turn in flight in a job's thread, a firing's or the
    /// person's, and drops what waited behind it. The job goes on firing:
    /// its next time is the one it already had.
    pub fn cancel_job_turn(&self, job_id: &str) -> Result<(), String> {
        let live = lock(&self.job_threads)
            .get(job_id)
            .cloned()
            .filter(|live| lock(&live.turns).running)
            .ok_or_else(|| "That job has no turn running.".to_string())?;
        live.halted.store(true, Ordering::SeqCst);
        live.interrupt();
        Ok(())
    }

    /// The jobs of this teammate with a turn running, by id, for its roster
    /// row.
    pub fn working_jobs(&self, persona_id: &str) -> Vec<String> {
        lock(&self.job_threads)
            .values()
            .filter(|live| live.persona_id == persona_id && lock(&live.turns).running)
            .map(|live| live.job_id.clone())
            .collect()
    }

    /// A job has ended: the person cancelled it, or it met its goal and said
    /// what came of it in `report`. Its final post says so, and its thread is
    /// closed and kept. A job the person cancelled has its turn in flight
    /// stopped; one that ended itself finishes the turn it ended in.
    pub(super) fn job_ended(&self, job: &ScheduledJob, report: Option<&str>) {
        let thread = ThreadId::job(&job.id);
        let live = lock(&self.job_threads).get(&job.id).cloned();
        let ended = if report.is_some() {
            "Done"
        } else {
            "Cancelled"
        };
        let final_word = match report.map(str::trim).filter(|report| !report.is_empty()) {
            Some(report) => Some(report.to_string()),
            None => last_note(&self.log.load(&StreamId::Job(job.id.clone())))
                .map(|note| format!("{ended}. Where it stood: {note}")),
        };
        if let Some(text) = &final_word {
            let posted = self.post(
                &job.persona_id,
                &thread,
                Post {
                    scheduled: Some(run_of(job)),
                    ..Post::words(&crate::driver::clip(text, POST_CHARS))
                },
            );
            if let Err(error) = posted {
                eprintln!("a job's final post could not be written: {error}");
            }
        }
        let by = if report.is_some() {
            End::Agent
        } else {
            End::Person
        };
        match live {
            Some(live) if report.is_some() && lock(&live.turns).running => {
                // The turn that ended it closes the thread when it is over.
                live.done.store(true, Ordering::SeqCst);
                lock(&live.turns).clear();
                self.mark_job(&live, ThreadState::Closed(by), final_word);
            }
            Some(live) => {
                live.stopped.store(true, Ordering::SeqCst);
                live.interrupt();
                live.let_go(self);
                lock(&self.job_threads).remove(&job.id);
                self.mark_job(&live, ThreadState::Closed(by), final_word);
            }
            None => {
                if let Some(link) = self.link_of(&thread) {
                    self.mark_job_thread(
                        &job.id,
                        ThreadState::Closed(by),
                        final_word,
                        link.binding,
                    );
                }
            }
        }
    }

    /// A job whose last firing is over, or which ended itself, lets go of its
    /// agent and closes its thread with what came of it, unless the end
    /// already closed it.
    fn close_job(&self, live: &LiveJob, end: End, outcome: Option<String>) {
        live.let_go(self);
        lock(&self.job_threads).remove(&live.job_id);
        let closed = self
            .link_of(&live.thread())
            .is_some_and(|link| matches!(link.state, ThreadState::Closed(_)));
        if !closed {
            self.mark_job(live, ThreadState::Closed(end), outcome);
        }
    }

    /// The job's link, at the head of its own stream.
    fn mark_job(&self, live: &LiveJob, state: ThreadState, outcome: Option<String>) {
        let binding = lock(&live.saved).clone();
        self.mark_job_thread(&live.job_id, state, outcome, binding);
    }

    fn mark_job_thread(
        &self,
        job_id: &str,
        state: ThreadState,
        outcome: Option<String>,
        binding: Option<AgentBinding>,
    ) {
        let thread = ThreadId::job(job_id);
        let Some(job) = room::all_jobs(&self.log)
            .into_iter()
            .find(|job| job.id == job_id)
        else {
            return;
        };
        self.write_link(&Link {
            id: self.link_id(&thread),
            ts: job.created_at,
            thread,
            persona_id: Some(job.persona_id.clone()),
            title: schedule::entry(job).name,
            state,
            outcome: outcome.map(|outcome| crate::driver::clip(&outcome, OUTCOME_CHARS)),
            at: matches!(state, ThreadState::Closed(_)).then(now_ms),
            note: None,
            binding,
            elapsed_ms: None,
            opener: None,
        });
    }

    /// The live jobs whose agent the sweep may park: idle, with an agent.
    pub(super) fn idle_jobs(&self) -> Vec<(ThreadId, i64, bool)> {
        lock(&self.job_threads)
            .values()
            .filter(|live| lock(&live.up).is_some())
            .map(|live| {
                (
                    live.thread(),
                    *lock(&live.last_used),
                    lock(&live.turns).running,
                )
            })
            .collect()
    }

    /// Lets go of a job's agent: the sweep found it idle.
    pub(super) fn park_job(&self, job_id: &str) {
        let live = lock(&self.job_threads).get(job_id).cloned();
        if let Some(live) = live {
            live.let_go(self);
        }
    }

    /// Stops every job agent a teammate has, or every one in the room: its
    /// authority changed or it was stopped. The jobs go on firing, each on an
    /// agent built under whatever the teammate has next.
    pub(super) fn let_go_of_jobs(&self, persona_id: Option<&str>) {
        let live: Vec<Arc<LiveJob>> = lock(&self.job_threads)
            .values()
            .filter(|live| persona_id.is_none_or(|id| live.persona_id == id))
            .cloned()
            .collect();
        for live in live {
            live.interrupt();
            live.let_go(self);
        }
    }

    /// What the room does with a job's turn when it is stopping for a
    /// restart and the drain ran out: stop it, and say so in the thread.
    pub(super) fn interrupt_jobs(&self) {
        let live: Vec<Arc<LiveJob>> = lock(&self.job_threads).values().cloned().collect();
        for live in live {
            if !lock(&live.turns).running {
                continue;
            }
            live.interrupt();
            live.notice(
                self,
                NoticeLevel::Warn,
                "Hotline restarted while this turn was running, so it was stopped. The job fires again at its next time.".to_string(),
            );
        }
    }

    /// A job thread the last process left live had its agent die with it: it
    /// is parked, and its next firing builds one. A card it left open is a
    /// button nobody is behind.
    pub(super) fn settle_jobs(&self, now: i64) {
        for job in room::all_jobs(&self.log) {
            let thread = ThreadId::job(&job.id);
            let Some(link) = self.link_of(&thread) else {
                continue;
            };
            let stream = StreamId::Job(job.id.clone());
            let mut lines = crate::log::expire_orphaned_permissions(&self.log.load(&stream), now);
            if link.state == ThreadState::Live {
                lines.push(
                    Link {
                        state: ThreadState::Parked,
                        ..link
                    }
                    .event(),
                );
            }
            for line in lines {
                if let Err(error) = self.log.append(&stream, &line) {
                    eprintln!("could not settle the {thread}: {error}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_firing_starts_from_its_last_turn_and_the_few_before() {
        let events = vec![
            json!({"kind":"user","id":"u1","ts":1,"text":"check","scheduled":{"jobId":"j","kind":"loop","name":"check"}}),
            json!({"kind":"agent","id":"a1","ts":2,"text":"Looking."}),
            json!({"kind":"tool","id":"t1","ts":3}),
            json!({"kind":"agent","id":"a2","ts":4,"text":"Order still processing."}),
            json!({"kind":"user","id":"u2","ts":5,"text":"skip the next one"}),
            json!({"kind":"agent","id":"a3","ts":6,"text":"Will skip the next check."}),
        ];
        let standing = standing(&events).unwrap();
        assert!(
            standing.starts_with("Where this job stood after its last turn, in your own words:\nWill skip the next check."),
            "{standing}"
        );
        assert!(
            standing.contains("a firing: Order still processing."),
            "{standing}"
        );
        assert!(
            !standing.contains("Looking."),
            "only what it said after its tools"
        );
        assert_eq!(
            last_note(&events).as_deref(),
            Some("Will skip the next check.")
        );
    }

    #[test]
    fn a_job_that_never_ran_has_no_standing() {
        assert_eq!(standing(&[]), None);
        let fired = vec![json!({"kind":"user","id":"u1","ts":1,"text":"check"})];
        assert_eq!(standing(&fired), None);
    }

    #[test]
    fn a_skip_suggests_an_interval_that_fits_the_work() {
        assert_eq!(suggested_every(20 * 60_000, 15 * 60_000), 30 * 60_000);
        assert_eq!(suggested_every(50 * 60_000, 15 * 60_000), 75 * 60_000);
        assert_eq!(suggested_every(1_000, 15_000), 60_000);
        assert_eq!(spoken(30 * 60_000), "30m");
        assert_eq!(spoken(75 * 60_000), "1h 15m");
        assert_eq!(spoken(2 * 3_600_000), "2h");
        assert_eq!(spoken(15_000), "15s");
    }

    #[test]
    fn a_skip_says_how_long_the_firing_before_has_run() {
        // A one-minute loop whose firing began a minute and a half second ago.
        let said = skipped_text(
            "Tue 01:13",
            Some(60_500),
            &spoken(suggested_every(60_500, 60_000)),
        );
        assert_eq!(
            said,
            "Skipped the Tue 01:13 run: the one before it was still going after 1m. Every 2m would fit the time this takes."
        );
        assert_eq!(spoken(119_999), "1m");
        assert_eq!(spoken(59_999), "59s");
        let waiting = skipped_text("Tue 01:13", None, "2m");
        assert!(
            waiting.contains("the one before it was still waiting its turn."),
            "{waiting}"
        );
    }

    #[test]
    fn a_quiet_firing_says_quiet_and_names_its_goal() {
        let job = ScheduledJob {
            id: "j".to_string(),
            persona_id: "ada".to_string(),
            kind: ScheduleKind::Loop,
            when: None,
            every: Some(60_000),
            prompt: "watch CI".to_string(),
            quiet: Some(true),
            reminder: None,
            goal: Some("CI is green".to_string()),
            until: None,
            times: None,
            without: None,
            operator_created: true,
            next_at: 1,
            created_at: 1,
            last_post: None,
            ended_at: None,
        };
        let text = firing_text(
            &job,
            &run_of(&job),
            false,
            Some("Where it stood".to_string()),
        );
        assert!(text.starts_with("quiet loop · watch CI"), "{text}");
        assert!(text.contains("The job's goal: CI is green."), "{text}");
        assert!(text.ends_with("Where it stood"), "{text}");
        assert!(!text.contains("last run"), "{text}");

        let last = firing_text(&job, &run_of(&job), true, None);
        assert!(
            last.contains("The job's goal: CI is green.\n\nThis is the job's last run"),
            "the last firing is not asked to end the job: {last}"
        );
        assert!(
            last.contains("do not end it with `cancel_schedule`"),
            "{last}"
        );
    }
}
