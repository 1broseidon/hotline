//! A Claude Code login's plan, read out of band the way the Agent SDK reads
//! it: the `claude` CLI started in stream-JSON mode with no prompt, asked
//! two control requests, and ended.
//!
//! `initialize` answers with the account (its subscription), and
//! `get_usage` with `skip_behaviors` answers with the plan's windows from
//! claude.ai's usage endpoint, without the scan of local transcripts. No
//! user message is ever written, so no model is called and nothing is
//! spent; no session is kept (`--no-session-persistence`), hooks are off
//! and no MCP server is started.
//!
//! ```json
//! {"type":"control_request","request_id":"hotline-2","request":{"subtype":"get_usage","skip_behaviors":true}}
//! {"type":"control_response","response":{"subtype":"success","request_id":"hotline-2","response":{
//!   "subscription_type":"max","rate_limits_available":true,"rate_limits":{
//!   "five_hour":{"utilization":38.0,"resets_at":"2026-10-29T21:00:00Z"},
//!   "seven_day":{"utilization":61.0,"resets_at":"2026-11-02T14:00:00Z"},
//!   "model_scoped":[{"display_name":"Fable","utilization":12.0,"resets_at":"…"}]}}}}
//! ```

use super::child::Probe;
use super::{FIVE_HOUR_MINUTES, WEEK_MINUTES, order, plan_name, status_of};
use crate::contract::{PlanUsage, PlanWindow};
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

/// The login's plan, read by starting `binary` in `cwd` with `env`, or
/// `None` when it could not be read in `bound` or the login has no plan
/// (an API key, a cloud provider).
pub(crate) async fn read(
    binary: &Path,
    env: &[(String, String)],
    cwd: &Path,
    bound: Duration,
    now: i64,
) -> Option<PlanUsage> {
    let mut probe = match Probe::spawn(binary, ARGS, env, ENV, cwd) {
        Ok(probe) => probe,
        Err(error) => {
            eprintln!("[plan] Claude Code could not be started to read the plan: {error}");
            return None;
        }
    };
    let asked = tokio::time::timeout(bound, ask(&mut probe)).await;
    probe.end().await;
    match asked {
        Ok(Some((account, usage))) => reading(&account, &usage, now),
        Ok(None) => None,
        Err(_) => {
            eprintln!("[plan] Claude Code did not say the plan within {bound:?}");
            None
        }
    }
}

/// The two control requests, each waited on: the account, then the usage.
async fn ask(probe: &mut Probe) -> Option<(Value, Value)> {
    probe
        .send(&request(INITIALIZE, json!({ "subtype": "initialize" })))
        .await
        .ok()?;
    let initialized = probe.answer(|line| answered(line, INITIALIZE)).await??;
    probe
        .send(&request(
            USAGE,
            json!({ "subtype": "get_usage", "skip_behaviors": true }),
        ))
        .await
        .ok()?;
    let usage = probe.answer(|line| answered(line, USAGE)).await??;
    let account = initialized.get("account").cloned().unwrap_or(Value::Null);
    Some((account, usage))
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
        assert_eq!(plan, None);
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
        assert_eq!(plan, None);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(gone(&folder), "the hung CLI is killed, not left running");
        assert!(
            heard(&folder)
                .iter()
                .all(|line| line["type"] == "control_request")
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
