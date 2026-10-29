//! A Grok Build login's plan, read out of band from xAI's billing endpoint
//! with the Grok CLI's own sign-in, as the CLI reads it for itself.
//!
//! The sign-in is `auth.json` in the Grok home (`GROK_HOME`, else
//! `~/.grok`), or `GROK_AUTH` when that holds it. Only the two entries the
//! CLI itself signs in as are read; an API-key entry, a key in
//! `XAI_API_KEY`, or any setting that points the CLI at another account or
//! deployment (the `GROK_*` overrides below, an `auth`, `grok_com_config` or
//! `endpoints` section in its config) is left to the CLI and gives no
//! reading. See docs/security.md, "A Grok Build login's plan".
//!
//! The token is read for the one request and dropped after it: it is never
//! logged, written, kept, or put in a reading or an error. An error says
//! what went wrong in words of its own, never the response's body.
//!
//! ```json
//! {"config":{"creditUsagePercent":3.0,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY",
//!   "start":"2026-10-27T18:30:44.429840+00:00","end":"2026-11-03T18:30:44.429840+00:00"}}}
//! ```

use super::{MONTH_MINUTES, WEEK_MINUTES, kind_of_minutes, status_of};
use crate::contract::{PlanUsage, PlanWindow};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const BILLING_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";

/// How long the billing request may take.
const BOUND: Duration = Duration::from_secs(10);

/// The two sign-ins the Grok CLI makes for itself: the current one, then
/// the one before it. Any other entry in the file is another deployment's.
const SIGN_INS: &[&str] = &[
    "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828",
    "https://accounts.x.ai/sign-in",
];

/// Settings that choose another account, scope or endpoint.
const OVERRIDES: &[&str] = &[
    "GROK_OIDC_ISSUER",
    "GROK_OIDC_CLIENT_ID",
    "GROK_OAUTH2_ISSUER",
    "GROK_OAUTH2_CLIENT_ID",
    "GROK_OAUTH2_PRINCIPAL_TYPE",
    "GROK_OAUTH2_PRINCIPAL_ID",
    "GROK_AUTH_PROVIDER_COMMAND",
    "GROK_LOCAL_AUTH",
    "GROK_CLI_CHAT_PROXY_BASE_URL",
    "GROK_MODELS_BASE_URL",
    "GROK_CONFIG",
    "GROK_CONFIG_PATH",
];

/// A bearer token, which says nothing of itself when printed.
struct Token(String);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(..)")
    }
}

/// The login's plan, or `None` when the login is not one the desk reads
/// for (see the module), or the read failed.
pub(crate) async fn read(
    env: &(dyn Fn(&str) -> Option<String> + Sync),
    now: i64,
) -> Option<PlanUsage> {
    read_from(env, BILLING_URL, now).await
}

pub(crate) async fn read_from(
    env: &(dyn Fn(&str) -> Option<String> + Sync),
    url: &str,
    now: i64,
) -> Option<PlanUsage> {
    let token = match credential(env) {
        Ok(Some(token)) => token,
        Ok(None) => return None,
        Err(error) => {
            eprintln!("[plan] Grok's plan was not read: {error}");
            return None;
        }
    };
    match fetch(token, url).await {
        Ok(body) => reading(&body, now),
        Err(error) => {
            eprintln!("[plan] Grok's plan was not read: {error}");
            None
        }
    }
}

/// The variable `name`, when set to something.
fn set(env: &(dyn Fn(&str) -> Option<String> + Sync), name: &str) -> Option<String> {
    env(name).filter(|value| !value.trim().is_empty())
}

/// The sign-in's token, `None` for a login the desk does not read for.
fn credential(env: &(dyn Fn(&str) -> Option<String> + Sync)) -> Result<Option<Token>, String> {
    if set(env, "XAI_API_KEY").is_some() || OVERRIDES.iter().any(|name| set(env, name).is_some()) {
        return Ok(None);
    }
    let home = set(env, "GROK_HOME")
        .map(|home| PathBuf::from(home.trim()))
        .or_else(|| {
            set(env, "HOME")
                .or_else(|| set(env, "USERPROFILE"))
                .map(|home| Path::new(&home).join(".grok"))
        })
        .ok_or("there is no home folder to find the Grok sign-in in")?;
    for config in [
        home.join("config.toml"),
        home.join("managed_config.toml"),
        home.join("requirements.toml"),
        PathBuf::from("/etc/grok/managed_config.toml"),
        PathBuf::from("/etc/grok/requirements.toml"),
    ] {
        match std::fs::read_to_string(&config) {
            Ok(text) if chooses_account(&text) => return Ok(None),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(format!("{} could not be read", config.display())),
        }
    }
    let contents = match set(env, "GROK_AUTH") {
        Some(contents) => contents,
        None => match std::fs::read_to_string(home.join("auth.json")) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("the Grok sign-in could not be read".to_string()),
        },
    };
    let entries: serde_json::Map<String, Value> =
        serde_json::from_str(&contents).map_err(|_| "the Grok sign-in is not one it can read")?;
    let Some(entry) = SIGN_INS.iter().find_map(|key| entries.get(*key)) else {
        return Ok(None);
    };
    if entry.get("auth_mode").and_then(Value::as_str) == Some("api_key") {
        return Ok(None);
    }
    Ok(entry
        .get("key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(|key| Token(key.to_string())))
}

/// Whether a Grok config has a section or key that changes the account or
/// the endpoint: `[auth]`, `[grok_com_config]`, `[endpoints]`, their
/// subsections, or a dotted key under one.
fn chooses_account(config: &str) -> bool {
    config.lines().any(|line| {
        let line = line.trim_start();
        let line = line
            .strip_prefix("[[")
            .or_else(|| line.strip_prefix('['))
            .map_or(line, str::trim_start);
        let line = line.trim_start_matches(['"', '\'']);
        ["auth", "grok_com_config", "endpoints"].iter().any(|name| {
            line.strip_prefix(name).is_some_and(|rest| {
                rest.trim_start_matches(['"', '\''])
                    .trim_start()
                    .starts_with(['.', ']', '='])
            })
        })
    })
}

/// The one GET, with the token as its bearer. The token goes no further
/// than this request.
async fn fetch(token: Token, url: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(BOUND)
        .build()
        .map_err(|_| "no HTTP client could be made".to_string())?;
    let response = client
        .get(url)
        .bearer_auth(&token.0)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "the billing endpoint did not answer in time".to_string()
            } else {
                "the billing endpoint could not be reached".to_string()
            }
        })?;
    drop(token);
    let status = response.status();
    if !status.is_success() {
        return Err(format!("the billing endpoint answered {}", status.as_u16()));
    }
    response
        .json::<Value>()
        .await
        .map_err(|_| "the billing endpoint's answer could not be read".to_string())
}

/// The reading a billing answer gives: one window, the current period's
/// credits used, weekly or monthly as the period says.
pub(crate) fn reading(body: &Value, at: i64) -> Option<PlanUsage> {
    let config = body.get("config")?;
    let period = config.get("currentPeriod");
    let period_type = period
        .and_then(|period| period.get("type"))
        .and_then(Value::as_str)
        .map(|kind| kind.trim_start_matches("USAGE_PERIOD_TYPE_"));
    let boundary = |name: &str| {
        period
            .and_then(|period| period.get(name))
            .and_then(Value::as_str)
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.timestamp_millis())
    };
    let (start, end) = (boundary("start"), boundary("end"));
    let minutes = match (period_type, start, end) {
        (_, Some(start), Some(end)) if end > start => Some((end - start) / 60_000),
        (Some("WEEKLY"), ..) => Some(WEEK_MINUTES),
        (Some("MONTHLY"), ..) => Some(MONTH_MINUTES),
        _ => None,
    };
    let used = match config.get("creditUsagePercent").and_then(Value::as_f64) {
        Some(used) if used.is_finite() => used.clamp(0.0, 100.0),
        // Grok leaves out a use of nothing, keeping the period.
        None if minutes.is_some() && start.is_some() => 0.0,
        _ => return None,
    };
    let kind = match period_type {
        Some("WEEKLY") => "seven_day".to_string(),
        Some("MONTHLY") => "monthly".to_string(),
        _ => minutes.map_or_else(|| "period".to_string(), kind_of_minutes),
    };
    Some(PlanUsage {
        windows: vec![PlanWindow {
            kind,
            used_percent: Some(used),
            resets_at: end,
            window_minutes: minutes,
            label: None,
            status: status_of(used),
        }],
        at,
        plan: None,
        banked: None,
        banked_expires_at: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::PlanStatus;
    use std::collections::HashMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const TOKEN: &str = "grok-secret-token-0123456789";

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "hotline-grok-billing-{name}-{}-{}",
            std::process::id(),
            chrono::Local::now()
                .timestamp_nanos_opt()
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn signed_in(home: &Path, auth_mode: &str) {
        std::fs::write(
            home.join("auth.json"),
            format!(
                r#"{{"https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828":{{"key":"{TOKEN}","auth_mode":"{auth_mode}","email":"someone@example.com"}},"https://other.example::x":{{"key":"another-deployment"}}}}"#
            ),
        )
        .unwrap();
    }

    fn env_of(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + Sync + use<> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        move |name| vars.get(name).cloned()
    }

    /// A billing endpoint on loopback that answers every request with
    /// `status` and `body`, and hands back each request it was sent.
    async fn endpoint(
        status: &'static str,
        body: String,
    ) -> (String, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (heard, requests) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = vec![0_u8; 8192];
                let read = socket.read(&mut request).await.unwrap_or(0);
                let _ = heard.send(String::from_utf8_lossy(&request[..read]).into_owned());
                let answer = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(answer.as_bytes()).await;
            }
        });
        (
            format!("http://{address}/v1/billing?format=credits"),
            requests,
        )
    }

    #[test]
    fn a_weekly_period_is_the_weekly_window_resetting_at_its_end() {
        let plan = reading(
            &serde_json::json!({ "config": {
                "creditUsagePercent": 3.0,
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "start": "2026-10-27T18:30:44.429840+00:00",
                    "end": "2026-11-03T18:30:44.429840+00:00"
                },
                "productUsage": [{ "product": "GrokBuild", "usagePercent": 3.0 }]
            }}),
            5,
        )
        .unwrap();
        assert_eq!(
            plan.windows,
            [PlanWindow {
                kind: "seven_day".into(),
                used_percent: Some(3.0),
                resets_at: Some(1_793_730_644_429),
                window_minutes: Some(10_080),
                label: None,
                status: PlanStatus::Allowed,
            }]
        );
        assert_eq!(plan.at, 5);
        // A period with nothing used yet, which Grok says by leaving the
        // figure out.
        let plan = reading(
            &serde_json::json!({ "config": { "currentPeriod": {
                "type": "USAGE_PERIOD_TYPE_MONTHLY",
                "start": "2026-10-01T00:00:00Z",
                "end": "2026-10-31T00:00:00Z"
            }}}),
            5,
        )
        .unwrap();
        assert_eq!(plan.windows[0].kind, "monthly");
        assert_eq!(plan.windows[0].used_percent, Some(0.0));
        assert_eq!(plan.windows[0].window_minutes, Some(30 * 24 * 60));
        // Nothing to go on is no reading, never an empty one.
        assert_eq!(reading(&serde_json::json!({ "config": {} }), 5), None);
        assert_eq!(reading(&serde_json::json!({}), 5), None);
    }

    #[tokio::test]
    async fn the_sign_ins_token_is_the_bearer_of_one_billing_request() {
        let home = scratch("read");
        signed_in(&home, "oidc");
        let (url, mut requests) = endpoint(
            "200 OK",
            r#"{"config":{"creditUsagePercent":41.5,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","start":"2026-10-27T00:00:00Z","end":"2026-11-03T00:00:00Z"}}}"#.to_string(),
        )
        .await;
        let home_text = home.to_string_lossy().into_owned();
        let env = env_of(&[("GROK_HOME", &home_text), ("HOME", "/nobody")]);
        let plan = read_from(&env, &url, 9).await.expect("a reading");
        assert_eq!(plan.windows[0].used_percent, Some(41.5));
        let request = requests.recv().await.unwrap();
        assert!(request.starts_with("GET /v1/billing?format=credits "));
        assert!(request.to_ascii_lowercase().contains(&format!(
            "authorization: bearer {}",
            TOKEN.to_ascii_lowercase()
        )));
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn another_account_or_deployment_or_an_api_key_is_left_to_the_cli() {
        let home = scratch("bail");
        signed_in(&home, "oidc");
        let home_text = home.to_string_lossy().into_owned();
        // Nothing would answer here: any request would fail the read.
        let url = "http://127.0.0.1:9/never";
        for (name, value) in [
            ("XAI_API_KEY", "xai-key"),
            ("GROK_CLI_CHAT_PROXY_BASE_URL", "https://proxy.example"),
            ("GROK_OIDC_ISSUER", "https://issuer.example"),
            ("GROK_CONFIG", "/etc/other.toml"),
        ] {
            let env = env_of(&[("GROK_HOME", &home_text), (name, value)]);
            assert!(credential(&env).unwrap().is_none(), "{name} bails out");
            assert_eq!(read_from(&env, url, 1).await, None);
        }
        let env = env_of(&[("GROK_HOME", &home_text)]);
        assert!(credential(&env).unwrap().is_some());
        for section in [
            "[auth]\nissuer = \"x\"",
            "[endpoints]\nchat = \"x\"",
            "[grok_com_config.extra]\n",
            "auth.issuer = \"x\"",
            "[\"auth\"]\n",
        ] {
            std::fs::write(
                home.join("config.toml"),
                format!("[cli]\nx = 1\n{section}\n"),
            )
            .unwrap();
            assert!(credential(&env).unwrap().is_none(), "{section} bails out");
        }
        std::fs::write(
            home.join("config.toml"),
            "[cli]\nauthor = 1\n[ui]\nendpoints_shown = 1\n",
        )
        .unwrap();
        assert!(credential(&env).unwrap().is_some());
        // An API-key sign-in has no plan to read.
        signed_in(&home, "api_key");
        assert!(credential(&env).unwrap().is_none());
        // Nor does a file holding only other deployments' sign-ins.
        std::fs::write(
            home.join("auth.json"),
            r#"{"https://other.example::x":{"key":"k"}}"#,
        )
        .unwrap();
        assert!(credential(&env).unwrap().is_none());
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn a_failed_request_never_says_the_token() {
        // An endpoint that refuses, echoing the token back in its body.
        let (url, _requests) = endpoint(
            "401 Unauthorized",
            format!(r#"{{"error":"bad token {TOKEN}"}}"#),
        )
        .await;
        let error = fetch(Token(TOKEN.to_string()), &url).await.unwrap_err();
        assert_eq!(error, "the billing endpoint answered 401");
        // One that answers with something that is not JSON, holding it too.
        let (url, _requests) = endpoint("200 OK", format!("not json {TOKEN}")).await;
        let error = fetch(Token(TOKEN.to_string()), &url).await.unwrap_err();
        assert!(!error.contains(TOKEN), "{error}");
        // One that cannot be reached.
        let error = fetch(Token(TOKEN.to_string()), "http://127.0.0.1:9/billing")
            .await
            .unwrap_err();
        assert!(!error.contains(TOKEN), "{error}");
        assert!(!format!("{:?}", Token(TOKEN.to_string())).contains(TOKEN));
        // A sign-in that cannot be read says so without its contents.
        let home = scratch("unreadable");
        let home_text = home.to_string_lossy().into_owned();
        let env = env_of(&[
            ("GROK_HOME", &home_text),
            ("GROK_AUTH", &format!("{{not json {TOKEN}")),
        ]);
        let error = credential(&env).unwrap_err();
        assert!(!error.contains(TOKEN), "{error}");
        let _ = std::fs::remove_dir_all(home);
    }
}
