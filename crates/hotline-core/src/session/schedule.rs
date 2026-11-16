//! Durable wakes for teammates: the clock of their jobs.
//!
//! Jobs live on the room stream. This module is the clock: it folds them,
//! sleeps until the nearest `nextAt`, and fires each one. A loop runs every
//! interval, counted from when a firing starts, or at a time of day on
//! chosen days ([`calendar`]), each next run worked out on the desk's clock.
//! A plain reminder is posted to the teammate's chat as it was written
//! ([`Room::post`]) and does no work; any other job's firing is a turn on
//! the job's own thread ([`super::job_thread`]), never the teammate's DM.
//! Missed times, while Hotline was closed, the machine asleep or background
//! work off, fire once when the desk can rather than catching up a pile of
//! them — the teammate should do the work now, not replay every interval it
//! slept through. A loop's run that could only start more than two hours
//! after its time ([`LATE_RUN_LIMIT`]) is not run at all: the loop moves on
//! to its next time and the job keeps the skip (`skipped`). A one-shot runs
//! however late, since it has no next time to wait for.
//!
//! A job and a subagent are one thing with one schema ([`Brief`], ADR 0008
//! §7): the task, when it runs, whom it reports to, and what ends it. The
//! `subagent` tool writes one that runs now and reports to the agent that
//! asked; `schedule` and `loop` write one that runs at a time or every
//! interval and reports to the person, and this module keeps it.
//!
//! `parse_duration` and `parse_when` are the agent-facing sugar. The wire
//! takes milliseconds; the teammate tools `schedule` and `loop` speak these
//! strings and call this module.

use super::{Post, Room, lock, now_ms};
use crate::contract::{
    Ability, Calendar, ScheduleEntry, ScheduleKind, ScheduledJob, ScheduledRun, SkippedRun,
    ThreadId,
};
use crate::log::Log;
use crate::room;
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, TimeZone, Timelike};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::Notify;

/// Shorter than this is a busy-loop dressed as a schedule.
pub(super) const MIN_LOOP: i64 = 15_000;
/// Longer than a week is a calendar, not a loop.
const MAX_LOOP: i64 = 7 * 86_400_000;
/// Further than a month is a reminder the person will have forgotten why.
const MAX_AHEAD: i64 = 30 * 86_400_000;
/// A teammate that schedules itself into a crowd is usually stuck in a loop
/// of its own making.
const MAX_JOBS: usize = 20;
const MIN_WAIT: i64 = 1_000;
const MAX_PROMPT: usize = 8_000;
/// A goal is a sentence the job checks itself against, not a second prompt.
const MAX_GOAL: usize = 500;
/// How long a failed fire waits before trying again.
const RETRY_WAIT: i64 = 60_000;
/// How late a loop's run may start. George, 2026-11-15: "skip after 2 hours
/// from original time". A run the desk could not start until later than
/// this after its time is skipped, and the loop waits for its next.
const LATE_RUN_LIMIT: i64 = 2 * 3_600_000;
/// Sets [`LATE_RUN_LIMIT`] in milliseconds, so the rule can be watched on a
/// running desk without waiting two hours.
const LATE_RUN_LIMIT_ENV: &str = "HOTLINE_LATE_RUN_LIMIT_MS";
/// How long the clock will sleep before looking again, even when the next job
/// is further away. The task holds the room weakly, and this is how often it
/// notices the room is gone; a create notifies and does not wait.
const LOOK_AGAIN: Duration = Duration::from_secs(60);
/// Longer than this stops being a name and starts being the prompt again.
const SCHEDULE_NAME_MAX: usize = 48;
/// What a calendar loop writes as its `every`, so a build that knows only
/// `every` still reads it as a loop that runs once a day.
const DAY: i64 = 86_400_000;

pub(crate) mod calendar;

/// What a teammate is asked to do away from its home thread: one schema for
/// a subagent and a job (ADR 0008 §7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Brief {
    pub task: String,
    pub trigger: Trigger,
    pub report: Report,
    pub ends: Ends,
    /// What the home agent left out of the clone's permission set for this
    /// task: see [`Ability`]. Empty for work the person asked for.
    pub without: Vec<Ability>,
}

/// When the work runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// At once: a subagent.
    Now,
    /// Once, at this moment, milliseconds since the epoch.
    At(i64),
    /// Every this many milliseconds, until it ends.
    Every(i64),
    /// At a time of day on chosen days, on the desk's clock, until it ends.
    Calendar(Calendar),
}

/// Whom the work reports to, and how often.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Report {
    /// The agent that asked, as its tool's result: a subagent's report.
    Asker,
    /// The person, each firing: what the firing came to is posted to the chat.
    EachFiring,
    /// The person, only what is worth telling: the firing posts it itself,
    /// and one that finds nothing says nothing in the chat.
    Significant,
}

/// What ends a recurring job, besides the person cancelling it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Ends {
    /// The goal it ends itself on reaching.
    pub goal: Option<String>,
    /// The last moment it fires.
    pub at: Option<i64>,
    /// How many times it fires.
    pub after: Option<u32>,
}

impl Brief {
    /// What the `subagent` tool writes: the task, now, reported to the agent
    /// that handed it over.
    pub(crate) fn subagent(task: String, without: Vec<Ability>) -> Self {
        Self {
            task,
            trigger: Trigger::Now,
            report: Report::Asker,
            ends: Ends::default(),
            without,
        }
    }

    /// A job as the desk and the voice dispatcher say one: a one-shot at
    /// `when`, or a loop of `every`, quiet or not.
    pub(crate) fn job(
        kind: ScheduleKind,
        when: Option<i64>,
        every: Option<i64>,
        prompt: &str,
        quiet: bool,
    ) -> Result<Self, String> {
        let trigger = match kind {
            ScheduleKind::Schedule => {
                if every.is_some() {
                    return Err("A one-shot schedule cannot have every.".to_string());
                }
                Trigger::At(when.ok_or_else(|| {
                    "A one-shot schedule needs when, in milliseconds since epoch.".to_string()
                })?)
            }
            ScheduleKind::Loop => {
                if when.is_some() {
                    return Err("A loop cannot have when.".to_string());
                }
                Trigger::Every(
                    every.ok_or_else(|| "A loop needs every, in milliseconds.".to_string())?,
                )
            }
        };
        Ok(Self {
            task: prompt.to_string(),
            trigger,
            report: if quiet {
                Report::Significant
            } else {
                Report::EachFiring
            },
            ends: Ends::default(),
            without: Vec::new(),
        })
    }
}

const UNITS: &[(&str, i64)] = &[
    ("s", 1_000),
    ("sec", 1_000),
    ("secs", 1_000),
    ("second", 1_000),
    ("seconds", 1_000),
    ("m", 60_000),
    ("min", 60_000),
    ("mins", 60_000),
    ("minute", 60_000),
    ("minutes", 60_000),
    ("h", 3_600_000),
    ("hr", 3_600_000),
    ("hrs", 3_600_000),
    ("hour", 3_600_000),
    ("hours", 3_600_000),
    ("d", 86_400_000),
    ("day", 86_400_000),
    ("days", 86_400_000),
];

/// A duration like `20m` or `1.5h`, as milliseconds.
pub fn parse_duration(value: &str) -> Option<i64> {
    let value = value.trim().to_ascii_lowercase();
    let unit_at = value.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let amount: f64 = value[..unit_at].parse().ok()?;
    if !amount.is_finite() || amount <= 0.0 {
        return None;
    }
    let unit = value[unit_at..].trim();
    let ms = UNITS
        .iter()
        .find(|(name, _)| *name == unit)
        .map(|(_, ms)| *ms)?;
    Some((amount * ms as f64).round() as i64)
}

/// When a job should fire: a duration from `now`, an epoch-millisecond
/// number, or a date and time, read on this machine's clock (see
/// [`parse_when_in`]).
pub fn parse_when(value: &str, now: i64) -> Option<i64> {
    parse_when_in(value, now, &Local)
}

/// [`parse_when`] on `zone`'s clock. A date and time with an offset (`Z`,
/// `+01:00`) is that instant. One without, or a bare date, which is its
/// midnight, is a time on `zone`'s wall clock, because that is the clock a
/// teammate is shown: every line it reads opens with the local time and no
/// offset. [`local_instant`] says which instant a wall time that the clocks
/// skip or repeat is.
pub(crate) fn parse_when_in<Tz: TimeZone>(value: &str, now: i64, zone: &Tz) -> Option<i64> {
    if let Some(relative) = parse_duration(value) {
        return Some(now.saturating_add(relative));
    }
    let trimmed = value.trim();
    if let Ok(ms) = trimmed.parse::<i64>()
        && ms > 1_000_000_000_000
    {
        return Some(ms);
    }
    parse_datetime(trimmed, zone)
}

/// The ways a date and time is written that carry their own offset.
const WITH_OFFSET: &[&str] = &["%Y-%m-%dT%H:%M%#z", "%Y-%m-%d %H:%M:%S%.f%#z"];
/// The ways one is written on a wall clock.
const WALL: &[&str] = &[
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%dT%H:%M",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%d %H:%M",
];

fn parse_datetime<Tz: TimeZone>(value: &str, zone: &Tz) -> Option<i64> {
    if let Ok(at) = DateTime::parse_from_rfc3339(value) {
        return Some(at.timestamp_millis());
    }
    if let Some(at) = WITH_OFFSET
        .iter()
        .find_map(|format| DateTime::parse_from_str(value, format).ok())
    {
        return Some(at.timestamp_millis());
    }
    let wall = WALL
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
        .or_else(|| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .map(|date| date.and_time(NaiveTime::MIN))
        })?;
    local_instant(zone, wall)
}

/// The instant a wall-clock time names on `zone`, decided the same way every
/// time. A time the clocks pass twice, as they go back, is the first time
/// they pass it. A time the clocks skip, as they go forward, is the first
/// moment after it that exists, which is the moment they changed: 2:30 on
/// the morning the clocks go from 2:00 to 3:00 is 3:00.
pub(crate) fn local_instant<Tz: TimeZone>(zone: &Tz, wall: NaiveDateTime) -> Option<i64> {
    if let Some(at) = zone.from_local_datetime(&wall).earliest() {
        return Some(at.timestamp_millis());
    }
    // Clocks change on a whole minute, so the first whole minute after a
    // skipped time that exists is when they changed.
    let mut minute = wall.with_second(0)?.with_nanosecond(0)?;
    for _ in 0..MINUTES_A_CLOCK_MAY_SKIP {
        minute = minute.checked_add_signed(TimeDelta::minutes(1))?;
        if let Some(at) = zone.from_local_datetime(&minute).earliest() {
            return Some(at.timestamp_millis());
        }
    }
    None
}

/// No zone has ever moved its clocks by more than a day at once.
const MINUTES_A_CLOCK_MAY_SKIP: u32 = 24 * 60;

/// A job's short name: the first breath of its prompt, clipped so a firing
/// draws as a line rather than as the whole instruction.
fn schedule_name(prompt: &str) -> String {
    let first_line = prompt
        .lines()
        .find(|line| !collapse(line).is_empty())
        .unwrap_or("");
    let derived = clip(&strip_punct(&collapse(first_line)));
    if derived.is_empty() {
        "scheduled work".to_string()
    } else {
        derived
    }
}

fn collapse(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_punct(value: &str) -> String {
    value
        .trim_end_matches(|c: char| {
            c.is_whitespace() || matches!(c, ':' | '.' | ',' | ';' | '—' | '-')
        })
        .to_string()
}

fn clip(value: &str) -> String {
    if value.chars().count() <= SCHEDULE_NAME_MAX {
        return value.to_string();
    }
    let mut clipped: String = value.chars().take(SCHEDULE_NAME_MAX - 1).collect();
    clipped = clipped.trim_end().to_string();
    clipped.push('…');
    clipped
}

/// What a firing of `job` is stamped with, wherever it lands.
pub(super) fn run_of(job: &ScheduledJob) -> ScheduledRun {
    ScheduledRun {
        job_id: job.id.clone(),
        kind: job.kind,
        name: schedule_name(&job.prompt),
        operator_created: job.operator_created,
        quiet: job.quiet.filter(|quiet| *quiet),
    }
}

/// When a loop that fired, or skipped a run, at `now` fires next: its
/// calendar's next time after `now` on the desk's clock, or `every` after
/// `now`.
pub(super) fn next_after(job: &ScheduledJob, now: i64) -> i64 {
    next_after_in(job, now, &Local)
}

/// [`next_after`] on `zone`'s clock. A calendar this build cannot work a
/// time from, written by a newer one, falls back to its daily `every`.
pub(super) fn next_after_in<Tz: TimeZone>(job: &ScheduledJob, now: i64, zone: &Tz) -> i64 {
    let every = || now.saturating_add(job.every.unwrap_or(MIN_LOOP));
    match &job.calendar {
        Some(calendar) => calendar::next_slot(calendar, now, zone).unwrap_or_else(every),
        None => every(),
    }
}

/// A job as the schedules view lists it.
pub(crate) fn entry(job: ScheduledJob) -> ScheduleEntry {
    ScheduleEntry {
        name: schedule_name(&job.prompt),
        id: job.id,
        persona_id: job.persona_id,
        kind: job.kind,
        prompt: job.prompt,
        when: job.when,
        every: job.every,
        calendar: job.calendar,
        next_at: job.next_at,
        quiet: job.quiet,
        reminder: job.reminder,
        last_post: job.last_post,
        ended_at: job.ended_at,
        without: job.without,
        skipped: job.skipped,
    }
}

/// Whether a scheduled job may start a turn now.
///
/// The operator's desk command records its provenance on the job, so that
/// deliberate automation remains independent of a teammate's autonomous
/// background-work grant. Every other job reads the live persona record here;
/// the check is intentionally repeated by the job's thread just before its
/// turn begins, so a grant revoked while a firing waits cannot become a turn.
pub(crate) fn scheduled_job_allowed(log: &Log, persona_id: &str, job_id: &str) -> bool {
    let Some(job) = room::schedules(log)
        .into_iter()
        .find(|job| job.id == job_id && job.persona_id == persona_id)
    else {
        return false;
    };
    let run = run_of(&job);
    scheduled_run_allowed(log, persona_id, &run)
}

/// Whether a queued firing may begin its turn now.
///
/// The clock's job lookup is not enough at this point: a one-shot has ended
/// as soon as its firing is queued. The run therefore carries the trusted
/// operator bit captured at fire time, while the teammate grant is read live
/// so revocation still stops a queued firing.
pub(crate) fn scheduled_run_allowed(log: &Log, persona_id: &str, run: &ScheduledRun) -> bool {
    run.operator_created || background_work_allowed(log, persona_id)
}

/// Whether a teammate has the standing grant needed by jobs it creates.
pub(crate) fn background_work_allowed(log: &Log, persona_id: &str) -> bool {
    room::roster(log)
        .into_iter()
        .find(|persona| persona.id == persona_id)
        .is_some_and(|persona| persona.background_work)
}

fn persona_exists(log: &Log, persona_id: &str) -> bool {
    room::roster(log)
        .into_iter()
        .any(|persona| persona.id == persona_id)
}

fn should_consider(log: &Log, job: &ScheduledJob) -> bool {
    job.operator_created
        || background_work_allowed(log, &job.persona_id)
        || !persona_exists(log, &job.persona_id)
}

/// Starts the clock. One task for the whole room, holding it weakly so the
/// room's own end is what stops it.
pub(super) fn start(room: Weak<Room>, changed: Arc<Notify>) {
    tokio::spawn(async move {
        loop {
            let notified = changed.notified();
            tokio::pin!(notified);
            let Some(room) = room.upgrade() else {
                return;
            };
            let jobs = room::schedules(&room.log);
            let now = now_ms();
            let due: Vec<ScheduledJob> = jobs
                .iter()
                .filter(|job| {
                    job.final_firing.is_none()
                        && job.next_at <= now
                        && should_consider(&room.log, job)
                })
                .cloned()
                .collect();
            if !due.is_empty() {
                for job in due {
                    fire(&room, job);
                }
                // A write that failed leaves the job due; wait a beat rather
                // than spinning on a disk that cannot take the next word.
                let still_due = room::schedules(&room.log)
                    .iter()
                    .any(|job| job.final_firing.is_none() && job.next_at <= now_ms());
                drop(room);
                if still_due {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                continue;
            }
            let wait = jobs
                .iter()
                .filter(|job| job.final_firing.is_none() && should_consider(&room.log, job))
                .map(|job| job.next_at.saturating_sub(now))
                .min()
                .map_or(LOOK_AGAIN, |ms| Duration::from_millis(ms as u64))
                .min(LOOK_AGAIN);
            drop(room);
            tokio::select! {
                () = notified => {}
                () = tokio::time::sleep(wait) => {}
            }
        }
    });
}

/// How late a loop's run may start: [`LATE_RUN_LIMIT`], or what
/// [`LATE_RUN_LIMIT_ENV`] says.
fn late_run_limit() -> i64 {
    std::env::var(LATE_RUN_LIMIT_ENV)
        .ok()
        .and_then(|ms| ms.trim().parse::<i64>().ok())
        .filter(|ms| *ms >= 0)
        .unwrap_or(LATE_RUN_LIMIT)
}

/// Whether a run due at `slot` is too late to start at `now`: more than
/// `limit` after it. A run exactly `limit` late still runs.
pub(super) fn too_late(slot: i64, now: i64, limit: i64) -> bool {
    now.saturating_sub(slot) > limit
}

/// What became of a due time of a job's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// Its firing was handed over, or its reminder posted.
    Ran,
    /// The firing before was still going: its thread skipped this one.
    Overlapped,
    /// A loop's time passed more than [`LATE_RUN_LIMIT`] before the desk
    /// could start it.
    TooLate,
}

/// Fires one due job: a reminder is posted, any other job's firing is handed
/// to its thread. Neither waits for the work, so a loop counts its next
/// interval from when this firing started, and one still running when the
/// next is due is what its thread skips. A loop's time the desk reaches more
/// than [`LATE_RUN_LIMIT`] late is skipped instead of run. A skipped slot is
/// not a firing, so it does not count toward the loop's count.
fn fire(room: &Arc<Room>, job: ScheduledJob) {
    let Ok(_working) = room.working() else {
        return;
    };
    let Some(current) = room::schedules(&room.log)
        .into_iter()
        .find(|living| living.id == job.id && living.final_firing.is_none())
    else {
        return;
    };
    if !persona_exists(&room.log, &current.persona_id) {
        tombstone_if_live(room, &current.id);
        return;
    }
    if !scheduled_job_allowed(&room.log, &current.persona_id, &current.id) {
        return;
    }
    if current.kind == ScheduleKind::Loop && too_late(current.next_at, now_ms(), late_run_limit()) {
        finish(room, &current.id, Slot::TooLate);
        return;
    }
    let ran = if current.reminder == Some(true) {
        room.remind(&current).map(|()| true)
    } else {
        room.fire_job(&current)
    };
    match ran {
        Ok(true) => finish(room, &current.id, Slot::Ran),
        Ok(false) => finish(room, &current.id, Slot::Overlapped),
        Err(error) => retry(room, &current, &error),
    }
}

fn write_or_log(result: Result<(), String>) {
    if let Err(error) = result {
        eprintln!("a scheduled job could not be written: {error}");
    }
}

fn retry(room: &Room, job: &ScheduledJob, error: &str) {
    eprintln!(
        "a scheduled job for {} could not fire: {error}",
        job.persona_id
    );
    let _mutations = lock(&room.schedule_mutations);
    let Some(mut retry) = room::schedules(&room.log)
        .into_iter()
        .find(|living| living.id == job.id)
    else {
        return;
    };
    retry.next_at = now_ms().saturating_add(RETRY_WAIT);
    write_or_log(room::append_schedule(&room.log, &retry));
}

/// Removes an orphan only while it still exists. This uses the same guard as
/// cancellation so a concurrent cancel cannot be followed by a stale write.
fn tombstone_if_live(room: &Room, job_id: &str) {
    let _mutations = lock(&room.schedule_mutations);
    if room::schedules(&room.log)
        .into_iter()
        .any(|living| living.id == job_id)
    {
        write_or_log(room::tombstone_schedule(&room.log, job_id));
    }
}

/// Advances or ends the job that just fired, under the same guard as desk
/// edits. Re-folding here is essential: a cancellation or a quiet/source edit
/// may have landed while the firing was handed over.
///
/// A one-shot has fired its last, and so has a loop whose count runs out or
/// whose next firing would fall after its end: each is ended rather than
/// deleted, because its thread stays readable. A slot that was skipped, not
/// run, moves the loop to its next slot without spending its count, so only
/// its end date can end it; one skipped for being too late is kept on the
/// job as its latest skip.
fn finish(room: &Room, job_id: &str, slot: Slot) {
    let _mutations = lock(&room.schedule_mutations);
    let Some(mut next) = room::schedules(&room.log)
        .into_iter()
        .find(|living| living.id == job_id)
    else {
        return;
    };
    let now = now_ms();
    let ran = slot == Slot::Ran;
    if slot == Slot::TooLate {
        next.skipped = Some(SkippedRun {
            slot: next.next_at,
            at: now,
        });
    }
    let ends = if ran {
        fires_last(&next, now)
    } else {
        next_after_end(&next, now)
    };
    if ends && ran && next.reminder != Some(true) {
        // The last firing is on its thread: the job fires no more, and ends
        // when that firing does.
        next.final_firing = Some(now);
        // A firing so quick that its turn is already over has nothing left
        // to wait for.
        if room.job_at_rest(job_id) {
            next.ended_at = Some(now);
        }
    } else if ends {
        next.ended_at = Some(now);
    }
    if next.kind == ScheduleKind::Loop {
        next.next_at = next_after(&next, now);
        if ran {
            next.times = next.times.map(|times| times.saturating_sub(1));
        }
    }
    write_or_log(room::append_schedule(&room.log, &next));
}

/// Whether the job firing at `now` ends after this firing: a one-shot, or a
/// loop on its last count or whose next firing would fall after its end.
pub(super) fn fires_last(job: &ScheduledJob, now: i64) -> bool {
    match job.kind {
        ScheduleKind::Schedule => true,
        ScheduleKind::Loop => matches!(job.times, Some(0 | 1)) || next_after_end(job, now),
    }
}

/// Whether a loop's next slot after `now` would fall after its end date.
fn next_after_end(job: &ScheduledJob, now: i64) -> bool {
    job.until.is_some_and(|until| next_after(job, now) > until)
}

impl Room {
    /// Writes a job onto the room stream and wakes the clock: one the desk
    /// itself asked for, which runs on the operator's word.
    pub(crate) fn schedule_create(
        &self,
        persona_id: &str,
        brief: Brief,
        reminder: bool,
    ) -> Result<ScheduledJob, String> {
        self.schedule_create_with_source(
            persona_id,
            brief,
            reminder,
            !crate::wire::commands::from_voice(),
        )
    }

    /// Creates a job from one of the teammate's built-in tools. The tool is
    /// always scoped to its caller, and cannot choose operator provenance.
    pub(crate) fn schedule_create_agent(
        &self,
        persona_id: &str,
        brief: Brief,
        reminder: bool,
    ) -> Result<ScheduledJob, String> {
        self.schedule_create_with_source(persona_id, brief, reminder, false)
    }

    fn schedule_create_with_source(
        &self,
        persona_id: &str,
        brief: Brief,
        reminder: bool,
        operator_created: bool,
    ) -> Result<ScheduledJob, String> {
        let Brief {
            task,
            trigger,
            report,
            ends,
            without,
        } = brief;
        let _mutations = lock(&self.schedule_mutations);
        if !operator_created && !background_work_allowed(&self.log, persona_id) {
            return Err(
                "Background work is not granted for this teammate; ask the operator to enable it."
                    .to_string(),
            );
        }
        if room::roster(&self.log)
            .iter()
            .all(|persona| persona.id != persona_id)
        {
            return Err(format!("There is no teammate {persona_id}."));
        }
        let prompt = task.trim();
        if prompt.is_empty() || prompt.len() > MAX_PROMPT {
            return Err("prompt must be 1–8000 characters".to_string());
        }
        let goal = ends
            .goal
            .map(|goal| goal.trim().to_string())
            .filter(|goal| !goal.is_empty());
        if goal.as_ref().is_some_and(|goal| goal.len() > MAX_GOAL) {
            return Err("goal must be at most 500 characters".to_string());
        }
        let living = room::schedules(&self.log)
            .into_iter()
            .filter(|job| job.persona_id == persona_id)
            .count();
        if living >= MAX_JOBS {
            return Err(format!(
                "A teammate can have at most {MAX_JOBS} scheduled jobs"
            ));
        }
        let now = now_ms();
        let (kind, when, every, next_at, calendar) = match trigger {
            Trigger::Now => {
                return Err(
                    "A job runs at a time or every interval; a subagent runs now.".to_string(),
                );
            }
            Trigger::At(when) => {
                let wait = when.saturating_sub(now);
                if !(MIN_WAIT..=MAX_AHEAD).contains(&wait) {
                    return Err(
                        "schedule must be between 1 second and 30 days from now".to_string()
                    );
                }
                (ScheduleKind::Schedule, Some(when), None, when, None)
            }
            Trigger::Every(every) => {
                if !(MIN_LOOP..=MAX_LOOP).contains(&every) {
                    return Err("every must be a duration between 15s and 7d".to_string());
                }
                (
                    ScheduleKind::Loop,
                    None,
                    Some(every),
                    now.saturating_add(every),
                    None,
                )
            }
            Trigger::Calendar(calendar) => {
                calendar::check(&calendar)?;
                let first = calendar::next_slot(&calendar, now, &Local).ok_or_else(|| {
                    "This desk's clock has no next time for that calendar.".to_string()
                })?;
                (ScheduleKind::Loop, None, Some(DAY), first, Some(calendar))
            }
        };
        let quiet = match report {
            Report::EachFiring => false,
            Report::Significant => true,
            Report::Asker => {
                return Err("A scheduled job reports to the person, not to a turn.".to_string());
            }
        };
        if kind == ScheduleKind::Schedule && (ends.at.is_some() || ends.after.is_some()) {
            return Err("A one-shot fires once: only a loop has an end.".to_string());
        }
        if ends.after == Some(0) {
            return Err("times must be at least 1".to_string());
        }
        if ends.at.is_some_and(|until| until <= next_at) {
            return Err("until must be after the loop's first run".to_string());
        }
        let job = ScheduledJob {
            id: uuid::Uuid::new_v4().to_string(),
            persona_id: persona_id.to_string(),
            kind,
            when,
            every,
            calendar,
            prompt: prompt.to_string(),
            quiet: quiet.then_some(true),
            reminder: reminder.then_some(true),
            goal,
            until: ends.at,
            times: ends.after,
            without: (!without.is_empty()).then_some(without),
            operator_created,
            next_at,
            created_at: now,
            last_post: None,
            ended_at: None,
            final_firing: None,
            skipped: None,
        };
        room::append_schedule(&self.log, &job)?;
        self.schedule_changed.notify_one();
        Ok(job)
    }

    /// The jobs still to fire.
    pub(crate) fn schedule_list(&self) -> Vec<ScheduledJob> {
        room::schedules(&self.log)
    }

    /// The person, or the teammate in its main conversation, cancels a job:
    /// a living one ends with a final post saying where it stood, and an
    /// ended one is taken off the list. Its thread stays readable.
    pub(crate) fn schedule_cancel(&self, id: &str) -> Result<(), String> {
        let job = {
            let _mutations = lock(&self.schedule_mutations);
            let job = room::all_jobs(&self.log)
                .into_iter()
                .find(|job| job.id == id)
                .ok_or_else(|| format!("There is no scheduled job {id}."))?;
            if job.ended_at.is_some() {
                room::tombstone_schedule(&self.log, id)?;
                return Ok(());
            }
            self.end_job_record(job)?
        };
        self.schedule_changed.notify_one();
        self.job_ended(&job, None);
        Ok(())
    }

    /// The job, from its own thread, has met its goal: it ends, and what it
    /// said came of it is its final post.
    pub(crate) fn schedule_done(&self, id: &str, report: &str) -> Result<(), String> {
        let job = {
            let _mutations = lock(&self.schedule_mutations);
            let job = room::schedules(&self.log)
                .into_iter()
                .find(|job| job.id == id)
                .ok_or_else(|| format!("There is no scheduled job {id} still running."))?;
            self.end_job_record(job)?
        };
        self.schedule_changed.notify_one();
        self.job_ended(&job, Some(report));
        Ok(())
    }

    /// A job whose last firing is over ends: its record says so, and what
    /// gates on a job having ended turns away from it from here. Does nothing
    /// for a job that is not in its last firing.
    pub(super) fn end_final_firing(&self, job_id: &str) {
        let _mutations = lock(&self.schedule_mutations);
        let Some(mut job) = room::schedules(&self.log)
            .into_iter()
            .find(|job| job.id == job_id && job.final_firing.is_some())
        else {
            return;
        };
        job.ended_at = Some(now_ms());
        write_or_log(room::append_schedule(&self.log, &job));
        self.schedule_changed.notify_one();
    }

    /// Writes `job` as ended now. The caller holds the schedule guard.
    fn end_job_record(&self, mut job: ScheduledJob) -> Result<ScheduledJob, String> {
        job.ended_at = Some(now_ms());
        room::append_schedule(&self.log, &job)?;
        Ok(job)
    }

    pub(crate) fn schedule_set_quiet(&self, id: &str, quiet: bool) -> Result<(), String> {
        let _mutations = lock(&self.schedule_mutations);
        let mut job = room::schedules(&self.log)
            .into_iter()
            .find(|job| job.id == id)
            .ok_or_else(|| format!("There is no scheduled job {id}."))?;
        let quiet = quiet.then_some(true);
        if job.quiet == quiet {
            return Ok(());
        }
        job.quiet = quiet;
        room::append_schedule(&self.log, &job)?;
        Ok(())
    }

    /// A plain reminder: its words, posted to the chat as the teammate's,
    /// stamped with the job. Nothing is asked of any agent.
    fn remind(&self, job: &ScheduledJob) -> Result<(), String> {
        self.post(
            &job.persona_id,
            &ThreadId::dm(&job.persona_id),
            Post {
                scheduled: Some(run_of(job)),
                ..Post::words(&job.prompt)
            },
        )
    }

    /// Notes a job's post on its record, for the listing.
    pub(super) fn job_posted(&self, job_id: &str, text: &str) {
        let _mutations = lock(&self.schedule_mutations);
        let Some(mut job) = room::all_jobs(&self.log)
            .into_iter()
            .find(|job| job.id == job_id)
        else {
            return;
        };
        job.last_post = Some(crate::contract::JobPost {
            at: now_ms(),
            text: crate::driver::clip(text, POST_LINE_CHARS),
        });
        write_or_log(room::append_schedule(&self.log, &job));
    }
}

/// How much of a job's last post the listing keeps.
const POST_LINE_CHARS: usize = 160;

#[cfg(test)]
mod tests;
