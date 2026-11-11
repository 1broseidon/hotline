//! An account's plan limits: how much of each window (the 5-hour one, the
//! week, a model's own) its login has used, and when each resets; and, from
//! the same process, the harness's models with their effort levels
//! ([`super::catalog`]).
//!
//! A plan is the login's, not a teammate's, so it is read out of band, the
//! way each harness's own tools read it, with no teammate's session in the
//! way and no model asked:
//!
//! - Claude Code: the `claude` CLI in stream-JSON mode, asked `initialize`
//!   (which lists the models) and `get_usage` and ended ([`claude_cli`]).
//! - Codex: `codex app-server`, asked `account/rateLimits/read` and
//!   `model/list` and ended ([`codex_server`]); when it cannot be reached,
//!   the last reading in Codex's own session logs ([`codex_log`]), which is
//!   also read after every turn, for nothing.
//! - Grok Build: xAI's billing endpoint, with the Grok CLI's sign-in
//!   ([`grok_billing`]); its models from the list the CLI keeps on disk
//!   ([`grok_models`]).
//!
//! Claude Code's adapter also reports one window at a time after a reply,
//! under `_meta["_claude/rateLimit"]` (the SDK's `rate_limit_event`):
//! the window, how much of it is used, when it resets and whether the
//! account may use it. The room keeps one reading per login and merges each
//! new one into it ([`merged`]): a read of every window the account has
//! replaces the windows kept, a report of one replaces that one (keeping the
//! figure when the report has none and the period is the same), and a
//! window whose reset has passed holds no figure ([`as_of`]).

mod child;
pub(crate) mod claude_cli;
pub(crate) mod codex_log;
pub(crate) mod codex_server;
pub(crate) mod grok_billing;
pub(crate) mod grok_models;

use super::catalog::Catalog;
use super::registry;
use crate::contract::{PlanStatus, PlanUsage, PlanWindow};
use hotline_support::paths;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The backend id of Claude Code.
pub(crate) const CLAUDE_CODE: &str = "claude-acp";
/// The backend id of Codex.
pub(crate) const CODEX: &str = "codex-acp";
/// The backend id of Grok Build.
pub(crate) const GROK_BUILD: &str = "grok-build";

pub(crate) const FIVE_HOUR_MINUTES: i64 = 5 * 60;
pub(crate) const WEEK_MINUTES: i64 = 7 * 24 * 60;
pub(crate) const MONTH_MINUTES: i64 = 30 * 24 * 60;

/// How long a CLI started to read a plan has to answer before it is killed.
const READ_BOUND: Duration = Duration::from_secs(15);

/// Whether the desk reads this backend's plan out of band.
pub(crate) fn readable(backend_id: &str) -> bool {
    matches!(backend_id, CLAUDE_CODE | CODEX | GROK_BUILD)
}

/// Whether the same read lists the backend's models with their levels.
pub(crate) fn catalogued(backend_id: &str) -> bool {
    matches!(backend_id, CLAUDE_CODE | CODEX | GROK_BUILD)
}

/// What one out-of-band read of a login gave: its plan, and the harness's
/// models with their levels. Either may be missing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoginReading {
    pub plan: Option<PlanUsage>,
    pub catalog: Option<Catalog>,
}

/// The plan of the login a backend runs on, and its harness's models, read
/// now and out of band, with the environment the desk gives that backend's
/// teammates: one process for both. The plan is `None` when the backend's
/// plan is not one the desk reads, the login has no plan (an API key), or
/// it could not be read.
pub(crate) async fn read(root: &Path, backend_id: &str) -> LoginReading {
    if !readable(backend_id) {
        return LoginReading::default();
    }
    let env = launch_env(root, backend_id);
    let scratch = paths::plan_read_dir(root, backend_id);
    let now = crate::session::now_ms();
    match backend_id {
        CLAUDE_CODE => match registry::which("claude") {
            Some(binary) => claude_cli::read(&binary, &env, &scratch, READ_BOUND, now).await,
            None => LoginReading::default(),
        },
        CODEX => read_codex(registry::which("codex"), &env, &scratch, READ_BOUND, now).await,
        GROK_BUILD => {
            let lookup = |name: &str| variable(&env, name);
            LoginReading {
                plan: grok_billing::read(&lookup, now).await,
                catalog: grok_models::read(&lookup),
            }
        }
        _ => LoginReading::default(),
    }
}

/// The plan a harness names as its session opens, with no windows: the
/// plan's name beside the agent in the menu. Google Antigravity says its
/// tier on every session it opens (`_meta.antigravity.tier.name`, "Google
/// AI Pro"); its quota is read nowhere yet.
pub(crate) fn named_plan(meta: Option<&Map<String, Value>>, now: i64) -> Option<PlanUsage> {
    let name = meta?
        .get("antigravity")?
        .get("tier")?
        .get("name")?
        .as_str()?
        .trim();
    (!name.is_empty()).then(|| PlanUsage {
        windows: Vec::new(),
        at: now,
        plan: Some(name.to_string()),
        banked: None,
        banked_expires_at: None,
    })
}

/// The plan as the backend's own files say it after a turn, for nothing:
/// Codex's session logs. Every other backend is read only out of band.
pub(crate) async fn read_after_turn(root: &Path, backend_id: &str) -> Option<PlanUsage> {
    if backend_id != CODEX {
        return None;
    }
    read_codex_log(&launch_env(root, backend_id), crate::session::now_ms()).await
}

/// Codex's app-server, or its logs when the app-server cannot be reached.
/// An app-server that answers with no limits is no reading: the logs may
/// hold an older login's.
async fn read_codex(
    binary: Option<PathBuf>,
    env: &[(String, String)],
    scratch: &Path,
    bound: Duration,
    now: i64,
) -> LoginReading {
    let (plan, catalog) = match binary {
        Some(binary) => codex_server::read(&binary, env, scratch, bound, now).await,
        None => (Err(codex_server::Unreached), None),
    };
    let plan = match plan {
        Ok(reading) => reading,
        Err(codex_server::Unreached) => read_codex_log(env, now).await,
    };
    LoginReading { plan, catalog }
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

/// A variable as a teammate on the backend would have it: its launch
/// environment over the desk's.
fn variable(env: &[(String, String)], name: &str) -> Option<String> {
    env.iter()
        .rev()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
        .or_else(|| std::env::var(name).ok())
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
    // A fraction of the window, as the SDK says, and over 1 when use has run
    // past the limit (1.05 is 105%, not 1%).
    let used_percent = info
        .get("utilization")
        .and_then(Value::as_f64)
        .map(|used| (used * 100.0).clamp(0.0, 100.0));
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

/// How much of the account a reading says. The read out of band and
/// Codex's own logs say every window the account has; Claude Code's report
/// after a reply says one, and a harness that names its plan as its session
/// opens says none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Extent {
    /// Every window the account has now.
    Whole,
    /// Some of them.
    Part,
}

/// An account's reading with another merged in, window by window.
///
/// A later whole reading is the account's windows: a kept window it does not
/// have is gone, the account no longer having it (a Max login's 5-hour
/// window, lifted in November 2026, was shown for eleven days after its
/// last reset). Extra usage is the exception while its period lasts, as
/// only Claude Code's report after a reply says it. A whole reading with no windows is a read
/// that found nothing, not an account with none, so it is taken as a part.
/// A later part replaces the windows it has and leaves the rest. An earlier
/// reading adds no window, since the later may have dropped it: an agent's
/// report is merged again each time its session's info is.
///
/// A later reading of the same window in the same period never relaxes its
/// status, because use only grows until the window resets: a reading
/// worked out from a percentage does not undo a warning the agent itself
/// gave. A figure that use cannot have come down to is the exception: the
/// limit itself moved, so the figure stands ([`limit_moved`]). Claude Code
/// refused Fable at 05:43 on 2026-11-10 and read it at 31% that afternoon,
/// in the same week. What a window or the plan says only in one reading (its
/// length, its label, the plan's name, banked resets) is kept from
/// whichever said it, the later first. So is how much of a window is used,
/// in the same period: Claude Code's report after a reply leaves out
/// `utilization` when its CLI has no figure, and the week read at 21% showed
/// a dash after every reply (2026-11-10). A window rolled over keeps no
/// figure from the last period, and a refusal keeps none: it says the window
/// is used up, which every line draws as full. Claude Code's report of a model's
/// own allowance is filed under that model's window ([`OVERAGE_INCLUDED`]).
///
/// A window whose reset has passed is left to [`as_of`].
pub(crate) fn merged(was: Option<&PlanUsage>, mut reading: PlanUsage, extent: Extent) -> PlanUsage {
    let Some(was) = was else {
        return reading;
    };
    let newer = reading.at >= was.at;
    let whole = extent == Extent::Whole && !reading.windows.is_empty();
    let at = was.at.max(reading.at);
    let model_window = model_window(&reading).or_else(|| model_window(was));
    if let Some(kind) = &model_window {
        for window in &mut reading.windows {
            if window.kind == OVERAGE_INCLUDED {
                window.kind.clone_from(kind);
            }
        }
    }
    let mut plan = was.clone();
    if whole && newer {
        plan.windows.retain(|kept| {
            reading.windows.iter().any(|one| one.kind == kept.kind)
                || (kept.kind == EXTRA_USAGE
                    && kept.resets_at.is_some_and(|resets| resets > reading.at))
        });
    }
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
    let windows = if newer { reading.windows } else { Vec::new() };
    for mut window in windows {
        if let Some(kept) = plan.windows.iter().find(|one| one.kind == window.kind) {
            let same = same_period(kept, &window);
            let refused = window.status == PlanStatus::Limited;
            // Before the figure is kept: only one the later reading gives is
            // a sign the limit moved.
            if same
                && severity(kept.status) > severity(window.status)
                && !limit_moved(kept, &window)
            {
                window.status = kept.status;
            }
            if same && !refused {
                window.used_percent = window.used_percent.or(kept.used_percent);
            }
            window.window_minutes = window.window_minutes.or(kept.window_minutes);
            window.label = window.label.take().or_else(|| kept.label.clone());
        }
        plan = with_window(Some(plan), window, at);
    }
    if model_window.is_some() {
        plan.windows.retain(|one| one.kind != OVERAGE_INCLUDED);
    }
    plan.at = at;
    plan
}

/// The reading as it stands at `now`. A window whose reset has passed since
/// it was read has rolled over: what was used of it, and whether it was
/// refused, were the last period's, so it keeps its line with no figure
/// until it is read again, the way a weekly window not yet read shows a
/// dash. The account most likely still has it, and a line that went away at
/// the reset and came back at the next read would look broken. Once a whole
/// period more has passed with nothing reading it, or at once for a window
/// whose length is not known, it is gone.
pub(crate) fn as_of(mut plan: PlanUsage, now: i64) -> PlanUsage {
    plan.windows.retain_mut(|one| {
        let Some(resets_at) = one.resets_at.filter(|resets_at| *resets_at <= now) else {
            return true;
        };
        let length = one.window_minutes.unwrap_or(0).max(0) * 60_000;
        if now >= resets_at.saturating_add(length) {
            return false;
        }
        one.used_percent = None;
        one.status = PlanStatus::Allowed;
        true
    });
    plan
}

/// The kind Claude Code reports its extra usage under after a reply: money
/// spent past the plan, which the read out of band does not say.
const EXTRA_USAGE: &str = "overage";

/// The kind Claude Code reports a model's own weekly allowance under after
/// a reply: the allowance the plan includes of a model that otherwise takes
/// usage credits. Claude Code's own messages call it the "Fable limit"; the
/// read out of band lists the same window by its model (`seven_day_fable`,
/// "Weekly · Fable"), so while a plan has one such window the report is a
/// reading of it, not a window of its own.
const OVERAGE_INCLUDED: &str = "seven_day_overage_included";

/// The plan's one window named by its model, the only windows the read out
/// of band labels; `None` when it has none, or several and the report could
/// be any of them.
fn model_window(plan: &PlanUsage) -> Option<String> {
    let mut named = plan.windows.iter().filter(|one| one.label.is_some());
    let one = named.next()?;
    named.next().is_none().then(|| one.kind.clone())
}

/// Whether a window refused, or warned, and read again in the same period
/// has had its limit moved rather than its use undone: a refused one read
/// under the warning line, a warned one read well under it. A figure near
/// the line is no sign of that, only of a percentage rounded on another
/// side of it than the agent's own warning.
fn limit_moved(kept: &PlanWindow, later: &PlanWindow) -> bool {
    let Some(used) = later.used_percent else {
        return false;
    };
    match kept.status {
        PlanStatus::Limited => used < WARNING_LINE,
        PlanStatus::Warning => used < WELL_UNDER_WARNING_LINE,
        PlanStatus::Allowed => false,
    }
}

/// Where a window is near its limit when the agent gives only a figure.
const WARNING_LINE: f64 = 80.0;

/// A figure this far under the warning line, read in the period of a
/// warning, means the limit was raised: Claude Code warned on a Max
/// login's week, Anthropic raised the week's limits, and the read put it at
/// 21% with the warning still kept (2026-11-10).
const WELL_UNDER_WARNING_LINE: f64 = 60.0;

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
    } else if used_percent >= WARNING_LINE {
        PlanStatus::Warning
    } else {
        PlanStatus::Allowed
    }
}

/// The 5-hour window first, then the weekly one, the monthly one, then a
/// model's own (Opus, Sonnet, then any other weekly one by its model), then
/// any window the desk does not know, and extra usage last: it is money
/// spent past the plan, not a window of it.
pub(crate) fn order(kind: &str) -> u8 {
    match kind {
        "five_hour" => 0,
        "seven_day" => 1,
        "monthly" => 2,
        "seven_day_opus" => 3,
        "seven_day_sonnet" => 4,
        kind if kind.starts_with("seven_day_") => 5,
        EXTRA_USAGE => 7,
        _ => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antigravitys_tier_is_the_plans_name_and_nothing_else_is() {
        let meta = |value: Value| value.as_object().cloned();
        let named = named_plan(
            meta(serde_json::json!({"antigravity": {"tier": {
                "name": "Google AI Pro", "id": "g1-pro-tier", "upgradeUri": "https://antigravity.google/g1-upgrade"
            }}}))
            .as_ref(),
            7,
        )
        .expect("a plan");
        assert_eq!(named.plan.as_deref(), Some("Google AI Pro"));
        assert_eq!(named.at, 7);
        assert!(named.windows.is_empty(), "no quota is read");
        assert_eq!(named_plan(None, 7), None);
        assert_eq!(
            named_plan(
                meta(serde_json::json!({"antigravity": {"tier": true}})).as_ref(),
                7
            ),
            None
        );
    }
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
    fn a_later_part_replaces_its_windows_and_leaves_the_rest_and_an_earlier_one_adds_none() {
        let kept = usage(
            vec![window("five_hour", 30.0), window("seven_day", 50.0)],
            10,
        );
        let later = usage(vec![window("five_hour", 35.0)], 20);
        let plan = merged(Some(&kept), later, Extent::Part);
        assert_eq!(plan.at, 20);
        assert_eq!(plan.windows[0].used_percent, Some(35.0));
        assert_eq!(plan.windows[1].used_percent, Some(50.0));
        let earlier = usage(
            vec![window("five_hour", 5.0), window("seven_day_opus", 12.0)],
            5,
        );
        // An agent's old report, merged again with its session's info: the
        // later reading may have dropped what it has.
        for extent in [Extent::Part, Extent::Whole] {
            let again = merged(Some(&plan), earlier.clone(), extent);
            assert_eq!(again.at, 20);
            assert_eq!(
                again
                    .windows
                    .iter()
                    .map(|one| (one.kind.as_str(), one.used_percent))
                    .collect::<Vec<_>>(),
                [("five_hour", Some(35.0)), ("seven_day", Some(50.0))]
            );
        }
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
        let plan = merged(Some(&out_of_band), reply, Extent::Part);
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
        assert_eq!(
            merged(Some(&plan), renamed, Extent::Part).plan.as_deref(),
            Some("Pro")
        );
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
        let plan = merged(Some(&warned), read(1_793_774_940_000), Extent::Part);
        assert_eq!(plan.windows[0].status, PlanStatus::Warning);
        assert_eq!(plan.windows[0].used_percent, Some(77.0));
        // A week on, a new period.
        let plan = merged(Some(&warned), read(1_794_379_740_000), Extent::Part);
        assert_eq!(plan.windows[0].status, PlanStatus::Allowed);
    }

    #[test]
    fn a_reported_window_past_its_limit_reads_its_fraction_as_over_100_percent_and_full() {
        let past = reported_window(Some(&meta(json!({
            "status": "allowed_warning",
            "rateLimitType": "five_hour",
            "utilization": 1.05
        }))))
        .unwrap();
        assert_eq!(past.used_percent, Some(100.0));
    }

    /// Claude Code 2.1.322 on a Max login, 2026-11-10: the reply's report
    /// after Fable was refused, and the read out of band that afternoon.
    fn fable_refused_then_read() -> (PlanWindow, PlanUsage) {
        let refused = reported_window(Some(&meta(json!({
            "status": "rejected",
            "rateLimitType": "seven_day_overage_included",
            "resetsAt": 1_794_859_200
        }))))
        .unwrap();
        let read = claude_cli::reading(
            &Value::Null,
            &json!({
                "subscription_type": "max",
                "rate_limits_available": true,
                "rate_limits": {
                    "five_hour": null,
                    "seven_day": {"utilization": 17, "resets_at": "2026-11-16T20:00:00.492522+00:00"},
                    "seven_day_opus": null,
                    "seven_day_sonnet": null,
                    "extra_usage": {"is_enabled": false, "monthly_limit": null, "used_credits": null, "utilization": null},
                    "model_scoped": [{"display_name": "Fable", "utilization": 31, "resets_at": "2026-11-16T20:00:00.492641+00:00"}]
                }
            }),
            20,
        )
        .unwrap();
        (refused, read)
    }

    fn kinds(plan: &PlanUsage) -> Vec<(&str, PlanStatus, Option<f64>)> {
        plan.windows
            .iter()
            .map(|one| (one.kind.as_str(), one.status, one.used_percent))
            .collect()
    }

    #[test]
    fn claude_codes_report_of_a_models_own_allowance_is_a_reading_of_that_models_window() {
        let (refused, read) = fable_refused_then_read();
        assert_eq!(refused.kind, OVERAGE_INCLUDED);
        assert_eq!(refused.window_minutes, Some(WEEK_MINUTES));
        // Reported after the read: filed under Fable, its name kept.
        let plan = merged(Some(&read), usage(vec![refused.clone()], 30), Extent::Part);
        assert_eq!(
            kinds(&plan),
            [
                ("seven_day", PlanStatus::Allowed, Some(17.0)),
                ("seven_day_fable", PlanStatus::Limited, None),
            ]
        );
        assert_eq!(plan.windows[1].label.as_deref(), Some("Weekly · Fable"));
        // Reported before any read: a window of its own until the read
        // names it, which takes its place.
        let alone = merged(None, usage(vec![refused.clone()], 10), Extent::Part);
        assert_eq!(alone.windows[0].kind, OVERAGE_INCLUDED);
        let plan = merged(Some(&alone), read.clone(), Extent::Whole);
        assert_eq!(
            plan.windows
                .iter()
                .map(|one| one.kind.as_str())
                .collect::<Vec<_>>(),
            ["seven_day", "seven_day_fable"]
        );
        // With two models' own windows the report could be either, so it
        // stays its own.
        let mut two = read;
        let mut opus = two.windows[1].clone();
        opus.kind = "seven_day_opus_5".into();
        opus.label = Some("Weekly · Opus 5".into());
        two.windows.push(opus);
        let plan = merged(Some(&two), usage(vec![refused], 30), Extent::Part);
        assert!(plan.windows.iter().any(|one| one.kind == OVERAGE_INCLUDED));
    }

    #[test]
    fn a_refusal_gives_way_to_a_later_figure_under_the_warning_line_and_a_warning_to_one_well_under_it()
     {
        let (refused, read) = fable_refused_then_read();
        let mut kept = read.clone();
        kept.at = 10;
        let kept = merged(Some(&kept), usage(vec![refused], 15), Extent::Part);
        assert_eq!(kept.windows[1].status, PlanStatus::Limited);
        // The afternoon's read, the same week: 31% stands.
        let plan = merged(Some(&kept), read.clone(), Extent::Whole);
        assert_eq!(
            kinds(&plan)[1],
            ("seven_day_fable", PlanStatus::Allowed, Some(31.0))
        );
        // A figure near the limit keeps the refusal.
        let mut near = read.clone();
        near.at = 40;
        near.windows[1].used_percent = Some(97.0);
        near.windows[1].status = status_of(97.0);
        let plan = merged(Some(&kept), near, Extent::Whole);
        assert_eq!(plan.windows[1].status, PlanStatus::Limited);
        // A warning the agent gave stands over a figure near the line, and
        // gives way to one well under it.
        let mut warned = read.clone();
        warned.windows[0].status = PlanStatus::Warning;
        warned.at = 10;
        let mut near_the_line = read.clone();
        near_the_line.windows[0].used_percent = Some(76.0);
        let plan = merged(Some(&warned), near_the_line, Extent::Whole);
        assert_eq!(
            kinds(&plan)[0],
            ("seven_day", PlanStatus::Warning, Some(76.0))
        );
        let plan = merged(Some(&warned), read, Extent::Whole);
        assert_eq!(
            kinds(&plan)[0],
            ("seven_day", PlanStatus::Allowed, Some(17.0))
        );
    }

    #[test]
    fn windows_read_five_hours_weekly_monthly_then_each_models_then_unknown_and_extra_last() {
        let mut plan = None;
        for (at, kind) in [
            "overage",
            "period",
            "seven_day_fable",
            OVERAGE_INCLUDED,
            "seven_day_sonnet",
            "seven_day_opus",
            "monthly",
            "seven_day",
            "five_hour",
        ]
        .into_iter()
        .enumerate()
        {
            plan = Some(with_window(plan, window(kind, 1.0), at as i64));
        }
        assert_eq!(
            plan.unwrap()
                .windows
                .iter()
                .map(|one| one.kind.as_str())
                .collect::<Vec<_>>(),
            [
                "five_hour",
                "seven_day",
                "monthly",
                "seven_day_opus",
                "seven_day_sonnet",
                "seven_day_fable",
                OVERAGE_INCLUDED,
                "period",
                "overage"
            ]
        );
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

    /// A Max login's kept reading on 2026-11-10, stuck since 2026-10-30: a
    /// 5-hour window and extra usage, each eleven days past its reset, kept
    /// through every read since because a window only the kept reading had
    /// was kept.
    fn stuck() -> PlanUsage {
        serde_json::from_value(json!({
            "windows": [
                {"kind": "five_hour", "usedPercent": 9.0, "resetsAt": 1_793_421_000_638_i64, "windowMinutes": 300, "status": "allowed"},
                {"kind": "seven_day", "usedPercent": 20.0, "resetsAt": 1_794_859_200_679_i64, "windowMinutes": 10_080, "status": "allowed"},
                {"kind": "seven_day_fable", "usedPercent": 31.0, "resetsAt": 1_794_859_200_641_i64, "windowMinutes": 10_080, "label": "Weekly · Fable", "status": "allowed"},
                {"kind": "overage", "resetsAt": 1_793_491_200_000_i64, "status": "allowed"}
            ],
            "at": 1_794_357_596_343_i64,
            "plan": "Max"
        }))
        .unwrap()
    }

    /// When the reading after [`stuck`] was taken.
    const STUCK_READ_AT: i64 = 1_794_358_200_000;

    /// The read out of band that login gave with the file gone: the week
    /// and Fable's, and no 5-hour window.
    fn read_without_five_hours(at: i64) -> PlanUsage {
        claude_cli::reading(
            &Value::Null,
            &json!({
                "subscription_type": "max",
                "rate_limits_available": true,
                "rate_limits": {
                    "five_hour": null,
                    "seven_day": {"utilization": 21, "resets_at": "2026-11-16T20:00:00.679+00:00"},
                    "model_scoped": [{"display_name": "Fable", "utilization": 33, "resets_at": "2026-11-16T20:00:00.641+00:00"}]
                }
            }),
            at,
        )
        .unwrap()
    }

    fn kinds_of(plan: &PlanUsage) -> Vec<&str> {
        plan.windows.iter().map(|one| one.kind.as_str()).collect()
    }

    #[test]
    fn a_whole_reading_drops_a_window_the_account_no_longer_has() {
        let plan = merged(
            Some(&stuck()),
            read_without_five_hours(STUCK_READ_AT),
            Extent::Whole,
        );
        assert_eq!(kinds_of(&plan), ["seven_day", "seven_day_fable"]);
        assert_eq!(
            kinds(&plan),
            [
                ("seven_day", PlanStatus::Allowed, Some(21.0)),
                ("seven_day_fable", PlanStatus::Allowed, Some(33.0)),
            ]
        );
        assert_eq!(plan.windows[1].label.as_deref(), Some("Weekly · Fable"));
        assert_eq!(plan.plan.as_deref(), Some("Max"));
        assert_eq!(plan.at, STUCK_READ_AT);
        // A 5-hour window still in its period goes too: the account says it
        // has none. Extra usage, which the read never says, stays while its
        // period lasts.
        let mut lately = stuck();
        lately.windows[0].resets_at = Some(STUCK_READ_AT + 3_600_000);
        lately.windows[3].resets_at = Some(STUCK_READ_AT + 86_400_000);
        let plan = merged(
            Some(&lately),
            read_without_five_hours(STUCK_READ_AT),
            Extent::Whole,
        );
        assert_eq!(kinds_of(&plan), ["seven_day", "seven_day_fable", "overage"]);
        // The same reading as a part leaves every window it lacks.
        let plan = merged(
            Some(&lately),
            read_without_five_hours(STUCK_READ_AT),
            Extent::Part,
        );
        assert_eq!(
            kinds_of(&plan),
            ["five_hour", "seven_day", "seven_day_fable", "overage"]
        );
    }

    #[test]
    fn a_report_after_a_whole_reading_neither_brings_back_nor_drops_a_window() {
        let read = merged(
            Some(&stuck()),
            read_without_five_hours(STUCK_READ_AT),
            Extent::Whole,
        );
        let report = |info: Value, at: i64| {
            let window = reported_window(Some(&meta(info))).unwrap();
            with_window(None, window, at)
        };
        // The week, reported after a reply.
        let plan = merged(
            Some(&read),
            report(
                json!({"status": "allowed", "rateLimitType": "seven_day", "utilization": 0.22, "resetsAt": 1_794_859_200}),
                STUCK_READ_AT + 60_000,
            ),
            Extent::Part,
        );
        assert_eq!(kinds_of(&plan), ["seven_day", "seven_day_fable"]);
        assert_eq!(plan.windows[0].used_percent, Some(22.0));
        assert_eq!(plan.windows[1].used_percent, Some(33.0));
        // An agent's old report of the 5-hour window, merged again with its
        // session's info, does not bring it back.
        let again = merged(
            Some(&plan),
            report(
                json!({"status": "allowed", "rateLimitType": "five_hour", "utilization": 0.09, "resetsAt": 1_793_421_000}),
                1_793_410_000_000,
            ),
            Extent::Part,
        );
        assert_eq!(again, plan);
        // A new report of it is the account having it again.
        let plan = merged(
            Some(&plan),
            report(
                json!({"status": "allowed", "rateLimitType": "five_hour", "utilization": 0.01, "resetsAt": 1_794_376_000}),
                STUCK_READ_AT + 120_000,
            ),
            Extent::Part,
        );
        assert_eq!(
            kinds_of(&plan),
            ["five_hour", "seven_day", "seven_day_fable"]
        );
    }

    #[test]
    fn a_warning_and_a_refusal_hold_through_a_whole_reading_as_through_a_part() {
        let (refused, read) = fable_refused_then_read();
        // Warned on the week at 78%, Fable refused, and a 5-hour window kept
        // from before: the whole read drops the 5-hour window and keeps both
        // rules.
        let mut kept = read.clone();
        kept.at = 10;
        kept.windows[0].used_percent = Some(78.0);
        kept.windows[0].status = PlanStatus::Warning;
        kept.windows.insert(0, window("five_hour", 12.0));
        let kept = merged(Some(&kept), usage(vec![refused], 15), Extent::Part);
        assert_eq!(kinds(&kept)[2].1, PlanStatus::Limited);
        let mut near = read.clone();
        near.windows[0].used_percent = Some(79.0);
        near.windows[1].used_percent = Some(97.0);
        near.windows[1].status = status_of(97.0);
        let plan = merged(Some(&kept), near, Extent::Whole);
        assert_eq!(
            kinds(&plan),
            [
                ("seven_day", PlanStatus::Warning, Some(79.0)),
                ("seven_day_fable", PlanStatus::Limited, Some(97.0)),
            ]
        );
        assert_eq!(plan.windows[1].label.as_deref(), Some("Weekly · Fable"));
        // A figure well under the line: the limit moved, the figure stands.
        let plan = merged(Some(&kept), read, Extent::Whole);
        assert_eq!(
            kinds(&plan)[1],
            ("seven_day_fable", PlanStatus::Allowed, Some(31.0))
        );
    }

    #[test]
    fn a_window_past_its_reset_holds_no_figure_and_goes_once_a_period_more_has_passed() {
        let hour = 3_600_000;
        let reset = 1_794_340_000_000;
        let mut five = window("five_hour", 100.0);
        five.resets_at = Some(reset);
        five.window_minutes = Some(FIVE_HOUR_MINUTES);
        let mut week = window("seven_day", 40.0);
        week.resets_at = Some(reset + 100 * hour);
        week.window_minutes = Some(WEEK_MINUTES);
        let mut extra = window(EXTRA_USAGE, 50.0);
        extra.resets_at = Some(reset);
        let plan = usage(vec![five, week, extra], reset - hour);
        // Before the reset, as it was read.
        assert_eq!(as_of(plan.clone(), reset - 1), plan);
        // Just past it: the 5-hour line stays, refused no more and with no
        // figure, its reset still said; extra usage, whose length is not
        // known, is gone.
        let rolled = as_of(plan.clone(), reset + 60_000);
        assert_eq!(
            kinds(&rolled),
            [
                ("five_hour", PlanStatus::Allowed, None),
                ("seven_day", PlanStatus::Allowed, Some(40.0)),
            ]
        );
        assert_eq!(rolled.windows[0].resets_at, Some(reset));
        assert_eq!(rolled.windows[0].window_minutes, Some(FIVE_HOUR_MINUTES));
        // Five hours on with nothing reading it, gone.
        assert_eq!(kinds_of(&as_of(plan, reset + 5 * hour)), ["seven_day"]);
        // The stuck reading as the desk loads it the next day: the 5-hour
        // window and extra usage are gone, the week as it was.
        let loaded = as_of(stuck(), STUCK_READ_AT);
        assert_eq!(kinds_of(&loaded), ["seven_day", "seven_day_fable"]);
        assert_eq!(loaded.windows[0].used_percent, Some(20.0));
    }

    /// The same login's kept reading on a desk without this rule, read
    /// three minutes before: a 5-hour window 261 hours past its reset, the
    /// week still warned at 21% after its limit was raised, Fable refused,
    /// and Claude Code's report of the Fable allowance refused beside it.
    fn live_desk(now: i64) -> PlanUsage {
        let hour = 3_600_000;
        let week = now + 138 * hour;
        serde_json::from_value(json!({
            "windows": [
                {"kind": "five_hour", "resetsAt": now - 261 * hour, "windowMinutes": 300, "status": "allowed"},
                {"kind": "seven_day", "usedPercent": 21.0, "resetsAt": week, "windowMinutes": 10_080, "status": "warning"},
                {"kind": "seven_day_fable", "usedPercent": 34.0, "resetsAt": week, "windowMinutes": 10_080, "label": "Weekly · Fable", "status": "limited"},
                {"kind": OVERAGE_INCLUDED, "resetsAt": week, "windowMinutes": 10_080, "status": "limited"}
            ],
            "at": now - 180_000,
            "plan": "Max"
        }))
        .unwrap()
    }

    #[test]
    fn a_whole_reading_well_under_a_kept_warning_or_refusal_is_the_whole_reading() {
        let now = STUCK_READ_AT;
        let kept = live_desk(now);
        let week = kept.windows[1].resets_at.unwrap();
        let mut read = read_without_five_hours(now);
        for (window, used) in read.windows.iter_mut().zip([21.0, 34.0]) {
            window.used_percent = Some(used);
            window.status = status_of(used);
            window.resets_at = Some(week);
        }
        let plan = as_of(merged(Some(&kept), read.clone(), Extent::Whole), now);
        assert_eq!(plan.windows, read.windows);
        assert_eq!(
            kinds(&plan),
            [
                ("seven_day", PlanStatus::Allowed, Some(21.0)),
                ("seven_day_fable", PlanStatus::Allowed, Some(34.0)),
            ]
        );
        assert_eq!(plan.windows[1].label.as_deref(), Some("Weekly · Fable"));
        // The week read near the warning line instead keeps the warning.
        let mut near = read;
        near.windows[0].used_percent = Some(77.0);
        near.windows[0].status = status_of(77.0);
        let plan = merged(Some(&kept), near, Extent::Whole);
        assert_eq!(
            kinds(&plan)[0],
            ("seven_day", PlanStatus::Warning, Some(77.0))
        );
    }

    #[test]
    fn a_login_nobody_reads_any_more_shows_no_window_past_its_reset_as_current() {
        let hour = 3_600_000;
        let now = STUCK_READ_AT;
        // Grok Build's week, read eight days ago, 175 hours past its reset.
        let mut week = window("seven_day", 11.0);
        week.resets_at = Some(now - 175 * hour);
        week.window_minutes = Some(WEEK_MINUTES);
        let grok = usage(vec![week.clone()], now - 8 * 24 * hour);
        let plan = as_of(grok, now);
        assert_eq!(plan.windows, []);
        // Had it reset within the week, its line would hold no figure.
        week.resets_at = Some(now - 100 * hour);
        let plan = as_of(usage(vec![week], now - 5 * 24 * hour), now);
        assert_eq!(kinds(&plan), [("seven_day", PlanStatus::Allowed, None)]);
    }

    /// A Max login's kept reading on 0.49.0-beta.2, 2026-11-10, from the
    /// whole read minutes before a Claude reply: the week at 21%.
    fn read_before_the_reply() -> PlanUsage {
        serde_json::from_value(json!({
            "windows": [
                {"kind": "seven_day", "usedPercent": 21.0, "resetsAt": 1_794_859_200_352_i64, "windowMinutes": 10_080, "status": "allowed"},
                {"kind": "seven_day_fable", "usedPercent": 38.0, "resetsAt": 1_794_859_200_360_i64, "windowMinutes": 10_080, "label": "Weekly · Fable", "status": "allowed"}
            ],
            "at": 1_794_368_400_000_i64,
            "plan": "Max"
        }))
        .unwrap()
    }

    /// When the reply's report came.
    const REPLY_AT: i64 = 1_794_368_907_078;

    /// Claude Code's report after a reply, with no `utilization`.
    fn figureless(status: &str, resets_at: i64) -> PlanUsage {
        let window = reported_window(Some(&meta(json!({
            "status": status,
            "rateLimitType": "seven_day",
            "resetsAt": resets_at
        }))))
        .unwrap();
        assert_eq!(window.used_percent, None);
        with_window(None, window, REPLY_AT)
    }

    #[test]
    fn a_report_with_no_figure_keeps_the_figure_read_in_the_same_period() {
        let plan = merged(
            Some(&read_before_the_reply()),
            figureless("allowed", 1_794_859_200),
            Extent::Part,
        );
        assert_eq!(
            kinds(&plan),
            [
                ("seven_day", PlanStatus::Allowed, Some(21.0)),
                ("seven_day_fable", PlanStatus::Allowed, Some(38.0)),
            ]
        );
        // The report's own reset and the rest as the read said them.
        assert_eq!(plan.windows[0].resets_at, Some(1_794_859_200_000));
        assert_eq!(plan.windows[0].window_minutes, Some(WEEK_MINUTES));
        assert_eq!(plan.windows[1].label.as_deref(), Some("Weekly · Fable"));
        assert_eq!(plan.at, REPLY_AT);
        // Warned with no figure: the figure and the warning.
        let plan = merged(
            Some(&read_before_the_reply()),
            figureless("allowed_warning", 1_794_859_200),
            Extent::Part,
        );
        assert_eq!(
            kinds(&plan)[0],
            ("seven_day", PlanStatus::Warning, Some(21.0))
        );
        // A warning kept at 21% stands through a later "allowed" with no
        // figure: the kept figure is no sign the limit moved.
        let plan = merged(
            Some(&plan),
            with_window(
                None,
                reported_window(Some(&meta(json!({
                    "status": "allowed", "rateLimitType": "seven_day", "resetsAt": 1_794_859_200
                }))))
                .unwrap(),
                REPLY_AT + 60_000,
            ),
            Extent::Part,
        );
        assert_eq!(
            kinds(&plan)[0],
            ("seven_day", PlanStatus::Warning, Some(21.0))
        );
    }

    #[test]
    fn a_report_with_no_figure_in_a_new_period_keeps_none() {
        let plan = merged(
            Some(&read_before_the_reply()),
            figureless("allowed", 1_794_859_200 + WEEK_MINUTES * 60),
            Extent::Part,
        );
        assert_eq!(kinds(&plan)[0], ("seven_day", PlanStatus::Allowed, None));
    }

    #[test]
    fn a_refusal_with_no_figure_keeps_none_and_a_later_report_with_none_does_not_lift_it() {
        let mut kept = read_before_the_reply();
        kept.windows[0].used_percent = Some(85.0);
        kept.windows[0].status = PlanStatus::Warning;
        let plan = merged(
            Some(&kept),
            figureless("rejected", 1_794_859_200),
            Extent::Part,
        );
        assert_eq!(kinds(&plan)[0], ("seven_day", PlanStatus::Limited, None));
        // A later "allowed" with no figure in the same week: still refused.
        let plan = merged(
            Some(&plan),
            with_window(
                None,
                reported_window(Some(&meta(json!({
                    "status": "allowed", "rateLimitType": "seven_day", "resetsAt": 1_794_859_200
                }))))
                .unwrap(),
                REPLY_AT + 60_000,
            ),
            Extent::Part,
        );
        assert_eq!(kinds(&plan)[0], ("seven_day", PlanStatus::Limited, None));
    }

    #[test]
    fn a_whole_reading_with_no_windows_leaves_the_windows_kept() {
        let kept = merged(
            Some(&stuck()),
            read_without_five_hours(STUCK_READ_AT),
            Extent::Whole,
        );
        // Claude Code answered with every window empty.
        let empty = claude_cli::reading(
            &Value::Null,
            &json!({
                "subscription_type": "max",
                "rate_limits_available": true,
                "rate_limits": {"five_hour": null, "seven_day": null, "model_scoped": []}
            }),
            STUCK_READ_AT + 600_000,
        )
        .unwrap();
        assert_eq!(empty.windows, []);
        let plan = merged(Some(&kept), empty, Extent::Whole);
        assert_eq!(plan.windows, kept.windows);
        assert_eq!(plan.at, STUCK_READ_AT + 600_000);
        // A login whose plan limits do not apply has no reading at all.
        assert_eq!(
            claude_cli::reading(&Value::Null, &json!({"rate_limits_available": false}), 1),
            None
        );
    }

    /// Codex's app-server, when it cannot be started, leaves the reading to
    /// Codex's own logs; one that answers is the reading, logs or not.
    #[cfg(unix)]
    #[tokio::test]
    async fn codex_is_read_from_its_app_server_and_from_its_logs_when_that_cannot_be_reached() {
        let folder = std::env::temp_dir().join(format!(
            "hotline-plan-codex-{}-{}",
            std::process::id(),
            chrono::Local::now()
                .timestamp_nanos_opt()
                .unwrap_or_default()
        ));
        let home = folder.join("codex-home");
        let day = home.join("sessions/2026/10/29");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(
            day.join("rollout-2026-10-29T08-00-00-a.jsonl"),
            r#"{"timestamp":"2026-10-29T08:00:01.000Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":7.0,"window_minutes":10080,"resets_at":1793774976},"secondary":null,"plan_type":"plus"}}}"#,
        )
        .unwrap();
        let env = vec![(
            "CODEX_HOME".to_string(),
            home.to_string_lossy().into_owned(),
        )];
        let scratch = folder.join("scratch");
        // No codex to start: the logs.
        let missing = folder.join("no-codex-here");
        let plan = read_codex(Some(missing), &env, &scratch, Duration::from_secs(5), 1)
            .await
            .plan
            .expect("the logs' reading");
        assert_eq!(plan.windows[0].used_percent, Some(7.0));
        assert_eq!(plan.plan.as_deref(), Some("ChatGPT Plus"));
        assert_eq!(
            read_codex(None, &env, &scratch, Duration::from_secs(5), 1).await,
            LoginReading {
                plan: Some(plan),
                catalog: None
            }
        );
        // A codex that answers: its reading, and its models.
        let codex = codex_server::tests::fake_codex(&folder, codex_server::tests::LIMITS);
        let read = read_codex(Some(codex), &env, &scratch, Duration::from_secs(5), 1).await;
        let plan = read.plan.expect("the app-server's reading");
        assert_eq!(plan.windows[0].kind, "five_hour");
        assert_eq!(plan.windows[0].used_percent, Some(81.0));
        assert_eq!(read.catalog.expect("the models").models.len(), 2);
        let _ = std::fs::remove_dir_all(folder);
    }
}
