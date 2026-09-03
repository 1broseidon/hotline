//! Durable wakes for teammates.
//!
//! Jobs live on the room stream. This module is the clock: it folds them,
//! sleeps until the nearest `nextAt`, and fires through
//! [`Room::prompt_scheduled`]. A missed tick while Toad was closed fires once
//! on reopen rather than catching up a pile of them — the teammate should do
//! the work now, not replay every interval it slept through.
//!
//! `parse_duration` and `parse_when` are the agent-facing sugar. The wire
//! takes milliseconds; the teammate tools `schedule` and `loop` speak these
//! strings and call this module.

use super::{Room, lock, now_ms};
use crate::contract::{ScheduleKind, ScheduledJob, ScheduledRun, SessionState};
use crate::log::Log;
use crate::room;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::Notify;

/// Shorter than this is a busy-loop dressed as a schedule.
const MIN_LOOP: i64 = 15_000;
/// Longer than a week is a calendar, not a loop.
const MAX_LOOP: i64 = 7 * 86_400_000;
/// Further than a month is a reminder the person will have forgotten why.
const MAX_AHEAD: i64 = 30 * 86_400_000;
/// A teammate that schedules itself into a crowd is usually stuck in a loop
/// of its own making.
const MAX_JOBS: usize = 20;
const MIN_WAIT: i64 = 1_000;
const MAX_PROMPT: usize = 8_000;
/// How long a failed fire waits before trying again.
const RETRY_WAIT: i64 = 60_000;
/// How long the clock will sleep before looking again, even when the next job
/// is further away. The task holds the room weakly, and this is how often it
/// notices the room is gone; a create notifies and does not wait.
const LOOK_AGAIN: Duration = Duration::from_secs(60);
/// Longer than this stops being a name and starts being the prompt again.
const SCHEDULE_NAME_MAX: usize = 48;

struct ScheduleRequest<'a> {
    kind: ScheduleKind,
    when: Option<i64>,
    every: Option<i64>,
    prompt: &'a str,
    quiet: bool,
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

fn scheduled_run_of(job: &ScheduledJob) -> ScheduledRun {
    ScheduledRun {
        job_id: job.id.clone(),
        kind: job.kind,
        name: schedule_name(&job.prompt),
        operator_created: job.operator_created,
        quiet: job.quiet.filter(|quiet| *quiet),
    }
}

/// Whether a scheduled job may start a turn now.
///
/// The operator's desk command records its provenance on the job, so that
/// deliberate automation remains independent of a teammate's autonomous
/// background-work grant. Every other job reads the live persona record here;
/// the check is intentionally repeated by the session just before dispatch so
/// a grant revoked while a start or queue operation is in progress cannot
/// become a new turn.
pub(crate) fn scheduled_job_allowed(log: &Log, persona_id: &str, job_id: &str) -> bool {
    let Some(job) = room::schedules(log)
        .into_iter()
        .find(|job| job.id == job_id && job.persona_id == persona_id)
    else {
        return false;
    };
    let run = scheduled_run_of(&job);
    scheduled_run_allowed(log, persona_id, &run)
}

/// Whether a queued scheduled run may dispatch now.
///
/// The scheduler's job lookup is not enough at this point: one-shot jobs are
/// tombstoned as soon as their prompt is queued. The run therefore carries the
/// trusted operator bit captured at fire time, while the teammate grant is
/// read live so revocation still stops a queued run after a restart or race.
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
                    fire(&room, job).await;
                }
                // A write that failed leaves the job due; wait a beat rather
                // than spinning on a disk that cannot take the tombstone.
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
                .map(|ms| Duration::from_millis(ms as u64))
                .unwrap_or(LOOK_AGAIN)
                .min(LOOK_AGAIN);
            drop(room);
            tokio::select! {
                () = notified => {}
                () = tokio::time::sleep(wait) => {}
            }
        }
    });
}

async fn fire(room: &Arc<Room>, job: ScheduledJob) {
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
    match room.info(&current.persona_id).state {
        SessionState::Idle | SessionState::Stopped | SessionState::Error => {
            if let Err(error) = room.start(&current.persona_id).await {
                retry(room, &current, &error);
                return;
            }
        }
        SessionState::Starting | SessionState::Ready | SessionState::Thinking => {}
    }
    if !scheduled_job_allowed(&room.log, &current.persona_id, &current.id) {
        return;
    }
    let run = scheduled_run_of(&current);
    if let Err(error) = room
        .prompt_scheduled(&current.persona_id, &current.prompt, run)
        .await
    {
        retry(room, &current, &error);
        return;
    }
    finish(room, &current.id);
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

/// Advances or consumes the job that just ran, under the same guard as desk
/// edits. Re-folding here is essential: a cancellation or a quiet/source edit
/// may have landed while the session was starting or answering.
fn finish(room: &Room, job_id: &str) {
    let _mutations = lock(&room.schedule_mutations);
    let Some(current) = room::schedules(&room.log)
        .into_iter()
        .find(|living| living.id == job_id)
    else {
        return;
    };
    match current.kind {
        ScheduleKind::Loop => {
            let mut next = current;
            let every = next.every.unwrap_or(MIN_LOOP);
            next.next_at = now_ms().saturating_add(every);
            write_or_log(room::append_schedule(&room.log, &next));
        }
        ScheduleKind::Schedule => {
            write_or_log(room::tombstone_schedule(&room.log, job_id));
        }
    }
}

impl Room {
    /// Writes a job onto the room stream and wakes the clock.
    pub(crate) fn schedule_create(
        &self,
        persona_id: &str,
        kind: ScheduleKind,
        when: Option<i64>,
        every: Option<i64>,
        prompt: &str,
        quiet: bool,
    ) -> Result<ScheduledJob, String> {
        self.schedule_create_with_source(
            persona_id,
            ScheduleRequest {
                kind,
                when,
                every,
                prompt,
                quiet,
            },
            true,
        )
    }

    /// Creates a job from one of the teammate's built-in tools. The tool is
    /// always scoped to its caller, and cannot choose operator provenance.
    pub(crate) fn schedule_create_agent(
        &self,
        persona_id: &str,
        kind: ScheduleKind,
        when: Option<i64>,
        every: Option<i64>,
        prompt: &str,
        quiet: bool,
    ) -> Result<ScheduledJob, String> {
        self.schedule_create_with_source(
            persona_id,
            ScheduleRequest {
                kind,
                when,
                every,
                prompt,
                quiet,
            },
            false,
        )
    }

    fn schedule_create_with_source(
        &self,
        persona_id: &str,
        request: ScheduleRequest<'_>,
        operator_created: bool,
    ) -> Result<ScheduledJob, String> {
        let ScheduleRequest {
            kind,
            when,
            every,
            prompt,
            quiet,
        } = request;
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
        let prompt = prompt.trim();
        if prompt.is_empty() || prompt.len() > MAX_PROMPT {
            return Err("prompt must be 1–8000 characters".to_string());
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
        let (when, every, next_at) = match kind {
            ScheduleKind::Schedule => {
                if every.is_some() {
                    return Err("A one-shot schedule cannot have every.".to_string());
                }
                let when = when.ok_or_else(|| {
                    "A one-shot schedule needs when, in milliseconds since epoch.".to_string()
                })?;
                let wait = when.saturating_sub(now);
                if !(MIN_WAIT..=MAX_AHEAD).contains(&wait) {
                    return Err(
                        "schedule must be between 1 second and 30 days from now".to_string()
                    );
                }
                (Some(when), None, when)
            }
            ScheduleKind::Loop => {
                if when.is_some() {
                    return Err("A loop cannot have when.".to_string());
                }
                let every =
                    every.ok_or_else(|| "A loop needs every, in milliseconds.".to_string())?;
                if !(MIN_LOOP..=MAX_LOOP).contains(&every) {
                    return Err("every must be a duration between 15s and 7d".to_string());
                }
                (None, Some(every), now.saturating_add(every))
            }
        };
        let job = ScheduledJob {
            id: uuid::Uuid::new_v4().to_string(),
            persona_id: persona_id.to_string(),
            kind,
            when,
            every,
            prompt: prompt.to_string(),
            quiet: quiet.then_some(true),
            operator_created,
            next_at,
            created_at: now,
        };
        room::append_schedule(&self.log, &job)?;
        self.schedule_changed.notify_one();
        Ok(job)
    }

    pub(crate) fn schedule_list(&self) -> Vec<ScheduledJob> {
        room::schedules(&self.log)
    }

    pub(crate) fn schedule_cancel(&self, id: &str) -> Result<(), String> {
        let _mutations = lock(&self.schedule_mutations);
        if room::schedules(&self.log).iter().all(|job| job.id != id) {
            return Err(format!("There is no scheduled job {id}."));
        }
        room::tombstone_schedule(&self.log, id)?;
        self.schedule_changed.notify_one();
        Ok(())
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
            "toad-core-schedule-grant-{}-{}",
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
            face: None,
            team: None,
            backend_id: "pi".to_string(),
            cwd: "/tmp/ada".to_string(),
            reach: None,
            model_id: None,
            mode_id: None,
            effort_id: None,
            harness_override: None,
            hop_notice: None,
            mcp_policy: McpPolicy {
                mode: PolicyMode::None,
                server_ids: Vec::new(),
            },
            background_work,
            web_search_policy: None,
            computer: None,
            subagents: None,
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
            operator_created: false,
            next_at: now + 60_000,
            created_at: now,
        };
        room::append_schedule(&log, &job).unwrap();
        let room = Room::new(log, Arc::new(NoKeys));

        // This is the deterministic equivalent of cancellation winning while
        // fire awaits start: retry receives the old captured job afterwards.
        room.schedule_cancel(&job.id).unwrap();
        retry(&room, &job, "simulated start failure");

        assert!(room.schedule_list().is_empty());
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
            operator_created: true,
            next_at: now + 60_000,
            created_at: now,
        };
        room::append_schedule(&log, &job).unwrap();
        let room = Room::new(log, Arc::new(NoKeys));

        room.schedule_set_quiet(&job.id, false).unwrap();
        retry(&room, &job, "simulated start failure");
        let retried = room.schedule_list();
        assert_eq!(retried.len(), 1);
        assert!(retried[0].operator_created);
        assert_eq!(retried[0].quiet, None);

        finish(&room, &job.id);
        let advanced = room.schedule_list();
        assert_eq!(advanced.len(), 1);
        assert!(advanced[0].operator_created);
        assert_eq!(advanced[0].quiet, None);
        assert!(advanced[0].next_at > now);
    }
}
