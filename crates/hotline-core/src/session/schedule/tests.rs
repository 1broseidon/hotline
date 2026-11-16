use super::*;
use crate::contract::SkillPolicy;
use crate::contract::{McpPolicy, Persona, PolicyMode};
use chrono::{Datelike, FixedOffset, MappedLocalTime, Utc};
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
        bubble: None,
        backend_id: "hotline".to_string(),
        cwd: "/tmp/ada".to_string(),
        reach: None,
        model_id: None,
        mode_id: None,
        effort_id: None,
        config_choices: std::collections::BTreeMap::new(),
        effort_by_model: std::collections::BTreeMap::new(),
        harness_override: None,
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
        secrets: None,
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

/// New York's clock with no zone database, so no test hangs on the machine's
/// zone: five hours behind UTC, four from 2:00 on the second Sunday of March
/// to 2:00 on the first Sunday of November. In 2026 the clocks go forward on
/// 8 March and back on 1 November.
#[derive(Clone, Copy, Debug)]
pub(super) struct NewYork;

impl NewYork {
    fn standard() -> FixedOffset {
        FixedOffset::west_opt(5 * 3_600).expect("five hours is an offset")
    }

    fn daylight() -> FixedOffset {
        FixedOffset::west_opt(4 * 3_600).expect("four hours is an offset")
    }

    fn offset_at(utc: &NaiveDateTime) -> FixedOffset {
        let year = utc.year();
        let sunday = |month, nth| {
            NaiveDate::from_weekday_of_month_opt(year, month, chrono::Weekday::Sun, nth)
                .expect("every March and November has the Sundays asked for")
        };
        // 2:00 standard is 7:00 UTC; 2:00 daylight is 6:00 UTC.
        let forward = sunday(3, 2).and_hms_opt(7, 0, 0).expect("a time");
        let back = sunday(11, 1).and_hms_opt(6, 0, 0).expect("a time");
        if (forward..back).contains(utc) {
            Self::daylight()
        } else {
            Self::standard()
        }
    }
}

impl TimeZone for NewYork {
    type Offset = FixedOffset;

    fn from_offset(_: &FixedOffset) -> Self {
        NewYork
    }

    fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<FixedOffset> {
        self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
    }

    /// Daylight first: of two readings of one wall time, the one with the
    /// larger offset is the earlier instant.
    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> MappedLocalTime<FixedOffset> {
        let fits: Vec<FixedOffset> = [Self::daylight(), Self::standard()]
            .into_iter()
            .filter(|offset| Self::offset_at(&(*local - *offset)) == *offset)
            .collect();
        match fits.as_slice() {
            [] => MappedLocalTime::None,
            [one] => MappedLocalTime::Single(*one),
            [first, second, ..] => MappedLocalTime::Ambiguous(*first, *second),
        }
    }

    fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
        Self::offset_at(&utc.and_time(NaiveTime::MIN))
    }

    fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
        Self::offset_at(utc)
    }
}

/// The instant `at` UTC, written as `2026-03-08 07:00`.
pub(super) fn utc(at: &str) -> i64 {
    NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M")
        .expect("a test's time")
        .and_utc()
        .timestamp_millis()
}

#[test]
fn parse_when_takes_a_duration_an_epoch_or_a_date_and_time() {
    let now = 1_700_000_000_000;
    assert_eq!(parse_when_in("20m", now, &NewYork), Some(now + 20 * 60_000));
    assert_eq!(
        parse_when_in("1700000001000", now, &NewYork),
        Some(1_700_000_001_000)
    );
    assert_eq!(parse_when_in("nope", now, &NewYork), None);
    assert_eq!(parse_when_in("1000", now, &NewYork), None);
    assert_eq!(parse_when_in("2026-13-01", now, &NewYork), None);
}

/// A teammate is shown local time with no offset, so that is how a time it
/// writes without one is read. An offset it does write is honoured.
#[test]
fn a_time_without_a_zone_is_read_on_the_desks_clock() {
    let now = 1_700_000_000_000;
    let read = |value: &str| parse_when_in(value, now, &NewYork);
    // 9:00 in New York in November is 14:00 UTC; in July, 13:00 UTC.
    assert_eq!(read("2026-11-16T09:00:00"), Some(utc("2026-11-16 14:00")));
    assert_eq!(read("2026-11-16T09:00"), Some(utc("2026-11-16 14:00")));
    assert_eq!(read("2026-11-16 09:00"), Some(utc("2026-11-16 14:00")));
    assert_eq!(
        read("2026-11-16T09:00:00.000"),
        Some(utc("2026-11-16 14:00"))
    );
    assert_eq!(read("2026-07-16T09:00"), Some(utc("2026-07-16 13:00")));
    // A bare date is its local midnight.
    assert_eq!(read("2026-11-16"), Some(utc("2026-11-16 05:00")));
    // An offset, or Z, is that instant whatever the desk's clock says.
    assert_eq!(read("2026-11-16T09:00:00Z"), Some(utc("2026-11-16 09:00")));
    assert_eq!(
        read("2026-11-16T09:00:00+01:00"),
        Some(utc("2026-11-16 08:00"))
    );
    assert_eq!(
        read("2026-11-16T09:00+01:00"),
        Some(utc("2026-11-16 08:00"))
    );
    assert_eq!(read("2026-11-16T09:00Z"), Some(utc("2026-11-16 09:00")));
    // The same text on a desk in UTC is the same wall time there.
    assert_eq!(
        parse_when_in("2026-11-16T09:00", now, &Utc),
        Some(utc("2026-11-16 09:00"))
    );
}

/// On the morning the clocks go forward from 2:00 to 3:00, 2:30 never
/// happens: it is read as 3:00, when they changed. On the morning they go
/// back from 2:00 to 1:00, 1:30 happens twice: it is read as the first.
#[test]
fn a_wall_time_the_clocks_skip_or_repeat_is_one_instant() {
    let now = 1_700_000_000_000;
    let read = |value: &str| parse_when_in(value, now, &NewYork);
    // 3:00 daylight is 7:00 UTC.
    assert_eq!(read("2026-03-08T02:30"), Some(utc("2026-03-08 07:00")));
    assert_eq!(read("2026-03-08T02:00"), Some(utc("2026-03-08 07:00")));
    assert_eq!(read("2026-03-08T03:00"), Some(utc("2026-03-08 07:00")));
    assert_eq!(read("2026-03-08T01:59"), Some(utc("2026-03-08 06:59")));
    // The first 1:30 is daylight's, 5:30 UTC; the second would be 6:30.
    assert_eq!(read("2026-11-01T01:30"), Some(utc("2026-11-01 05:30")));
    assert_eq!(read("2026-11-01T02:00"), Some(utc("2026-11-01 07:00")));
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
        final_firing: None,
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
        final_firing: None,
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
        final_firing: None,
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

/// A restart's drain holds the room: a job due meanwhile is neither fired nor moved.
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
        final_firing: None,
    };
    room::append_schedule(&log, &job).unwrap();
    let before = log.load(&crate::log::StreamId::Room);
    fire(&room, job);
    assert_eq!(log.load(&crate::log::StreamId::Room), before);
    assert_eq!(room::schedules(&log).len(), 1);
    drop(held);
}
