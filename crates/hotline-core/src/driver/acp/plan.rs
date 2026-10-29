//! An account's plan limits as its agent reports them. Claude Code's adapter
//! sends one window at a time with its usage update after a reply, under
//! `_meta["_claude/rateLimit"]` (the SDK's `rate_limit_event`): the window
//! (`five_hour`, `seven_day`, a model's own), how much of it is used, when it
//! resets and whether the account may use it. Each window is kept until the
//! same window is reported again, so the plan reads whole across replies.

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
}
