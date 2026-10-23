//! Durable wakes for teammates: the clock of their jobs.
//!
//! Jobs live on the room stream. This module is the clock: it folds them,
//! sleeps until the nearest `nextAt`, and fires each one. A plain reminder is
//! posted to the teammate's chat as it was written ([`Room::post`]) and does
//! no work; any other job's firing is a turn on the job's own thread
//! ([`super::job_thread`]), never the teammate's DM. A missed tick while
//! Hotline was closed fires once on reopen rather than catching up a pile of
//! them — the teammate should do the work now, not replay every interval it
//! slept through.
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
use crate::contract::{Ability, ScheduleEntry, ScheduleKind, ScheduledJob, ScheduledRun, ThreadId};
use crate::log::Log;
use crate::room;
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
/// How long the clock will sleep before looking again, even when the next job
/// is further away. The task holds the room weakly, and this is how often it
/// notices the room is gone; a create notifies and does not wait.
const LOOK_AGAIN: Duration = Duration::from_secs(60);
/// Longer than this stops being a name and starts being the prompt again.
const SCHEDULE_NAME_MAX: usize = 48;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// At once: a subagent.
    Now,
    /// Once, at this moment, milliseconds since the epoch.
    At(i64),
    /// Every this many milliseconds, until it ends.
    Every(i64),
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
/// number, or an RFC3339 time.
pub fn parse_when(value: &str, now: i64) -> Option<i64> {
    if let Some(relative) = parse_duration(value) {
        return Some(now.saturating_add(relative));
    }
    let trimmed = value.trim();
    if let Ok(ms) = trimmed.parse::<i64>()
        && ms > 1_000_000_000_000
    {
        return Some(ms);
    }
    parse_datetime(trimmed)
}

fn parse_datetime(value: &str) -> Option<i64> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(dt.timestamp_millis());
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis());
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S") {
        return Some(dt.and_utc().timestamp_millis());
    }
    None
}

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
        next_at: job.next_at,
        quiet: job.quiet,
        reminder: job.reminder,
        last_post: job.last_post,
        ended_at: job.ended_at,
        without: job.without,
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
                .filter(|job| job.next_at <= now && should_consider(&room.log, job))
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
                    .any(|job| job.next_at <= now_ms());
                drop(room);
                if still_due {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                continue;
            }
            let wait = jobs
                .iter()
                .filter(|job| should_consider(&room.log, job))
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

/// Fires one due job: a reminder is posted, any other job's firing is handed
/// to its thread. Neither waits for the work, so a loop counts its next
/// interval from when this firing started, and one still running when the
/// next is due is what its thread skips. A skipped slot is not a firing, so
/// it does not count toward the loop's count.
fn fire(room: &Arc<Room>, job: ScheduledJob) {
    let Ok(_working) = room.working() else {
        return;
    };
    let Some(current) = room::schedules(&room.log)
        .into_iter()
        .find(|living| living.id == job.id)
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
    let ran = if current.reminder == Some(true) {
        room.remind(&current).map(|()| true)
    } else {
        room.fire_job(&current)
    };
    match ran {
        Ok(ran) => finish(room, &current.id, ran),
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
/// `ran`, moves the loop to its next slot without spending its count, so only
/// its end date can end it.
fn finish(room: &Room, job_id: &str, ran: bool) {
    let _mutations = lock(&room.schedule_mutations);
    let Some(mut next) = room::schedules(&room.log)
        .into_iter()
        .find(|living| living.id == job_id)
    else {
        return;
    };
    let now = now_ms();
    let ends = if ran {
        fires_last(&next, now)
    } else {
        next_after_end(&next, now)
    };
    if ends {
        next.ended_at = Some(now);
    }
    if next.kind == ScheduleKind::Loop {
        next.next_at = now.saturating_add(next.every.unwrap_or(MIN_LOOP));
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
    let next_at = now.saturating_add(job.every.unwrap_or(MIN_LOOP));
    job.until.is_some_and(|until| next_at > until)
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
        let (kind, when, every, next_at) = match trigger {
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
                (ScheduleKind::Schedule, Some(when), None, when)
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
                )
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
mod tests {
    use super::*;
    use crate::contract::SkillPolicy;
    use crate::contract::{McpPolicy, Persona, PolicyMode};
    use std::collections::HashMap;

    struct NoKeys;

    impl crate::session::ProviderKeys for NoKeys {
        fn provider_auth(&self) -> HashMap<String, crate::session::ProviderAuth> {
            HashMap::new()
        }
    }

    fn grant_log(background_work: bool) -> Log {
        let root = std::env::temp_dir().join(format!(
            "hotline-core-schedule-grant-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let log = Log::open(root);
        let persona = Persona {
            node: None,
            id: "ada".to_string(),
            name: "Ada".to_string(),
            goal: String::new(),
            avatar: None,
            team: None,
            backend_id: "hotline".to_string(),
            cwd: "/tmp/ada".to_string(),
            reach: None,
            model_id: None,
            mode_id: None,
            effort_id: None,
            harness_override: None,
            account_id: None,
            hop_notice: None,
            mcp_policy: McpPolicy {
                mode: PolicyMode::None,
                server_ids: Vec::new(),
            },
            skill_policy: SkillPolicy::default(),
            background_work,
            allowed_senders: Vec::new(),
            web_search_policy: None,
            folders: None,
            computer: None,
            voice: None,
            session_checkpoints: Vec::new(),
            last_session_id: None,
            created_at: 1,
            updated_at: 1,
        };
        log.append(
            &crate::log::StreamId::Room,
            &crate::room::room_event("persona", serde_json::to_value(persona).unwrap()),
        )
        .unwrap();
        log
    }

    fn run(operator_created: bool) -> ScheduledRun {
        ScheduledRun {
            job_id: "job".to_string(),
            kind: ScheduleKind::Schedule,
            name: "check".to_string(),
            operator_created,
            quiet: None,
        }
    }

    #[test]
    fn parse_duration_reads_the_units_a_person_types() {
        assert_eq!(parse_duration("15s"), Some(15_000));
        assert_eq!(parse_duration("20m"), Some(20 * 60_000));
        assert_eq!(parse_duration("1.5h"), Some(5_400_000));
        assert_eq!(parse_duration("7d"), Some(7 * 86_400_000));
        assert_eq!(parse_duration("20 minutes"), Some(20 * 60_000));
        assert_eq!(parse_duration(" 20m "), Some(20 * 60_000));
        assert_eq!(parse_duration("0s"), None);
        assert_eq!(parse_duration("20"), None);
        assert_eq!(parse_duration("nope"), None);
        assert_eq!(parse_duration("-1m"), None);
    }

    #[test]
    fn parse_when_takes_a_duration_an_epoch_or_an_rfc3339_time() {
        let now = 1_700_000_000_000;
        assert_eq!(parse_when("20m", now), Some(now + 20 * 60_000));
        assert_eq!(parse_when("1700000001000", now), Some(1_700_000_001_000));
        assert_eq!(
            parse_when("2020-01-01T00:00:00Z", now),
            Some(1_577_836_800_000)
        );
        assert_eq!(parse_when("2020-01-01", now), Some(1_577_836_800_000));
        assert_eq!(parse_when("nope", now), None);
        assert_eq!(parse_when("1000", now), None);
    }

    #[test]
    fn a_job_name_is_the_prompts_first_breath() {
        assert_eq!(
            schedule_name("Check the Apple order\n\nThen report."),
            "Check the Apple order"
        );
        assert_eq!(schedule_name("\n   \nCheck the order"), "Check the order");
        assert_eq!(schedule_name("Runner mode:\nstep one"), "Runner mode");
        assert_eq!(schedule_name("Check the order — "), "Check the order");
        assert_eq!(schedule_name("   \n\t "), "scheduled work");
        let long = schedule_name(&"x".repeat(400));
        assert_eq!(long.chars().count(), SCHEDULE_NAME_MAX);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn scheduled_run_authority_reads_the_live_grant_and_trusted_source() {
        let off = grant_log(false);
        assert!(!scheduled_run_allowed(&off, "ada", &run(false)));
        assert!(scheduled_run_allowed(&off, "ada", &run(true)));

        let on = grant_log(true);
        assert!(scheduled_run_allowed(&on, "ada", &run(false)));
    }

    #[tokio::test]
    async fn retry_receives_a_cancelled_job() {
        let log = grant_log(true);
        let now = now_ms();
        let job = ScheduledJob {
            id: "cancel-during-start".to_string(),
            persona_id: "ada".to_string(),
            kind: ScheduleKind::Schedule,
            when: Some(now + 60_000),
            every: None,
            prompt: "retry this".to_string(),
            quiet: Some(true),
            reminder: None,
            goal: None,
            until: None,
            times: None,
            without: None,
            operator_created: false,
            next_at: now + 60_000,
            created_at: now,
            last_post: None,
            ended_at: None,
        };
        room::append_schedule(&log, &job).unwrap();
        let room = Room::new(log, Arc::new(NoKeys));

        // This is the deterministic equivalent of cancellation winning while
        // fire awaits start: retry receives the old captured job afterwards.
        room.schedule_cancel(&job.id).unwrap();
        retry(&room, &job, "simulated start failure");

        assert_eq!(room.schedule_list(), [] as [ScheduledJob; 0]);
    }

    #[tokio::test]
    async fn retry_and_finish_keep_the_latest_job_source_and_quiet_setting() {
        let log = grant_log(true);
        let now = now_ms();
        let job = ScheduledJob {
            id: "preserve-edits".to_string(),
            persona_id: "ada".to_string(),
            kind: ScheduleKind::Loop,
            when: None,
            every: Some(MIN_LOOP),
            prompt: "keep this loop".to_string(),
            quiet: Some(true),
            reminder: None,
            goal: None,
            until: None,
            times: None,
            without: None,
            operator_created: true,
            next_at: now + 60_000,
            created_at: now,
            last_post: None,
            ended_at: None,
        };
        room::append_schedule(&log, &job).unwrap();
        let room = Room::new(log, Arc::new(NoKeys));

        room.schedule_set_quiet(&job.id, false).unwrap();
        retry(&room, &job, "simulated start failure");
        let retried = room.schedule_list();
        assert_eq!(retried.len(), 1);
        assert!(retried[0].operator_created);
        assert_eq!(retried[0].quiet, None);

        finish(&room, &job.id, true);
        let advanced = room.schedule_list();
        assert_eq!(advanced.len(), 1);
        assert!(advanced[0].operator_created);
        assert_eq!(advanced[0].quiet, None);
        assert!(advanced[0].next_at > now);
    }

    #[tokio::test]
    async fn a_skipped_slot_moves_a_loop_on_without_spending_its_count() {
        let log = grant_log(true);
        let now = now_ms();
        let job = ScheduledJob {
            id: "skips".to_string(),
            persona_id: "ada".to_string(),
            kind: ScheduleKind::Loop,
            when: None,
            every: Some(MIN_LOOP),
            prompt: "post the time".to_string(),
            quiet: None,
            reminder: None,
            goal: None,
            until: None,
            times: Some(1),
            without: None,
            operator_created: true,
            next_at: now,
            created_at: now,
            last_post: None,
            ended_at: None,
        };
        room::append_schedule(&log, &job).unwrap();
        let room = Room::new(log, Arc::new(NoKeys));

        finish(&room, &job.id, false);
        let skipped = room.schedule_list();
        assert_eq!(skipped[0].times, Some(1));
        assert_eq!(skipped[0].ended_at, None);
        assert!(skipped[0].next_at > now);

        finish(&room, &job.id, true);
        assert!(room::all_jobs(&room.log)[0].ended_at.is_some());
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;

    #[tokio::test]
    async fn an_update_does_not_consume_or_reschedule_a_due_job() {
        let log = super::super::tests::scratch("update-keeps-jobs");
        super::super::tests::enrol(&log, &super::super::tests::persona("ada"));
        let room = Room::new(log.clone(), Arc::new(super::super::tests::DeskKeys));
        let held = room.prepare_restart().unwrap();
        let job = ScheduledJob {
            id: "due-during-update".into(),
            persona_id: "ada".into(),
            kind: ScheduleKind::Schedule,
            when: Some(now_ms()),
            every: None,
            prompt: "Resume after updating".into(),
            quiet: None,
            reminder: None,
            goal: None,
            until: None,
            times: None,
            without: None,
            operator_created: true,
            next_at: now_ms(),
            created_at: now_ms(),
            last_post: None,
            ended_at: None,
        };
        room::append_schedule(&log, &job).unwrap();
        let before = log.load(&crate::log::StreamId::Room);
        fire(&room, job);
        assert_eq!(log.load(&crate::log::StreamId::Room), before);
        assert_eq!(room::schedules(&log).len(), 1);
        drop(held);
    }
}
