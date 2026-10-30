//! A Claude Code login's plan and its models, read out of band the way the
//! Agent SDK reads them: the `claude` CLI started in stream-JSON mode with no
//! prompt, asked two control requests, and ended.
//!
//! `initialize` answers with the account (its subscription) and every model
//! with its effort levels, and `get_usage` with `skip_behaviors` answers
//! with the plan's windows from claude.ai's usage endpoint, without the scan
//! of local transcripts. No user message is ever written, so no model is
//! called and nothing is spent; no session is kept
//! (`--no-session-persistence`), hooks are off and no MCP server is started.
//!
//! ```json
//! {"type":"control_response","response":{"subtype":"success","request_id":"hotline-1","response":{
//!   "models":[{"value":"opus","resolvedModel":"claude-opus-5-5","displayName":"Opus 5.5",
//!     "description":"…","supportsEffort":true,"supportedEffortLevels":["low","medium","high","xhigh","max"]}],
//!   "account":{"subscriptionType":"Claude Max"}}}}
//! {"type":"control_request","request_id":"hotline-2","request":{"subtype":"get_usage","skip_behaviors":true}}
//! {"type":"control_response","response":{"subtype":"success","request_id":"hotline-2","response":{
//!   "subscription_type":"max","rate_limits_available":true,"rate_limits":{
//!   "five_hour":{"utilization":38.0,"resets_at":"2026-10-29T21:00:00Z"},
//!   "seven_day":{"utilization":61.0,"resets_at":"2026-11-02T14:00:00Z"},
//!   "model_scoped":[{"display_name":"Fable","utilization":12.0,"resets_at":"…"}]}}}}
//! ```
//!
//! A model's `value` is the id Claude Code's ACP adapter lists it by and
//! takes in `session/set_model`, and the adapter's effort picker is
//! `effort`: Default, then each level the model supports, named as the
//! adapter names it ("Xhigh"). So the catalogue is the adapter's own.

use super::child::Probe;
use super::{FIVE_HOUR_MINUTES, LoginReading, WEEK_MINUTES, order, plan_name, status_of};
use crate::contract::{ConfigChoice, PlanUsage, PlanWindow};
use crate::driver::acp::catalog::Catalog;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

/// What the SDK passes to stream over stdio, and what keeps the probe from
/// leaving anything behind: no transcript, no hooks, no MCP servers.
const ARGS: &[&str] = &[
    "--output-format",
    "stream-json",
    "--verbose",
    "--input-format",
    "stream-json",
    "--setting-sources",
    "user,project,local",
    "--strict-mcp-config",
    "--no-session-persistence",
    "--settings",
    r#"{"disableAllHooks":true}"#,
];

/// claude.ai's own MCP servers are found outside the settings files, and an
/// IDE is looked for unless told not to.
const ENV: &[(&str, &str)] = &[
    ("ENABLE_CLAUDEAI_MCP_SERVERS", "false"),
    ("CLAUDE_CODE_AUTO_CONNECT_IDE", "0"),
    ("CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL", "1"),
];

const INITIALIZE: &str = "hotline-1";
const USAGE: &str = "hotline-2";

/// The login's plan and its models, read by starting `binary` in `cwd`
/// with `env`. The plan is `None` when it could not be read in `bound` or
/// the login has no plan (an API key, a cloud provider); the catalogue is
/// there whenever `initialize` answered.
pub(crate) async fn read(
    binary: &Path,
    env: &[(String, String)],
    cwd: &Path,
    bound: Duration,
    now: i64,
) -> LoginReading {
    let mut probe = match Probe::spawn(binary, ARGS, env, ENV, cwd) {
        Ok(probe) => probe,
        Err(error) => {
            eprintln!("[plan] Claude Code could not be started to read the plan: {error}");
            return LoginReading::default();
        }
    };
    let mut answers = Answers::default();
    let asked = tokio::time::timeout(bound, ask(&mut probe, &mut answers)).await;
    probe.end().await;
    if asked.is_err() {
        eprintln!("[plan] Claude Code did not say the plan within {bound:?}");
    }
    let account = answers
        .initialized
        .as_ref()
        .and_then(|initialized| initialized.get("account"))
        .cloned()
        .unwrap_or(Value::Null);
    LoginReading {
        plan: answers
            .usage
            .as_ref()
            .and_then(|usage| reading(&account, usage, now)),
        catalog: answers.initialized.as_ref().and_then(catalog),
    }
}

/// What the CLI answered, kept as it comes so a later request that runs
/// out of time leaves an earlier answer standing.
#[derive(Default)]
struct Answers {
    initialized: Option<Value>,
    usage: Option<Value>,
}

/// The two control requests, each waited on: the account and models, then
/// the usage.
async fn ask(probe: &mut Probe, answers: &mut Answers) -> Option<()> {
    probe
        .send(&request(INITIALIZE, json!({ "subtype": "initialize" })))
        .await
        .ok()?;
    answers.initialized = Some(probe.answer(|line| answered(line, INITIALIZE)).await??);
    probe
        .send(&request(
            USAGE,
            json!({ "subtype": "get_usage", "skip_behaviors": true }),
        ))
        .await
        .ok()?;
    answers.usage = Some(probe.answer(|line| answered(line, USAGE)).await??);
    Some(())
}

fn request(id: &str, request: Value) -> Value {
    json!({ "type": "control_request", "request_id": id, "request": request })
}

/// The answer to request `id`, when this line is it: `Some(Some(_))` for a
/// success, `Some(None)` for an error, `None` for any other line.
fn answered(line: &Value, id: &str) -> Option<Option<Value>> {
    if line.get("type")?.as_str()? != "control_response" {
        return None;
    }
    let response = line.get("response")?;
    if response.get("request_id")?.as_str()? != id {
        return None;
    }
    Some(
        (response.get("subtype").and_then(Value::as_str) == Some("success"))
            .then(|| response.get("response").cloned().unwrap_or(Value::Null)),
    )
}

/// The reading `get_usage` gives, with the plan named from it or, failing
/// that, from the account `initialize` gave. `None` when the plan's limits
/// do not apply to this login.
pub(crate) fn reading(account: &Value, usage: &Value, at: i64) -> Option<PlanUsage> {
    if usage.get("rate_limits_available").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let limits = usage.get("rate_limits")?.as_object()?;
    let mut windows = Vec::new();
    for (kind, minutes) in [
        ("five_hour", FIVE_HOUR_MINUTES),
        ("seven_day", WEEK_MINUTES),
        ("seven_day_opus", WEEK_MINUTES),
        ("seven_day_sonnet", WEEK_MINUTES),
    ] {
        if let Some(window) = limits
            .get(kind)
            .and_then(|one| window(kind, minutes, one, None))
        {
            windows.push(window);
        }
    }
    for scoped in limits
        .get("model_scoped")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = scoped.get("display_name").and_then(Value::as_str) else {
            continue;
        };
        let kind = format!("seven_day_{}", slug(name));
        let label = format!("Weekly · {name}");
        if let Some(window) = window(&kind, WEEK_MINUTES, scoped, Some(label)) {
            windows.retain(|one| one.kind != window.kind);
            windows.push(window);
        }
    }
    windows.sort_by_key(|one| order(&one.kind));
    let plan = usage
        .get("subscription_type")
        .and_then(Value::as_str)
        .or_else(|| account.get("subscriptionType").and_then(Value::as_str))
        .and_then(plan_name);
    Some(PlanUsage {
        windows,
        at,
        plan,
        banked: None,
        banked_expires_at: None,
    })
}

/// The models `initialize` lists, each by the id the ACP adapter takes,
/// with the levels the adapter's effort picker offers for it: Default, then
/// each level the model supports. `None` when it lists none.
pub(crate) fn catalog(initialized: &Value) -> Option<Catalog> {
    let listed = initialized.get("models")?.as_array()?;
    let mut catalog = Catalog {
        effort_id: "effort".to_string(),
        effort_name: "Effort".to_string(),
        ..Catalog::default()
    };
    for model in listed {
        let Some(id) = model.get("value").and_then(Value::as_str) else {
            continue;
        };
        let text = |key: &str| {
            model
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
        };
        let resolved = text("resolvedModel");
        let levels: Vec<ConfigChoice> =
            if model.get("supportsEffort").and_then(Value::as_bool) == Some(true) {
                model
                    .get("supportedEffortLevels")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|level| effort_level(level, &effort_name(level)))
                    .collect()
            } else {
                Vec::new()
            };
        let has_levels = !levels.is_empty();
        let efforts = if has_levels {
            std::iter::once(effort_level(DEFAULT, "Default"))
                .chain(levels)
                .collect()
        } else {
            Vec::new()
        };
        // The adapter describes its default by the model it runs.
        let description = if id == DEFAULT {
            listed
                .iter()
                .find(|other| {
                    other.get("value").and_then(Value::as_str) != Some(DEFAULT)
                        && other.get("resolvedModel").and_then(Value::as_str) == resolved
                })
                .and_then(|other| other.get("displayName").and_then(Value::as_str))
                .or(resolved)
        } else {
            text("description")
        };
        if let Some(resolved) = resolved
            && resolved != id
            && id != DEFAULT
        {
            catalog
                .aliases
                .entry(resolved.to_string())
                .or_insert_with(|| id.to_string());
        }
        catalog.models.push(ConfigChoice {
            id: id.to_string(),
            name: text("displayName").unwrap_or(id).to_string(),
            description: description.map(str::to_string),
            group: None,
            account: None,
            efforts: Some(crate::models::tidy_efforts(efforts)),
            default_effort: has_levels.then(|| DEFAULT.to_string()),
            // Only the models that can run fast say so.
            fast: Some(model.get("supportsFastMode").and_then(Value::as_bool) == Some(true)),
        });
    }
    if catalog.models.is_empty() {
        return None;
    }
    catalog.default_model = catalog
        .models
        .iter()
        .any(|model| model.id == DEFAULT)
        .then(|| DEFAULT.to_string());
    Some(catalog)
}

/// The adapter's "use what the CLI would" model and effort.
const DEFAULT: &str = "default";

fn effort_level(id: &str, name: &str) -> ConfigChoice {
    ConfigChoice {
        id: id.to_string(),
        name: name.to_string(),
        description: None,
        group: None,
        account: None,
        efforts: None,
        default_effort: None,
        fast: None,
    }
}

/// A level as the adapter names it: `xhigh` → "Xhigh", `extra_high` → "Extra High".
fn effort_name(level: &str) -> String {
    level
        .split(['_', '-'])
        .map(|part| {
            let mut letters = part.chars();
            letters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(letters).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One window, when it says how much of it is used (0 to 100).
fn window(kind: &str, minutes: i64, limit: &Value, label: Option<String>) -> Option<PlanWindow> {
    let used = limit.get("utilization")?.as_f64()?.clamp(0.0, 100.0);
    let resets_at = limit
        .get("resets_at")
        .and_then(Value::as_str)
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.timestamp_millis());
    Some(PlanWindow {
        kind: kind.to_string(),
        used_percent: Some(used),
        resets_at,
        window_minutes: Some(minutes),
        label,
        status: status_of(used),
    })
}

/// A model's name as a window kind: "Fable" → `fable`, "Opus 5" → `opus_5`.
fn slug(name: &str) -> String {
    name.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::contract::PlanStatus;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "hotline-claude-cli-{name}-{}-{}",
            std::process::id(),
            chrono::Local::now()
                .timestamp_nanos_opt()
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// A stand-in `claude`: it writes every line it is sent to `heard`, its
    /// process id to `pid`, and answers each control request by its id with
    /// what the test gives, or with nothing at all when `answers` is empty.
    fn fake_claude(folder: &Path, initialize: &str, usage: &str) -> PathBuf {
        let script = folder.join("claude");
        let heard = folder.join("heard");
        let pid = folder.join("pid");
        let body = if initialize.is_empty() {
            format!(
                "#!/bin/sh\necho $$ > '{pid}'\nwhile IFS= read -r line; do printf '%s\\n' \"$line\" >> '{heard}'; done\nsleep 30\n",
                pid = pid.display(),
                heard = heard.display(),
            )
        } else {
            format!(
                "#!/bin/sh\necho $$ > '{pid}'\necho '{{\"type\":\"system\",\"subtype\":\"hello\"}}'\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> '{heard}'\n  case \"$line\" in\n    *'\"{INITIALIZE}\"'*) printf '%s\\n' '{{\"type\":\"control_response\",\"response\":{{\"subtype\":\"success\",\"request_id\":\"{INITIALIZE}\",\"response\":{initialize}}}}}' ;;\n    *'\"{USAGE}\"'*) printf '%s\\n' '{{\"type\":\"control_response\",\"response\":{{\"subtype\":\"success\",\"request_id\":\"{USAGE}\",\"response\":{usage}}}}}' ;;\n  esac\ndone\n",
                pid = pid.display(),
                heard = heard.display(),
            )
        };
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    fn heard(folder: &Path) -> Vec<Value> {
        std::fs::read_to_string(folder.join("heard"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn gone(folder: &Path) -> bool {
        let pid: i32 = std::fs::read_to_string(folder.join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // Safety: signal 0 only asks whether the process exists.
        unsafe { libc::kill(pid, 0) != 0 }
    }

    const ACCOUNT: &str =
        r#"{"account":{"email":"someone@example.com","subscriptionType":"Claude Max"}}"#;
    const USAGE_ANSWER: &str = r#"{"subscription_type":"max","rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":38,"resets_at":"2026-10-29T21:00:00Z"},"seven_day":{"utilization":61.4,"resets_at":"2026-11-02T14:00:00+00:00"},"seven_day_opus":null,"model_scoped":[{"display_name":"Fable","utilization":85,"resets_at":"2026-11-02T14:00:00Z"}],"extra_usage":null},"behaviors":null}"#;

    #[tokio::test]
    async fn the_cli_is_asked_its_account_and_usage_and_nothing_else() {
        let folder = scratch("reading");
        let claude = fake_claude(&folder, ACCOUNT, USAGE_ANSWER);
        let plan = read(
            &claude,
            &[],
            &folder.join("cwd"),
            Duration::from_secs(10),
            7,
        )
        .await
        .plan
        .expect("a reading");
        assert_eq!(plan.at, 7);
        assert_eq!(plan.plan.as_deref(), Some("Max"));
        assert_eq!(
            plan.windows,
            [
                PlanWindow {
                    kind: "five_hour".into(),
                    used_percent: Some(38.0),
                    resets_at: Some(1_793_307_600_000),
                    window_minutes: Some(300),
                    label: None,
                    status: PlanStatus::Allowed,
                },
                PlanWindow {
                    kind: "seven_day".into(),
                    used_percent: Some(61.4),
                    resets_at: Some(1_793_628_000_000),
                    window_minutes: Some(10_080),
                    label: None,
                    status: PlanStatus::Allowed,
                },
                PlanWindow {
                    kind: "seven_day_fable".into(),
                    used_percent: Some(85.0),
                    resets_at: Some(1_793_628_000_000),
                    window_minutes: Some(10_080),
                    label: Some("Weekly · Fable".into()),
                    status: PlanStatus::Warning,
                },
            ]
        );
        // Two control requests, and never a user message.
        let said = heard(&folder);
        assert_eq!(said.len(), 2);
        assert!(said.iter().all(|line| line["type"] == "control_request"));
        assert_eq!(said[0]["request"]["subtype"], "initialize");
        assert_eq!(said[1]["request"]["subtype"], "get_usage");
        assert_eq!(said[1]["request"]["skip_behaviors"], true);
        assert!(gone(&folder), "the CLI exits once its stdin closes");
        let _ = std::fs::remove_dir_all(folder);
    }

    #[tokio::test]
    async fn a_login_whose_plan_limits_do_not_apply_has_no_reading() {
        let folder = scratch("api-key");
        let claude = fake_claude(
            &folder,
            r#"{"account":{"apiKeySource":"ANTHROPIC_API_KEY"}}"#,
            r#"{"subscription_type":null,"rate_limits_available":false,"rate_limits":null}"#,
        );
        let plan = read(
            &claude,
            &[],
            &folder.join("cwd"),
            Duration::from_secs(10),
            7,
        )
        .await;
        assert_eq!(plan, LoginReading::default());
        assert!(
            heard(&folder)
                .iter()
                .all(|line| line["type"] == "control_request")
        );
        let _ = std::fs::remove_dir_all(folder);
    }

    #[tokio::test]
    async fn a_cli_that_never_answers_is_killed_when_its_time_is_up() {
        let folder = scratch("hung");
        let claude = fake_claude(&folder, "", "");
        let started = std::time::Instant::now();
        let plan = read(
            &claude,
            &[],
            &folder.join("cwd"),
            Duration::from_millis(500),
            7,
        )
        .await;
        assert_eq!(plan, LoginReading::default());
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(gone(&folder), "the hung CLI is killed, not left running");
        assert!(
            heard(&folder)
                .iter()
                .all(|line| line["type"] == "control_request")
        );
        let _ = std::fs::remove_dir_all(folder);
    }

    /// What `initialize` says of the models, as Claude Code 2.x answers it.
    const MODELS: &str = r#"{"account":{"subscriptionType":"Claude Max"},"models":[{"value":"default","resolvedModel":"claude-opus-5-5","displayName":"Default (recommended)","description":"Opus 5.5 · Best for everyday, complex tasks","supportsEffort":true,"supportedEffortLevels":["low","medium","high","xhigh","max"]},{"value":"opus","resolvedModel":"claude-opus-5-5","displayName":"Opus 5.5","description":"For complex work and everyday tasks","supportsEffort":true,"supportsFastMode":true,"supportedEffortLevels":["low","medium","high","xhigh","max"]},{"value":"claude-haiku-4-5-20251001","resolvedModel":"claude-haiku-4-5-20251001","displayName":"Haiku 4.5","description":"Fastest for quick answers"},{"value":"claude-sonnet-4-6","resolvedModel":"claude-sonnet-4-6","displayName":"Sonnet 4.6","description":"Efficient for routine tasks","supportsEffort":true,"supportedEffortLevels":["low","medium","high","max"]}]}"#;

    /// The same `initialize` names the models, each with the levels the ACP
    /// adapter's effort picker offers for it and by the id it takes, so one
    /// CLI started for the plan answers both, and a plan that cannot be read
    /// still leaves the models.
    #[tokio::test]
    async fn initialize_lists_every_model_with_its_effort_levels() {
        let folder = scratch("models");
        let claude = fake_claude(
            &folder,
            MODELS,
            r#"{"subscription_type":null,"rate_limits_available":false}"#,
        );
        let read = read(
            &claude,
            &[],
            &folder.join("cwd"),
            Duration::from_secs(10),
            7,
        )
        .await;
        assert_eq!(read.plan, None);
        let catalog = read.catalog.expect("the models");
        assert_eq!(heard(&folder).len(), 2, "one CLI, two control requests");
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
        let every = vec!["Default", "Low", "Medium", "High", "Extra high", "Max"];
        assert_eq!(
            listed,
            [
                (
                    "default",
                    "Default (recommended)",
                    every.clone(),
                    Some("default")
                ),
                ("opus", "Opus 5.5", every, Some("default")),
                ("claude-haiku-4-5-20251001", "Haiku 4.5", vec![], None),
                (
                    "claude-sonnet-4-6",
                    "Sonnet 4.6",
                    vec!["Default", "Low", "Medium", "High", "Max"],
                    Some("default")
                ),
            ]
        );
        assert_eq!(catalog.models[0].description.as_deref(), Some("Opus 5.5"));
        assert_eq!(catalog.models[1].efforts.as_ref().unwrap()[4].id, "xhigh");
        // Only a model that says it can run fast can.
        let fast: Vec<Option<bool>> = catalog.models.iter().map(|model| model.fast).collect();
        assert_eq!(fast, [Some(false), Some(true), Some(false), Some(false)]);
        assert_eq!(catalog.default_model.as_deref(), Some("default"));
        // The resolved id of a model finds it under the id the adapter takes.
        assert_eq!(
            catalog.aliases.get("claude-opus-5-5").map(String::as_str),
            Some("opus")
        );
        assert_eq!(
            (catalog.effort_id.as_str(), catalog.effort_name.as_str()),
            ("effort", "Effort")
        );
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_plan_is_named_from_the_usage_or_else_the_account() {
        let usage = serde_json::json!({
            "subscription_type": null,
            "rate_limits_available": true,
            "rate_limits": { "five_hour": { "utilization": 1, "resets_at": null } }
        });
        let account = serde_json::json!({ "subscriptionType": "Claude Pro" });
        let plan = reading(&account, &usage, 1).unwrap();
        assert_eq!(plan.plan.as_deref(), Some("Pro"));
        assert_eq!(plan.windows[0].resets_at, None);
        assert_eq!(slug("Opus 5.1"), "opus_5_1");
    }
}
