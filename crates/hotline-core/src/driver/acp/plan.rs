//! An account's plan limits as its agent reports them. Claude Code's adapter
//! sends one window at a time with its usage update after a reply, under
//! `_meta["_claude/rateLimit"]` (the SDK's `rate_limit_event`): the window
//! (`five_hour`, `seven_day`, a model's own), how much of it is used, when it
//! resets and whether the account may use it.
//!
//! A reading can also be had without a turn: Claude Code answers `/usage`
//! itself, with no model call ([`usage_markdown`]), and Codex writes its
//! limits into its own session logs ([`codex_log`]). The room keeps one
//! reading per account and merges each new one into it ([`merged`]).

pub(crate) mod codex_log;
pub(crate) mod usage_markdown;

use crate::contract::{PlanStatus, PlanUsage, PlanWindow};
use serde_json::{Map, Value};

/// The meta key Claude Code's adapter reports its plan under.
const CLAUDE_RATE_LIMIT: &str = "_claude/rateLimit";

/// The window a usage update's meta reports, if it carries one.
pub(super) fn reported_window(meta: Option<&Map<String, Value>>) -> Option<PlanWindow> {
    let info = meta?.get(CLAUDE_RATE_LIMIT)?;
    let status = match info.get("status")?.as_str()? {
        "allowed" => PlanStatus::Allowed,
        "allowed_warning" => PlanStatus::Warning,
        "rejected" => PlanStatus::Limited,
        _ => return None,
    };
    let kind = info
        .get("rateLimitType")
        .and_then(Value::as_str)
        .unwrap_or("five_hour")
        .to_string();
    // The service reports utilization as a fraction; read a figure over 1 as
    // a percentage already.
    let used_percent = info
        .get("utilization")
        .and_then(Value::as_f64)
        .map(|used| if used <= 1.0 { used * 100.0 } else { used })
        .map(|used| used.clamp(0.0, 100.0));
    // Unix seconds, as the SDK sends it.
    let resets_at = info
        .get("resetsAt")
        .and_then(Value::as_f64)
        .map(|seconds| (seconds * 1000.0) as i64);
    Some(PlanWindow {
        kind,
        used_percent,
        resets_at,
        status,
    })
}

/// The plan with a newly reported window in its place, the others kept.
pub(super) fn with_window(was: Option<PlanUsage>, window: PlanWindow, at: i64) -> PlanUsage {
    let mut windows = was.map(|plan| plan.windows).unwrap_or_default();
    match windows.iter_mut().find(|one| one.kind == window.kind) {
        Some(one) => *one = window,
        None => windows.push(window),
    }
    windows.sort_by_key(|one| order(&one.kind));
    PlanUsage { windows, at }
}

/// An account's reading with a newer one merged in, window by window: a
/// window the new reading has replaces the one kept when the new reading is
/// the later, and a window only one of them has is kept either way. A later
/// reading of the same window in the same period never relaxes its status,
/// because use only grows until the window resets: a reading worked out from
/// a percentage does not undo a warning the agent itself gave.
pub(crate) fn merged(was: Option<&PlanUsage>, reading: PlanUsage) -> PlanUsage {
    let Some(was) = was else {
        return reading;
    };
    let newer = reading.at >= was.at;
    let at = was.at.max(reading.at);
    let mut plan = was.clone();
    for mut window in reading.windows {
        match plan.windows.iter().find(|one| one.kind == window.kind) {
            Some(_) if !newer => continue,
            Some(kept)
                if same_period(kept, &window)
                    && severity(kept.status) > severity(window.status) =>
            {
                window.status = kept.status;
            }
            _ => {}
        }
        plan = with_window(Some(plan), window, at);
    }
    plan.at = at;
    plan
}

/// Whether two readings of a window are of the same period: they reset
/// together. `/usage` says the reset to the minute, the per-reply report to
/// the second.
fn same_period(a: &PlanWindow, b: &PlanWindow) -> bool {
    match (a.resets_at, b.resets_at) {
        (Some(a), Some(b)) => (a - b).abs() <= 90_000,
        _ => false,
    }
}

fn severity(status: PlanStatus) -> u8 {
    match status {
        PlanStatus::Allowed => 0,
        PlanStatus::Warning => 1,
        PlanStatus::Limited => 2,
    }
}

/// How a window stands when the agent gave only how much of it is used:
/// limited once it is all used, near the limit from 80%.
pub(crate) fn status_of(used_percent: f64) -> PlanStatus {
    if used_percent >= 100.0 {
        PlanStatus::Limited
    } else if used_percent >= 80.0 {
        PlanStatus::Warning
    } else {
        PlanStatus::Allowed
    }
}

/// The 5-hour window first, then the weekly one, then a model's own.
fn order(kind: &str) -> u8 {
    match kind {
        "five_hour" => 0,
        "seven_day" => 1,
        "seven_day_opus" => 2,
        "seven_day_sonnet" => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta(info: Value) -> Map<String, Value> {
        json!({ CLAUDE_RATE_LIMIT: info })
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn a_reported_window_reads_its_use_as_a_percentage_and_its_reset_in_milliseconds() {
        let window = reported_window(Some(&meta(json!({
            "status": "allowed_warning",
            "rateLimitType": "seven_day",
            "utilization": 0.82,
            "resetsAt": 1_793_774_976
        }))))
        .unwrap();
        assert_eq!(window.kind, "seven_day");
        assert_eq!(window.status, PlanStatus::Warning);
        assert!((window.used_percent.unwrap() - 82.0).abs() < 1e-9);
        assert_eq!(window.resets_at, Some(1_793_774_976_000));
    }

    #[test]
    fn an_update_without_claudes_meta_or_with_an_unknown_status_reports_nothing() {
        assert_eq!(reported_window(None), None);
        assert_eq!(reported_window(Some(&Map::new())), None);
        assert_eq!(
            reported_window(Some(&meta(json!({ "status": "maybe" })))),
            None
        );
    }

    #[test]
    fn each_window_is_kept_until_it_is_reported_again_and_they_read_shortest_first() {
        let window = |kind: &str, used: f64| PlanWindow {
            kind: kind.into(),
            used_percent: Some(used),
            resets_at: None,
            status: PlanStatus::Allowed,
        };
        let plan = with_window(None, window("seven_day", 40.0), 1);
        let plan = with_window(Some(plan), window("five_hour", 10.0), 2);
        let plan = with_window(Some(plan), window("seven_day", 45.0), 3);
        assert_eq!(
            plan.windows
                .iter()
                .map(|one| (one.kind.as_str(), one.used_percent))
                .collect::<Vec<_>>(),
            [("five_hour", Some(10.0)), ("seven_day", Some(45.0))]
        );
        assert_eq!(plan.at, 3);
    }
    #[test]
    fn a_later_reading_replaces_its_windows_and_an_earlier_one_only_adds_what_is_missing() {
        let window = |kind: &str, used: f64| PlanWindow {
            kind: kind.into(),
            used_percent: Some(used),
            resets_at: None,
            status: status_of(used),
        };
        let kept = PlanUsage {
            windows: vec![window("five_hour", 30.0), window("seven_day", 50.0)],
            at: 10,
        };
        let later = PlanUsage {
            windows: vec![window("five_hour", 35.0)],
            at: 20,
        };
        let plan = merged(Some(&kept), later);
        assert_eq!(plan.at, 20);
        assert_eq!(plan.windows[0].used_percent, Some(35.0));
        assert_eq!(plan.windows[1].used_percent, Some(50.0));
        let earlier = PlanUsage {
            windows: vec![window("five_hour", 5.0), window("seven_day_opus", 12.0)],
            at: 5,
        };
        let plan = merged(Some(&plan), earlier);
        assert_eq!(plan.at, 20);
        assert_eq!(
            plan.windows
                .iter()
                .map(|one| (one.kind.as_str(), one.used_percent))
                .collect::<Vec<_>>(),
            [
                ("five_hour", Some(35.0)),
                ("seven_day", Some(50.0)),
                ("seven_day_opus", Some(12.0))
            ]
        );
    }

    #[test]
    fn a_warning_stands_through_a_later_reading_of_the_same_period_and_not_past_its_reset() {
        let warned = PlanUsage {
            windows: vec![PlanWindow {
                kind: "seven_day".into(),
                used_percent: Some(76.0),
                resets_at: Some(1_793_774_976_000),
                status: PlanStatus::Warning,
            }],
            at: 10,
        };
        let read = |resets_at: i64| PlanUsage {
            windows: vec![PlanWindow {
                kind: "seven_day".into(),
                used_percent: Some(77.0),
                resets_at: Some(resets_at),
                status: status_of(77.0),
            }],
            at: 20,
        };
        // The same reset, to the minute.
        let plan = merged(Some(&warned), read(1_793_774_940_000));
        assert_eq!(plan.windows[0].status, PlanStatus::Warning);
        assert_eq!(plan.windows[0].used_percent, Some(77.0));
        // A week on, a new period.
        let plan = merged(Some(&warned), read(1_794_379_740_000));
        assert_eq!(plan.windows[0].status, PlanStatus::Allowed);
    }

    #[test]
    fn a_window_is_near_its_limit_from_80_percent_and_limited_when_all_used() {
        assert_eq!(status_of(79.9), PlanStatus::Allowed);
        assert_eq!(status_of(80.0), PlanStatus::Warning);
        assert_eq!(status_of(100.0), PlanStatus::Limited);
    }
}
