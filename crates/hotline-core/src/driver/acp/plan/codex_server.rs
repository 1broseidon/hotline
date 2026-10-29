//! A Codex login's plan, read out of band from `codex app-server`: started
//! for the one request, its JSON-RPC handshake done, asked
//! `account/rateLimits/read`, and ended. Nothing is sent to a model.
//!
//! ```json
//! {"id":2,"result":{"rateLimits":{"limitId":"codex","planType":"pro",
//!   "primary":{"usedPercent":2.0,"windowDurationMins":10080,"resetsAt":1793774976},
//!   "secondary":{"usedPercent":81.0,"windowDurationMins":300,"resetsAt":1793700000},
//!   "rateLimitReachedType":null},
//!  "rateLimitResetCredits":{"availableCount":1,"credits":[{"status":"available","expiresAt":1794000000}]}}}
//! ```
//!
//! `primary` and `secondary` are positions, not lengths: on a paid plan the
//! weekly window can be either. Each is placed by its `windowDurationMins`.

use super::child::Probe;
use super::{
    FIVE_HOUR_MINUTES, MONTH_MINUTES, WEEK_MINUTES, codex_plan, kind_of_minutes, order, status_of,
};
use crate::contract::{PlanStatus, PlanUsage, PlanWindow};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

/// The app-server could not be started, or did not answer.
#[derive(Debug)]
pub(crate) struct Unreached;

/// The login's plan, read by starting `binary app-server` in `cwd` with
/// `env`: `Ok(None)` when it answered and the login has no plan limits.
pub(crate) async fn read(
    binary: &Path,
    env: &[(String, String)],
    cwd: &Path,
    bound: Duration,
    now: i64,
) -> Result<Option<PlanUsage>, Unreached> {
    let mut probe = match Probe::spawn(binary, &["app-server"], env, &[], cwd) {
        Ok(probe) => probe,
        Err(error) => {
            eprintln!("[plan] codex app-server could not be started to read the plan: {error}");
            return Err(Unreached);
        }
    };
    let asked = tokio::time::timeout(bound, ask(&mut probe)).await;
    probe.end().await;
    match asked {
        Ok(Some(result)) => Ok(reading(&result, now)),
        Ok(None) => Err(Unreached),
        Err(_) => {
            eprintln!("[plan] codex app-server did not say the plan within {bound:?}");
            Err(Unreached)
        }
    }
}

/// The handshake, then the one request.
async fn ask(probe: &mut Probe) -> Option<Value> {
    probe
        .send(&json!({
            "id": 1,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "hotline",
                    "title": "Hotline",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": { "experimentalApi": true },
            },
        }))
        .await
        .ok()?;
    probe.answer(|line| answered(line, 1)).await??;
    probe.send(&json!({ "method": "initialized" })).await.ok()?;
    probe
        .send(&json!({ "id": 2, "method": "account/rateLimits/read" }))
        .await
        .ok()?;
    probe.answer(|line| answered(line, 2)).await?
}

/// The result of request `id`, when this line answers it: `Some(Some(_))`
/// for a result, `Some(None)` for an error, `None` for any other line.
fn answered(line: &Value, id: i64) -> Option<Option<Value>> {
    if line.get("id")?.as_i64()? != id || line.get("method").is_some() {
        return None;
    }
    Some(line.get("result").cloned())
}

/// The reading `account/rateLimits/read` gives: Codex's own allowance (not
/// a model's), its windows by length, the plan, and banked resets.
pub(crate) fn reading(result: &Value, at: i64) -> Option<PlanUsage> {
    let snapshot = result
        .get("rateLimitsByLimitId")
        .and_then(|by_id| by_id.get("codex"))
        .filter(|snapshot| snapshot.is_object())
        .or_else(|| result.get("rateLimits"))?;
    if snapshot
        .get("limitId")
        .and_then(Value::as_str)
        .is_some_and(|id| id != "codex")
    {
        return None;
    }
    let plan_type = snapshot.get("planType").and_then(Value::as_str);
    // Free and Go plans have a monthly allowance where paid plans have a
    // 5-hour one, when Codex does not say how long the window is.
    let monthly = matches!(plan_type, Some("free" | "go"));
    let mut windows: Vec<PlanWindow> = [
        (
            "primary",
            if monthly {
                MONTH_MINUTES
            } else {
                FIVE_HOUR_MINUTES
            },
        ),
        ("secondary", WEEK_MINUTES),
    ]
    .iter()
    .filter_map(|(position, fallback)| window(snapshot.get(*position)?, *fallback))
    .collect();
    if windows.is_empty() {
        return None;
    }
    windows.sort_by_key(|one| order(&one.kind));
    windows.dedup_by(|a, b| a.kind == b.kind);
    // Refused now: the window nearest its limit is the one that refused.
    let reached = snapshot
        .get("rateLimitReachedType")
        .is_some_and(Value::is_string);
    if reached && !windows.iter().any(|one| one.status == PlanStatus::Limited) {
        let fullest = windows.iter_mut().max_by(|a, b| {
            a.used_percent
                .unwrap_or(0.0)
                .total_cmp(&b.used_percent.unwrap_or(0.0))
        });
        if let Some(fullest) = fullest {
            fullest.status = PlanStatus::Limited;
        }
    }
    let credits = result
        .get("rateLimitResetCredits")
        .filter(|c| c.is_object());
    let banked = credits
        .and_then(|credits| credits.get("availableCount"))
        .and_then(Value::as_u64)
        .map(|count| u32::try_from(count).unwrap_or(u32::MAX));
    let banked_expires_at = credits
        .and_then(|credits| credits.get("credits"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|credit| credit.get("status").and_then(Value::as_str) == Some("available"))
        .filter_map(|credit| credit.get("expiresAt").and_then(Value::as_f64))
        .map(|seconds| (seconds * 1000.0) as i64)
        .min();
    Some(PlanUsage {
        windows,
        at,
        plan: plan_type.and_then(codex_plan),
        banked,
        banked_expires_at,
    })
}

fn window(limit: &Value, fallback_minutes: i64) -> Option<PlanWindow> {
    let used = limit.get("usedPercent")?.as_f64()?.clamp(0.0, 100.0);
    let minutes = limit
        .get("windowDurationMins")
        .and_then(Value::as_i64)
        .unwrap_or(fallback_minutes);
    Some(PlanWindow {
        kind: kind_of_minutes(minutes),
        used_percent: Some(used),
        // Unix seconds, as Codex sends it.
        resets_at: limit
            .get("resetsAt")
            .and_then(Value::as_f64)
            .map(|seconds| (seconds * 1000.0) as i64),
        window_minutes: Some(minutes),
        label: None,
        status: status_of(used),
    })
}

#[cfg(all(test, unix))]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "hotline-codex-server-{name}-{}-{}",
            std::process::id(),
            chrono::Local::now()
                .timestamp_nanos_opt()
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// A stand-in `codex`: run as `codex app-server`, it answers the
    /// handshake and the read, and writes what it heard to `heard`.
    pub(crate) fn fake_codex(folder: &Path, limits: &str) -> PathBuf {
        let script = folder.join("codex");
        let heard = folder.join("heard");
        let body = format!(
            "#!/bin/sh\n[ \"$1\" = app-server ] || exit 2\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> '{heard}'\n  case \"$line\" in\n    *'\"initialize\"'*) printf '%s\\n' '{{\"id\":1,\"result\":{{\"userAgent\":\"codex/0.164.0\"}}}}' ;;\n    *'account/rateLimits/read'*) printf '%s\\n' '{{\"method\":\"account/rateLimits/updated\",\"params\":{{}}}}' '{{\"id\":2,\"result\":{limits}}}' ;;\n  esac\ndone\n",
            heard = heard.display(),
        );
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    pub(crate) const LIMITS: &str = r#"{"rateLimits":{"limitId":"codex","planType":"pro","primary":{"usedPercent":2.0,"windowDurationMins":10080,"resetsAt":1793774976},"secondary":{"usedPercent":81.0,"windowDurationMins":300,"resetsAt":1793700000},"rateLimitReachedType":null},"rateLimitsByLimitId":{"codex":{"limitId":"codex","planType":"pro","primary":{"usedPercent":2.0,"windowDurationMins":10080,"resetsAt":1793774976},"secondary":{"usedPercent":81.0,"windowDurationMins":300,"resetsAt":1793700000}},"codex_fable":{"limitId":"codex_fable","primary":{"usedPercent":99.0}}},"rateLimitResetCredits":{"availableCount":2,"credits":[{"id":"a","status":"available","expiresAt":1794000000,"grantedAt":1,"resetType":"codexRateLimits"},{"id":"b","status":"redeemed","expiresAt":1793000000,"grantedAt":1,"resetType":"codexRateLimits"},{"id":"c","status":"available","expiresAt":1795000000,"grantedAt":1,"resetType":"codexRateLimits"}]}}"#;

    #[tokio::test]
    async fn the_app_server_is_greeted_then_asked_its_rate_limits() {
        let folder = scratch("reading");
        let codex = fake_codex(&folder, LIMITS);
        let plan = read(&codex, &[], &folder.join("cwd"), Duration::from_secs(10), 7)
            .await
            .expect("an answer")
            .expect("a reading");
        assert_eq!(plan.plan.as_deref(), Some("ChatGPT Pro"));
        assert_eq!(plan.banked, Some(2));
        assert_eq!(plan.banked_expires_at, Some(1_794_000_000_000));
        assert_eq!(
            plan.windows,
            [
                PlanWindow {
                    kind: "five_hour".into(),
                    used_percent: Some(81.0),
                    resets_at: Some(1_793_700_000_000),
                    window_minutes: Some(300),
                    label: None,
                    status: PlanStatus::Warning,
                },
                PlanWindow {
                    kind: "seven_day".into(),
                    used_percent: Some(2.0),
                    resets_at: Some(1_793_774_976_000),
                    window_minutes: Some(10_080),
                    label: None,
                    status: PlanStatus::Allowed,
                },
            ]
        );
        let heard: Vec<Value> = std::fs::read_to_string(folder.join("heard"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let methods: Vec<&str> = heard
            .iter()
            .map(|line| line["method"].as_str().unwrap())
            .collect();
        assert_eq!(
            methods,
            ["initialize", "initialized", "account/rateLimits/read"]
        );
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_free_plans_window_without_a_length_is_monthly_and_a_refusal_marks_the_fullest() {
        let plan = reading(
            &serde_json::json!({ "rateLimits": {
                "planType": "free",
                "primary": { "usedPercent": 64.0, "resetsAt": 1_793_774_976 },
                "rateLimitReachedType": "rate_limit_reached"
            }}),
            1,
        )
        .unwrap();
        assert_eq!(plan.plan.as_deref(), Some("ChatGPT Free"));
        assert_eq!(plan.windows[0].kind, "monthly");
        assert_eq!(plan.windows[0].window_minutes, Some(MONTH_MINUTES));
        assert_eq!(plan.windows[0].status, PlanStatus::Limited);
        assert_eq!(plan.banked, None);
    }

    #[test]
    fn a_models_own_allowance_or_no_windows_is_no_reading() {
        assert_eq!(
            reading(
                &serde_json::json!({ "rateLimits": { "limitId": "codex_fable", "primary": { "usedPercent": 3.0 } } }),
                1
            ),
            None
        );
        assert_eq!(
            reading(
                &serde_json::json!({ "rateLimits": { "planType": "pro" } }),
                1
            ),
            None
        );
    }
}
