//! `loop` at a time of day: what a teammate writes, what the room keeps, and
//! how `list_schedules` says it back.

use super::*;
use crate::contract::{Calendar, CalendarDay};
use chrono::{Local, TimeZone};

/// "Every weekday at 9" is one call, and the room keeps a calendar loop
/// whose first run is the next weekday 9:00 on this computer's clock.
#[tokio::test(flavor = "multi_thread")]
async fn loop_at_a_time_on_weekdays_lands_a_calendar_loop() {
    let room = room_with_a_conversation("loop-weekdays");
    let answered = through_rig(
        &tools(&room),
        LOOP,
        json!({ "at": "9:00", "days": "weekdays", "prompt": "read the overnight alerts" }),
    )
    .await;
    let body: Value = serde_json::from_str(&answered).unwrap();
    assert_eq!(body["kind"], "loop");
    assert_eq!(body["repeats"], "Weekdays at 09:00");

    let jobs = room.schedule_list();
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert_eq!(job.kind, ScheduleKind::Loop);
    assert_eq!(
        job.calendar,
        Some(Calendar {
            at: "09:00".to_string(),
            days: vec![
                CalendarDay::Mon,
                CalendarDay::Tue,
                CalendarDay::Wed,
                CalendarDay::Thu,
                CalendarDay::Fri,
            ],
        })
    );
    // What a build that knows only `every` reads: a loop once a day.
    assert_eq!(job.every, Some(86_400_000));
    let first = Local.timestamp_millis_opt(job.next_at).unwrap();
    assert_eq!(first.format("%H:%M").to_string(), "09:00");
    assert!(
        !matches!(
            chrono::Datelike::weekday(&first),
            chrono::Weekday::Sat | chrono::Weekday::Sun
        ),
        "{first}"
    );
    assert!(job.next_at > now_ms());
    assert!(job.next_at <= now_ms() + 4 * 86_400_000, "{first}");

    let listed = through_rig(&tools(&room), LIST_SCHEDULES, json!({})).await;
    let listed: Value = serde_json::from_str(&listed).unwrap();
    let row = &listed["jobs"][0];
    assert_eq!(row["repeats"], "Weekdays at 09:00");
    assert_eq!(row["at"], "09:00");
    assert_eq!(row["days"], json!(["mon", "tue", "wed", "thu", "fri"]));
    assert!(row.get("every").is_none(), "{row}");
    assert_eq!(row["next"], first.format("%a %-d %b %Y, %H:%M").to_string());
}

/// `days` is optional with `at` (daily), may be a list, and a strict
/// caller's nulls are what it left out.
#[tokio::test(flavor = "multi_thread")]
async fn loop_days_default_to_daily_and_take_a_list() {
    let room = room_with_a_conversation("loop-days");
    let daily = through_rig(
        &tools(&room),
        LOOP,
        json!({ "at": "07:30", "prompt": "water the plants", "every": null, "days": null }),
    )
    .await;
    let daily: Value = serde_json::from_str(&daily).unwrap();
    assert_eq!(daily["repeats"], "Daily at 07:30");
    let listed = through_rig(
        &tools(&room),
        LOOP,
        json!({ "at": "18:00", "days": ["fri", "mon", "wed"], "prompt": "send the digest" }),
    )
    .await;
    let listed: Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(listed["repeats"], "Mon, Wed and Fri at 18:00");
}

/// One of `at` and `every`, and `days` only with `at`; each refusal says
/// what to send instead, and nothing is kept.
#[tokio::test(flavor = "multi_thread")]
async fn loop_takes_at_or_every_and_says_so() {
    let room = room_with_a_conversation("loop-refusals");
    let call = |arguments: Value| {
        let tools = tools(&room);
        async move { tools.call(LOOP, &arguments).await.unwrap_err() }
    };
    let both = call(json!({ "at": "09:00", "every": "1d", "prompt": "x" })).await;
    assert!(both.contains("not both"), "{both}");
    let neither = call(json!({ "prompt": "x" })).await;
    assert!(neither.contains("loop needs `at`"), "{neither}");
    let stray = call(json!({ "every": "1h", "days": "weekdays", "prompt": "x" })).await;
    assert!(stray.contains("`days` goes with `at`"), "{stray}");
    let noon = call(json!({ "at": "12pm", "prompt": "x" })).await;
    assert!(noon.contains("24-hour"), "{noon}");
    let someday = call(json!({ "at": "09:00", "days": "someday", "prompt": "x" })).await;
    assert!(someday.contains("weekdays"), "{someday}");
    assert_eq!(room.schedule_list(), [] as [ScheduledJob; 0]);
}

/// The description a model reads says Hotline repeats the job and nothing
/// re-arms it, with the weekday example; `schedule` sends repeats to `loop`.
#[test]
fn the_loop_description_says_hotline_repeats_it() {
    let tools = listing(descriptors()).tools;
    let said = |name: &str| {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .and_then(|tool| tool.description.as_deref())
            .unwrap()
            .to_string()
    };
    let looping = said(LOOP);
    assert!(
        looping.contains("Hotline runs it again by itself each time until it ends: call this once, and nothing ever needs re-arming."),
        "{looping}"
    );
    assert!(
        looping.contains("give `at` \"09:00\" and `days` \"weekdays\""),
        "{looping}"
    );
    let once = said(SCHEDULE);
    assert!(
        once.contains(
            "For anything that repeats, such as every weekday at 9, use loop once instead"
        ),
        "{once}"
    );
    let schema = &tools
        .iter()
        .find(|tool| tool.name == LOOP)
        .unwrap()
        .input_schema;
    assert_eq!(schema["required"], json!(["prompt"]));
}
