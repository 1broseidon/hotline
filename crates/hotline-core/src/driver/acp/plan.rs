//! An account's plan limits: how much of each window (the 5-hour one, the
//! week, a model's own) its login has used, and when each resets.
//!
//! A plan is the login's, not a teammate's, so it is read out of band, the
//! way each harness's own tools read it, with no teammate's session in the
//! way and no model asked:
//!
//! - Claude Code: the `claude` CLI in stream-JSON mode, asked `initialize`
//!   and `get_usage` and ended ([`claude_cli`]).
//! - Codex: the last reading in Codex's own session logs ([`codex_log`]),
//!   which is also read after every turn, for nothing.
//!
//! Claude Code's adapter also reports one window at a time after a reply,
//! under `_meta["_claude/rateLimit"]` (the SDK's `rate_limit_event`):
//! the window, how much of it is used, when it resets and whether the
//! account may use it. The room keeps one reading per login and merges each
//! new one into it ([`merged`]).

mod child;
pub(crate) mod claude_cli;
pub(crate) mod codex_log;

use super::registry;
use crate::contract::{PlanStatus, PlanUsage, PlanWindow};
use hotline_support::paths;
use serde_json::{Map, Value};
use std::path::Path;
use std::time::Duration;

/// The backend id of Claude Code.
pub(crate) const CLAUDE_CODE: &str = "claude-acp";
/// The backend id of Codex.
pub(crate) const CODEX: &str = "codex-acp";

pub(crate) const FIVE_HOUR_MINUTES: i64 = 5 * 60;
pub(crate) const WEEK_MINUTES: i64 = 7 * 24 * 60;

/// How long a CLI started to read a plan has to answer before it is killed.
const READ_BOUND: Duration = Duration::from_secs(15);

/// Whether the desk reads this backend's plan out of band.
pub(crate) fn readable(backend_id: &str) -> bool {
    matches!(backend_id, CLAUDE_CODE | CODEX)
}

/// The plan of the login a backend runs on, read now and out of band, with
/// the environment the desk gives that backend's teammates. `None` when the
/// backend's plan is not one the desk reads, the login has no plan (an API
/// key), or it could not be read.
pub(crate) async fn read(root: &Path, backend_id: &str) -> Option<PlanUsage> {
    if !readable(backend_id) {
        return None;
    }
    let env = launch_env(root, backend_id);
    let scratch = paths::plan_read_dir(root, backend_id);
    let now = crate::session::now_ms();
    match backend_id {
        CLAUDE_CODE => {
            let binary = registry::which("claude")?;
            claude_cli::read(&binary, &env, &scratch, READ_BOUND, now).await
        }
        CODEX => read_codex_log(&env, now).await,
        _ => None,
    }
}

/// The plan as the backend's own files say it after a turn, for nothing:
/// Codex's session logs. Every other backend is read only out of band.
pub(crate) async fn read_after_turn(root: &Path, backend_id: &str) -> Option<PlanUsage> {
    if backend_id != CODEX {
        return None;
    }
    read_codex_log(&launch_env(root, backend_id), crate::session::now_ms()).await
}

async fn read_codex_log(env: &[(String, String)], now: i64) -> Option<PlanUsage> {
    let home = codex_log::codex_home(env)?;
    tokio::task::spawn_blocking(move || codex_log::read(&home, now))
        .await
        .ok()
        .flatten()
}

/// The environment the backend's catalogue entry adds to its teammates'.
fn launch_env(root: &Path, backend_id: &str) -> Vec<(String, String)> {
    registry::launch(root, backend_id)
        .map(|launch| launch.env)
        .unwrap_or_default()
}

/// A plan's name as its service gives it, as a person reads it: `max` and
/// "Claude Max" are "Max", `team_premium` is "Team Premium".
pub(crate) fn plan_name(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let raw = raw
        .strip_prefix("Claude ")
        .or_else(|| raw.strip_prefix("claude_"))
        .unwrap_or(raw);
    let words: Vec<String> = raw
        .split(['_', '-', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            if word.chars().any(char::is_uppercase) {
                return word.to_string();
            }
            let mut letters = word.chars();
            letters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(letters).collect()
            })
        })
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// Codex's plan type as the plan is sold: `pro` → "ChatGPT Pro".
pub(crate) fn codex_plan(plan_type: &str) -> Option<String> {
    let name = plan_name(plan_type)?;
    Some(if name.starts_with("ChatGPT") {
        name
    } else {
        format!("ChatGPT {name}")
    })
}

/// A window's kind by its length: the 5-hour one, the week, a month, or
/// its length itself.
pub(crate) fn kind_of_minutes(minutes: i64) -> String {
    match minutes {
        FIVE_HOUR_MINUTES => "five_hour".to_string(),
        WEEK_MINUTES => "seven_day".to_string(),
        minutes if minutes >= 28 * 24 * 60 => "monthly".to_string(),
        minutes if minutes % 60 == 0 => format!("{}h", minutes / 60),
        minutes => format!("{minutes}m"),
    }
}

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
    let window_minutes = match kind.as_str() {
        "five_hour" => Some(FIVE_HOUR_MINUTES),
        kind if kind.starts_with("seven_day") => Some(WEEK_MINUTES),
        _ => None,
    };
    Some(PlanWindow {
        kind,
        used_percent,
        resets_at,
        window_minutes,
        label: None,
        status,
    })
}

/// The plan with a newly reported window in its place, the others kept.
pub(super) fn with_window(was: Option<PlanUsage>, window: PlanWindow, at: i64) -> PlanUsage {
    let mut plan = was.unwrap_or(PlanUsage {
        windows: Vec::new(),
        at,
        plan: None,
        banked: None,
        banked_expires_at: None,
    });
    match plan.windows.iter_mut().find(|one| one.kind == window.kind) {
        Some(one) => *one = window,
        None => plan.windows.push(window),
    }
    plan.windows.sort_by_key(|one| order(&one.kind));
    plan.at = at;
    plan
}

/// An account's reading with a newer one merged in, window by window: a
/// window the new reading has replaces the one kept when the new reading is
/// the later, and a window only one of them has is kept either way. A later
/// reading of the same window in the same period never relaxes its status,
/// because use only grows until the window resets: a reading worked out from
/// a percentage does not undo a warning the agent itself gave. What a window
/// or the plan says only in one reading (its length, its label, the plan's
/// name, banked resets) is kept from whichever said it, the later first.
pub(crate) fn merged(was: Option<&PlanUsage>, reading: PlanUsage) -> PlanUsage {
    let Some(was) = was else {
        return reading;
    };
    let newer = reading.at >= was.at;
    let at = was.at.max(reading.at);
    let mut plan = was.clone();
    let (name, banked, banked_expires_at) =
        (reading.plan, reading.banked, reading.banked_expires_at);
    if newer {
        plan.plan = name.or(plan.plan);
        if banked.is_some() {
            plan.banked = banked;
            plan.banked_expires_at = banked_expires_at;
        }
    } else {
        plan.plan = plan.plan.or(name);
        if plan.banked.is_none() {
            plan.banked = banked;
            plan.banked_expires_at = banked_expires_at;
        }
    }
    for mut window in reading.windows {
        match plan.windows.iter().find(|one| one.kind == window.kind) {
            Some(_) if !newer => continue,
            Some(kept) => {
                if same_period(kept, &window) && severity(kept.status) > severity(window.status) {
                    window.status = kept.status;
                }
                window.window_minutes = window.window_minutes.or(kept.window_minutes);
                window.label = window.label.take().or_else(|| kept.label.clone());
            }
            None => {}
        }
        plan = with_window(Some(plan), window, at);
    }
    plan.at = at;
    plan
}

/// Whether two readings of a window are of the same period: they reset
/// together, to within a minute and a half, as two ways of reading it may
/// round it differently.
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

/// The 5-hour window first, then the weekly one, the monthly one, then a
/// model's own.
pub(crate) fn order(kind: &str) -> u8 {
    match kind {
        "five_hour" => 0,
        "seven_day" => 1,
        "monthly" => 2,
        "seven_day_opus" => 3,
        "seven_day_sonnet" => 4,
        _ => 5,
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

    fn window(kind: &str, used: f64) -> PlanWindow {
        PlanWindow {
            kind: kind.into(),
            used_percent: Some(used),
            resets_at: None,
            window_minutes: None,
            label: None,
            status: status_of(used),
        }
    }

    fn usage(windows: Vec<PlanWindow>, at: i64) -> PlanUsage {
        PlanUsage {
            windows,
            at,
            plan: None,
            banked: None,
            banked_expires_at: None,
        }
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
        assert_eq!(window.window_minutes, Some(WEEK_MINUTES));
        let five_hour = reported_window(Some(&meta(json!({ "status": "allowed" })))).unwrap();
        assert_eq!(five_hour.kind, "five_hour");
        assert_eq!(five_hour.window_minutes, Some(FIVE_HOUR_MINUTES));
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
        let kept = usage(
            vec![window("five_hour", 30.0), window("seven_day", 50.0)],
            10,
        );
        let later = usage(vec![window("five_hour", 35.0)], 20);
        let plan = merged(Some(&kept), later);
        assert_eq!(plan.at, 20);
        assert_eq!(plan.windows[0].used_percent, Some(35.0));
        assert_eq!(plan.windows[1].used_percent, Some(50.0));
        let earlier = usage(
            vec![window("five_hour", 5.0), window("seven_day_opus", 12.0)],
            5,
        );
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
    fn the_plans_name_a_windows_length_and_label_and_banked_resets_outlive_a_reading_without_them()
    {
        let mut scoped = window("seven_day_fable", 20.0);
        scoped.label = Some("Weekly · Fable".into());
        scoped.window_minutes = Some(WEEK_MINUTES);
        let mut out_of_band = usage(vec![window("five_hour", 30.0), scoped], 10);
        out_of_band.plan = Some("Max".into());
        out_of_band.banked = Some(1);
        out_of_band.banked_expires_at = Some(99);
        // A report after a reply says neither the plan nor the label.
        let reply = usage(vec![window("seven_day_fable", 25.0)], 20);
        let plan = merged(Some(&out_of_band), reply);
        assert_eq!(plan.plan.as_deref(), Some("Max"));
        assert_eq!((plan.banked, plan.banked_expires_at), (Some(1), Some(99)));
        let fable = plan
            .windows
            .iter()
            .find(|one| one.kind == "seven_day_fable")
            .unwrap();
        assert_eq!(fable.used_percent, Some(25.0));
        assert_eq!(fable.label.as_deref(), Some("Weekly · Fable"));
        assert_eq!(fable.window_minutes, Some(WEEK_MINUTES));
        // A later reading that names the plan names it anew.
        let mut renamed = usage(Vec::new(), 30);
        renamed.plan = Some("Pro".into());
        assert_eq!(merged(Some(&plan), renamed).plan.as_deref(), Some("Pro"));
    }

    #[test]
    fn a_warning_stands_through_a_later_reading_of_the_same_period_and_not_past_its_reset() {
        let mut warned = window("seven_day", 76.0);
        warned.resets_at = Some(1_793_774_976_000);
        warned.status = PlanStatus::Warning;
        let warned = usage(vec![warned], 10);
        let read = |resets_at: i64| {
            let mut later = window("seven_day", 77.0);
            later.resets_at = Some(resets_at);
            usage(vec![later], 20)
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

    #[test]
    fn a_plan_reads_as_it_is_sold_and_a_window_by_its_length() {
        assert_eq!(plan_name("max").as_deref(), Some("Max"));
        assert_eq!(plan_name("Claude Max").as_deref(), Some("Max"));
        assert_eq!(plan_name("team_premium").as_deref(), Some("Team Premium"));
        assert_eq!(plan_name("  ").as_deref(), None);
        assert_eq!(codex_plan("pro").as_deref(), Some("ChatGPT Pro"));
        assert_eq!(kind_of_minutes(300), "five_hour");
        assert_eq!(kind_of_minutes(10_080), "seven_day");
        assert_eq!(kind_of_minutes(43_200), "monthly");
        assert_eq!(kind_of_minutes(120), "2h");
        assert_eq!(kind_of_minutes(90), "90m");
    }
}
