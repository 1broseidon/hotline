//! A Codex login's plan and its models, read out of band from `codex
//! app-server`: started for the purpose, its JSON-RPC handshake done, asked
//! `account/rateLimits/read` and `model/list` (a page at a time), and ended.
//! Nothing is sent to a model.
//!
//! ```json
//! {"id":2,"result":{"rateLimits":{"limitId":"codex","planType":"pro",
//!   "primary":{"usedPercent":2.0,"windowDurationMins":10080,"resetsAt":1793774976},
//!   "secondary":{"usedPercent":81.0,"windowDurationMins":300,"resetsAt":1793700000},
//!   "rateLimitReachedType":null},
//!  "rateLimitResetCredits":{"availableCount":1,"credits":[{"status":"available","expiresAt":1794000000}]}}}
//! {"id":3,"result":{"data":[{"id":"gpt-6-astra","model":"gpt-6-astra","displayName":"GPT-6-Astra",
//!   "description":"…","hidden":false,"isDefault":false,"defaultReasoningEffort":"medium",
//!   "supportedReasoningEfforts":[{"reasoningEffort":"low","description":"…"}]}],"nextCursor":null}}
//! ```
//!
//! `primary` and `secondary` are positions, not lengths: on a paid plan the
//! weekly window can be either. Each is placed by its `windowDurationMins`.
//!
//! Codex's ACP adapter lists a model by its `id` and names it as below
//! ("GPT-6-Astra" → "6 Astra"); its effort picker is `reasoning_effort`, each
//! level named by its id capitalized. So the catalogue is the adapter's own.

use super::child::Probe;
use super::{
    FIVE_HOUR_MINUTES, MONTH_MINUTES, WEEK_MINUTES, codex_plan, kind_of_minutes, order, status_of,
};
use crate::contract::{ConfigChoice, PlanStatus, PlanUsage, PlanWindow};
use crate::driver::acp::catalog::Catalog;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

/// The app-server could not be started, or did not answer.
#[derive(Debug)]
pub(crate) struct Unreached;

/// How many pages of models are read before the rest is left unread.
const MODEL_PAGES: i64 = 20;

/// The login's plan, read by starting `binary app-server` in `cwd` with
/// `env` (`Ok(None)` when it answered and the login has no plan limits),
/// and its models, when it listed them.
pub(crate) async fn read(
    binary: &Path,
    env: &[(String, String)],
    cwd: &Path,
    bound: Duration,
    now: i64,
) -> (Result<Option<PlanUsage>, Unreached>, Option<Catalog>) {
    let mut probe = match Probe::spawn(binary, &["app-server"], env, &[], cwd) {
        Ok(probe) => probe,
        Err(error) => {
            eprintln!("[plan] codex app-server could not be started to read the plan: {error}");
            return (Err(Unreached), None);
        }
    };
    let mut answers = Answers::default();
    let asked = tokio::time::timeout(bound, ask(&mut probe, &mut answers)).await;
    probe.end().await;
    if asked.is_err() {
        eprintln!("[plan] codex app-server did not say the plan within {bound:?}");
    }
    let plan = answers
        .limits
        .map(|result| reading(&result, now))
        .ok_or(Unreached);
    let catalog = answers.models.and_then(|models| catalog(&models));
    (plan, catalog)
}

/// What the app-server answered, kept as it comes so a later request that
/// runs out of time leaves an earlier answer standing.
#[derive(Default)]
struct Answers {
    limits: Option<Value>,
    /// Every page's models, once the last page is in.
    models: Option<Vec<Value>>,
}

/// The handshake, then the rate limits, then each page of models.
async fn ask(probe: &mut Probe, answers: &mut Answers) -> Option<()> {
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
    answers.limits = probe.answer(|line| answered(line, 2)).await?;
    let mut models = Vec::new();
    let mut cursor = Value::Null;
    for id in 3..3 + MODEL_PAGES {
        probe
            .send(&json!({
                "id": id,
                "method": "model/list",
                "params": { "cursor": cursor, "limit": null },
            }))
            .await
            .ok()?;
        let page = probe.answer(|line| answered(line, id)).await??;
        models.extend(
            page.get("data")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .cloned(),
        );
        cursor = page.get("nextCursor").cloned().unwrap_or(Value::Null);
        if !cursor.is_string() {
            break;
        }
    }
    answers.models = Some(models);
    Some(())
}

/// The models `model/list` gave, as Codex's ACP adapter lists them, each
/// with its reasoning levels and the one it starts at; a model hidden from
/// Codex's own picker is left out. `None` when it listed none.
pub(crate) fn catalog(listed: &[Value]) -> Option<Catalog> {
    let mut catalog = Catalog {
        effort_id: "reasoning_effort".to_string(),
        effort_name: "Reasoning effort".to_string(),
        ..Catalog::default()
    };
    for model in listed {
        let text = |key: &str| {
            model
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
        };
        let Some(id) = text("id") else { continue };
        if model.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let efforts: Vec<ConfigChoice> = model
            .get("supportedReasoningEfforts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|option| {
                let level = option.get("reasoningEffort")?.as_str()?;
                Some(ConfigChoice {
                    id: level.to_string(),
                    name: capitalized(level),
                    description: option
                        .get("description")
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                        .map(str::to_string),
                    group: None,
                    account: None,
                    efforts: None,
                    default_effort: None,
                    fast: None,
                })
            })
            .collect();
        let default_effort = text("defaultReasoningEffort")
            .filter(|level| efforts.iter().any(|one| one.id == *level))
            .map(str::to_string);
        if let Some(slug) = text("model")
            && slug != id
        {
            catalog
                .aliases
                .entry(slug.to_string())
                .or_insert_with(|| id.to_string());
        }
        if model.get("isDefault").and_then(Value::as_bool) == Some(true) {
            catalog.default_model = Some(id.to_string());
        }
        catalog.models.push(ConfigChoice {
            id: id.to_string(),
            name: display_name(text("displayName").unwrap_or(id)),
            description: text("description").map(str::to_string),
            group: None,
            account: None,
            efforts: Some(crate::models::tidy_efforts(efforts)),
            default_effort,
            // A model that can run fast lists the tier beside its own.
            fast: Some(
                model
                    .get("additionalSpeedTiers")
                    .and_then(Value::as_array)
                    .is_some_and(|tiers| tiers.iter().any(|tier| tier.as_str() == Some("fast"))),
            ),
        });
    }
    (!catalog.models.is_empty()).then_some(catalog)
}

/// A model's name as Codex's ACP adapter shows it: past a leading "gpt-",
/// each part between dashes capitalized and spaced ("GPT-6.1-Sol" →
/// "6.1 Sol"); any other name as it is.
fn display_name(name: &str) -> String {
    if name.len() < 4 || !name[..4].eq_ignore_ascii_case("gpt-") {
        return name.to_string();
    }
    name[4..]
        .split(['-', '/'])
        .filter(|part| !part.is_empty())
        .map(capitalized)
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalized(text: &str) -> String {
    let mut letters = text.chars();
    letters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(letters).collect()
    })
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
    /// handshake, the read and `model/list` in two pages, and writes what it
    /// heard to `heard`.
    pub(crate) fn fake_codex(folder: &Path, limits: &str) -> PathBuf {
        let script = folder.join("codex");
        let heard = folder.join("heard");
        let body = format!(
            "#!/bin/sh\n[ \"$1\" = app-server ] || exit 2\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> '{heard}'\n  id=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\\([0-9]*\\).*/\\1/p')\n  case \"$line\" in\n    *'\"initialize\"'*) printf '%s\\n' '{{\"id\":1,\"result\":{{\"userAgent\":\"codex/0.164.0\"}}}}' ;;\n    *'account/rateLimits/read'*) printf '%s\\n' '{{\"method\":\"account/rateLimits/updated\",\"params\":{{}}}}' '{{\"id\":2,\"result\":{limits}}}' ;;\n    *'\"cursor\":\"page-2\"'*) printf '{{\"id\":%s,\"result\":%s}}\\n' \"$id\" '{second}' ;;\n    *'model/list'*) printf '{{\"id\":%s,\"result\":%s}}\\n' \"$id\" '{first}' ;;\n  esac\ndone\n",
            heard = heard.display(),
            first = MODELS_FIRST,
            second = MODELS_SECOND,
        );
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    /// `model/list`'s two pages, as codex 0.164 answers them.
    const MODELS_FIRST: &str = r#"{"data":[{"id":"gpt-6.1-sol","model":"gpt-6.1-sol","displayName":"GPT-6.1-Sol","description":"Latest workhorse model.","hidden":false,"isDefault":true,"additionalSpeedTiers":["fast"],"defaultReasoningEffort":"low","supportedReasoningEfforts":[{"reasoningEffort":"low","description":"Fast"},{"reasoningEffort":"medium","description":"Balanced"},{"reasoningEffort":"xhigh","description":"Deepest"}]},{"id":"gpt-hidden","model":"gpt-hidden","displayName":"GPT-Hidden","description":"","hidden":true,"isDefault":false,"defaultReasoningEffort":"low","supportedReasoningEfforts":[]}],"nextCursor":"page-2"}"#;
    const MODELS_SECOND: &str = r#"{"data":[{"id":"gpt-6.1-terra","model":"gpt-6.1-terra-2026","displayName":"GPT-6.1 Terra","description":"Balanced.","hidden":false,"isDefault":false,"defaultReasoningEffort":"medium","supportedReasoningEfforts":[{"reasoningEffort":"medium","description":"Balanced"},{"reasoningEffort":"max","description":"Most"}]}],"nextCursor":null}"#;

    pub(crate) const LIMITS: &str = r#"{"rateLimits":{"limitId":"codex","planType":"pro","primary":{"usedPercent":2.0,"windowDurationMins":10080,"resetsAt":1793774976},"secondary":{"usedPercent":81.0,"windowDurationMins":300,"resetsAt":1793700000},"rateLimitReachedType":null},"rateLimitsByLimitId":{"codex":{"limitId":"codex","planType":"pro","primary":{"usedPercent":2.0,"windowDurationMins":10080,"resetsAt":1793774976},"secondary":{"usedPercent":81.0,"windowDurationMins":300,"resetsAt":1793700000}},"codex_fable":{"limitId":"codex_fable","primary":{"usedPercent":99.0}}},"rateLimitResetCredits":{"availableCount":2,"credits":[{"id":"a","status":"available","expiresAt":1794000000,"grantedAt":1,"resetType":"codexRateLimits"},{"id":"b","status":"redeemed","expiresAt":1793000000,"grantedAt":1,"resetType":"codexRateLimits"},{"id":"c","status":"available","expiresAt":1795000000,"grantedAt":1,"resetType":"codexRateLimits"}]}}"#;

    #[tokio::test]
    async fn the_app_server_is_greeted_then_asked_its_rate_limits_and_models() {
        let folder = scratch("reading");
        let codex = fake_codex(&folder, LIMITS);
        let (plan, catalog) =
            read(&codex, &[], &folder.join("cwd"), Duration::from_secs(10), 7).await;
        let plan = plan.expect("an answer").expect("a reading");
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
            [
                "initialize",
                "initialized",
                "account/rateLimits/read",
                "model/list",
                "model/list"
            ]
        );
        assert_eq!(heard[4]["params"]["cursor"], "page-2");
        // Both pages, hidden models left out, each as the ACP adapter lists it.
        let catalog = catalog.expect("the models");
        let listed: Vec<(&str, &str, Vec<&str>, Option<&str>)> = catalog
            .models
            .iter()
            .map(|model| {
                (
                    model.id.as_str(),
                    model.name.as_str(),
                    model
                        .efforts
                        .iter()
                        .flatten()
                        .map(|level| level.name.as_str())
                        .collect(),
                    model.default_effort.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                (
                    "gpt-6.1-sol",
                    "6.1 Sol",
                    vec!["Low", "Medium", "Extra high"],
                    Some("low")
                ),
                (
                    "gpt-6.1-terra",
                    "6.1 Terra",
                    vec!["Medium", "Max"],
                    Some("medium")
                ),
            ]
        );
        assert_eq!(catalog.models[0].efforts.as_ref().unwrap()[2].id, "xhigh");
        assert_eq!(
            catalog.models[0].efforts.as_ref().unwrap()[0]
                .description
                .as_deref(),
            Some("Fast")
        );
        assert_eq!(catalog.default_model.as_deref(), Some("gpt-6.1-sol"));
        // A model that lists the fast tier can run fast.
        let fast: Vec<Option<bool>> = catalog.models.iter().map(|model| model.fast).collect();
        assert_eq!(fast, [Some(true), Some(false)]);
        assert_eq!(
            catalog
                .aliases
                .get("gpt-6.1-terra-2026")
                .map(String::as_str),
            Some("gpt-6.1-terra")
        );
        assert_eq!(catalog.effort_id, "reasoning_effort");
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_models_name_is_the_one_codexs_adapter_shows() {
        assert_eq!(display_name("GPT-6-Astra"), "6 Astra");
        assert_eq!(display_name("gpt-5.6-terra"), "5.6 Terra");
        assert_eq!(display_name("GPT-6.1 Terra"), "6.1 Terra");
        assert_eq!(display_name("Daybreak Blue"), "Daybreak Blue");
        assert_eq!(catalog(&[]), None);
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
