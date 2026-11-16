//! A calendar loop's times: a time of day on the desk machine's own clock, on
//! chosen days of the week (`crate::contract::Calendar`).
//!
//! Each next run is worked out afresh from the calendar and the moment after
//! which it must fall, never by adding a day to the run before. That is what
//! keeps a loop on its time: a run that started late, a desk that was closed
//! and the clocks changing all leave the next run at 9:00 if 9:00 is what it
//! was asked for. Every function here takes the clock's zone, so a test can
//! hand it one with clock changes of its own; the room hands it the desk's.

use super::local_instant;
use crate::contract::{Calendar, CalendarDay};
use chrono::{DateTime, Datelike, Days, NaiveTime, TimeZone, Weekday};
use serde_json::Value;

/// Every day of the week, Monday first.
const EVERY_DAY: [CalendarDay; 7] = [
    CalendarDay::Mon,
    CalendarDay::Tue,
    CalendarDay::Wed,
    CalendarDay::Thu,
    CalendarDay::Fri,
    CalendarDay::Sat,
    CalendarDay::Sun,
];
const WEEKDAYS: [CalendarDay; 5] = [
    CalendarDay::Mon,
    CalendarDay::Tue,
    CalendarDay::Wed,
    CalendarDay::Thu,
    CalendarDay::Fri,
];
const WEEKEND: [CalendarDay; 2] = [CalendarDay::Sat, CalendarDay::Sun];

/// The longest a calendar ever waits for its next run, a week, and the day
/// before, whose time the clocks may have pushed past midnight.
const DAYS_LOOKED_AT: u64 = 9;

/// A time of day as `at` is written, `9:00` or `09:00` on a 24-hour clock.
pub(crate) fn time_of_day(at: &str) -> Option<NaiveTime> {
    let (hours, minutes) = at.trim().split_once(':')?;
    if hours.is_empty() || hours.len() > 2 || minutes.len() != 2 {
        return None;
    }
    let hours: u32 = hours.parse().ok()?;
    let minutes: u32 = minutes.parse().ok()?;
    NaiveTime::from_hms_opt(hours, minutes, 0)
}

/// `at` as a calendar keeps it, `09:00`, or why it is not a time of day.
pub(crate) fn parse_at(at: &str) -> Result<String, String> {
    time_of_day(at)
        .map(|time| time.format("%H:%M").to_string())
        .ok_or_else(|| format!("`{at}` is not a time of day like 09:00 or 17:30 (24-hour)."))
}

/// The days a loop runs, as a teammate writes them: `daily`, `weekdays`, or
/// the days themselves, `mon` to `sun`, as a list or in one string with
/// commas (`mon,wed,fri`). Monday first, each once.
pub(crate) fn parse_days(days: &Value) -> Result<Vec<CalendarDay>, String> {
    let refused = || {
        format!(
            "`days` is daily, weekdays, or the days themselves like [\"mon\", \"wed\", \"fri\"]; `{days}` is none of those."
        )
    };
    let words: Vec<String> = match days {
        Value::String(text) => text
            .split([',', ' '])
            .map(str::trim)
            .filter(|word| !word.is_empty())
            .map(str::to_ascii_lowercase)
            .collect(),
        Value::Array(items) => items
            .iter()
            .map(|item| item.as_str().map(|word| word.trim().to_ascii_lowercase()))
            .collect::<Option<_>>()
            .ok_or_else(refused)?,
        _ => return Err(refused()),
    };
    let mut chosen: Vec<CalendarDay> = Vec::new();
    for word in &words {
        chosen.extend_from_slice(match word.as_str() {
            "daily" | "every day" => &EVERY_DAY,
            "weekdays" => &WEEKDAYS,
            other => std::slice::from_ref(day_named(other).ok_or_else(refused)?),
        });
    }
    chosen.sort();
    chosen.dedup();
    if chosen.is_empty() {
        return Err(refused());
    }
    Ok(chosen)
}

/// Each day's names: as a list says it, as a teammate may write it, and
/// whole.
const NAMES: [(CalendarDay, &str, &str); 7] = [
    (CalendarDay::Mon, "mon", "Monday"),
    (CalendarDay::Tue, "tue", "Tuesday"),
    (CalendarDay::Wed, "wed", "Wednesday"),
    (CalendarDay::Thu, "thu", "Thursday"),
    (CalendarDay::Fri, "fri", "Friday"),
    (CalendarDay::Sat, "sat", "Saturday"),
    (CalendarDay::Sun, "sun", "Sunday"),
];

fn day_named(word: &str) -> Option<&'static CalendarDay> {
    NAMES
        .iter()
        .find(|(_, short, long)| word == *short || word.eq_ignore_ascii_case(long))
        .map(|(day, _, _)| day)
}

fn names_of(day: CalendarDay) -> (&'static str, &'static str) {
    NAMES
        .iter()
        .find(|(named, _, _)| *named == day)
        .map(|(_, short, long)| (*short, *long))
        .expect("every day is named")
}

/// Whether a calendar can be kept: a time of day, and at least one day.
pub(crate) fn check(calendar: &Calendar) -> Result<(), String> {
    parse_at(&calendar.at)?;
    if calendar.days.is_empty() {
        return Err("A calendar loop runs on at least one day.".to_string());
    }
    Ok(())
}

fn runs_on(calendar: &Calendar, day: Weekday) -> bool {
    let index = day.num_days_from_monday() as usize;
    EVERY_DAY
        .get(index)
        .is_some_and(|day| calendar.days.contains(day))
}

/// The calendar's first run strictly after `after`, milliseconds since the
/// epoch, on `zone`'s clock. A time the clocks skip that day runs at the
/// moment they changed, and one they pass twice runs the first time, once
/// ([`local_instant`]). None only for a calendar [`check`] refuses.
pub(crate) fn next_slot<Tz: TimeZone>(calendar: &Calendar, after: i64, zone: &Tz) -> Option<i64> {
    let at = time_of_day(&calendar.at)?;
    let today = DateTime::from_timestamp_millis(after)?
        .with_timezone(zone)
        .date_naive();
    let yesterday = today.checked_sub_days(Days::new(1))?;
    (0..DAYS_LOOKED_AT)
        .filter_map(|offset| yesterday.checked_add_days(Days::new(offset)))
        .filter(|date| runs_on(calendar, date.weekday()))
        .filter_map(|date| local_instant(zone, date.and_time(at)))
        .find(|slot| *slot > after)
}

/// The calendar as a person says it, on the 24-hour clock it was written
/// in: `Weekdays at 09:00`, `Daily at 07:30`, `Mon, Wed and Fri at 18:00`.
pub(crate) fn describe(calendar: &Calendar) -> String {
    format!("{} at {}", days_said(&calendar.days), calendar.at)
}

fn days_said(days: &[CalendarDay]) -> String {
    if days == EVERY_DAY {
        return "Daily".to_string();
    }
    if days == WEEKDAYS {
        return "Weekdays".to_string();
    }
    if days == WEEKEND {
        return "Weekends".to_string();
    }
    let short = |day: &CalendarDay| {
        let (short, _) = names_of(*day);
        let mut said = short.to_string();
        said[..1].make_ascii_uppercase();
        said
    };
    match days {
        [one] => format!("{}s", names_of(*one).1),
        [rest @ .., last] => {
            let rest: Vec<String> = rest.iter().map(short).collect();
            format!("{} and {}", rest.join(", "), short(last))
        }
        [] => String::new(),
    }
}
