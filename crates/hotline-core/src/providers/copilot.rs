//! GitHub Copilot's two routes, over one signing transport.
//!
//! Rig's Copilot dialect sends every model except the Codex family to
//! `/chat/completions`. Copilot's own model list says which models answer
//! only on `/responses` (the newer OpenAI reasoning models and Grok, at the
//! time of writing), and a chat request to one of those is refused with
//! `unsupported_api_for_model`. Discovery records each model's endpoints
//! beside the login; a model that lists `/responses` and not
//! `/chat/completions` is then sent to `/responses` from here. Either way
//! the request carries the editor headers Rig's dialect writes, and is
//! signed with the login's live token as it goes ([`super::outgoing::Outgoing`]).
//! Sign-in and token refresh are Rig's, through the login's shared
//! authenticator.

use super::discovery::{self, DiscoveryHttp, ListedModel};
use crate::models::{self, ModelKind, Said};
use bytes::Bytes;
use rig::http_client::HttpClientExt;
use rig::providers::chatgpt::auth::AuthContext;
use rig::providers::copilot;
use rig::providers::openai::{OpenAIConfig, Route};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

const API_BASE: &str = "https://api.githubcopilot.com";
/// The per-model endpoints Copilot advertised, beside the login.
const ENDPOINTS_FILE: &str = "endpoints.json";
const RESPONSES: &str = "/responses";
const CHAT_COMPLETIONS: &str = "/chat/completions";
const MAX_ENDPOINTS: usize = 16;
const MAX_ENDPOINT_LEN: usize = 64;
/// How long a turn waits for the model list it fetches for itself.
const ENSURE_TIMEOUT: Duration = Duration::from_secs(10);
/// How soon a login whose list could not be fetched is tried again.
const ENSURE_RETRY: Duration = Duration::from_mins(5);

// The identity Rig presents to Copilot. Copilot refuses a request that does
// not look like an editor's, so the model list, which Rig does not fetch
// here, asks the same way.
const INTEGRATION_ID: &str = "vscode-chat";
const EDITOR_VERSION: &str = "vscode/1.107.0";
const EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
const USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
const API_VERSION: &str = "2025-04-01";

/// A live Copilot API token and the base it was issued for.
pub(crate) struct Session {
    token: String,
    api_base: String,
}

/// A usable token, refreshed by Rig when the stored one has expired. With no
/// sign-in on disk this fails at once, naming the sign-in, and never starts
/// a device flow.
pub(crate) async fn session(token_dir: &Path) -> Result<Session, String> {
    let context = super::rig_login::copilot(token_dir)
        .auth_context(&super::rig_login::http())
        .await
        .map_err(|error| error.to_string())?;
    session_of(context)
}

/// The session a sign-in's context describes. A host the token record names
/// is used only when it is a plain server URL; anything else falls back to
/// Copilot's own.
fn session_of(context: AuthContext) -> Result<Session, String> {
    let token = context.api_key.expose().to_string();
    if token.trim().is_empty() {
        return Err(
            "GitHub Copilot sign-in required. Sign in again under Settings → Providers.".into(),
        );
    }
    let api_base = context
        .api_base
        .and_then(|api| super::custom::server_url(&api).ok())
        .unwrap_or_else(|| API_BASE.to_string());
    Ok(Session { token, api_base })
}

// ---- Discovery -----------------------------------------------------------

#[derive(Deserialize)]
struct ListModelsResponse {
    data: Vec<ListModelEntry>,
}

#[derive(Deserialize)]
struct ListModelEntry {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    supported_endpoints: Option<Vec<String>>,
    #[serde(default)]
    capabilities: Option<Capabilities>,
}

#[derive(Deserialize)]
struct Capabilities {
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

/// The account's models from `GET /models`, with each model's endpoints
/// recorded beside the login so a turn can pick its route without asking
/// Copilot again.
pub(crate) async fn list_models(token_dir: &Path) -> Result<Vec<ListedModel>, String> {
    let session = session(token_dir).await?;
    let (models, endpoints) = fetch_models(&session).await?;
    write_endpoints(token_dir, &endpoints)?;
    Ok(models)
}

pub(crate) type Endpoints = BTreeMap<String, Vec<String>>;

async fn fetch_models(session: &Session) -> Result<(Vec<ListedModel>, Endpoints), String> {
    let failed = || {
        "Could not discover models. Check this connection and try again; the previous list is unchanged.".to_string()
    };
    let mut request = http::Request::get(format!("{}/models", session.api_base));
    for (name, value) in headers(&session.token) {
        request = request.header(name, value);
    }
    let request = request.body(Bytes::new()).map_err(|_| failed())?;
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        DiscoveryHttp::new()?.send::<Bytes, Vec<u8>>(request),
    )
    .await
    .map_err(|_| "Model discovery timed out; the previous list is unchanged.".to_string())?
    .map_err(|_| failed())?;
    let body = response.into_body().await.map_err(|_| failed())?;
    let listed: ListModelsResponse = serde_json::from_slice(&body).map_err(|_| failed())?;
    let mut endpoints = Endpoints::new();
    let mut models = Vec::new();
    for entry in listed.data {
        let said = entry
            .capabilities
            .and_then(|caps| caps.kind)
            .map_or(Said::Nothing, Said::Type);
        if models::kind("github-copilot", &entry.id, &said) != ModelKind::Chat {
            continue;
        }
        if let Some(offered) = entry
            .supported_endpoints
            .filter(|offered| !offered.is_empty())
        {
            endpoints.insert(entry.id.clone(), offered);
        }
        models.push(ListedModel {
            id: entry.id,
            name: entry.name,
            context_limit: None,
            output_limit: None,
        });
    }
    discovery::validate(&models)?;
    validate_endpoints(&endpoints)?;
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    Ok((models, endpoints))
}

fn validate_endpoints(endpoints: &Endpoints) -> Result<(), String> {
    discovery::validate_ids(&endpoints.keys().cloned().collect::<Vec<_>>())?;
    let sound = endpoints.values().all(|offered| {
        offered.len() <= MAX_ENDPOINTS
            && offered.iter().all(|endpoint| {
                !endpoint.is_empty()
                    && endpoint.len() <= MAX_ENDPOINT_LEN
                    && endpoint.bytes().all(|byte| byte.is_ascii_graphic())
            })
    });
    if sound {
        Ok(())
    } else {
        Err("The provider returned invalid model metadata.".into())
    }
}

fn write_endpoints(token_dir: &Path, endpoints: &Endpoints) -> Result<(), String> {
    let mut body = serde_json::to_vec(endpoints).map_err(|error| error.to_string())?;
    body.push(b'\n');
    crate::vault::write_beside_login(token_dir, ENDPOINTS_FILE, &body)
        .map_err(|_| "The GitHub Copilot model list could not be recorded.".to_string())
}

/// A login made before endpoints were recorded has none on file, and every
/// Responses-only model on it would keep failing until someone pressed
/// Refresh. The first Copilot turn on such a login fetches the list itself.
/// A failed fetch leaves the turn on Rig's route and is not retried for a
/// few minutes, so a Copilot outage does not add a wait to every turn.
pub(crate) async fn ensure_endpoints(token_dir: &Path) {
    if token_dir.join(ENDPOINTS_FILE).symlink_metadata().is_ok() {
        return;
    }
    static TRIED: Mutex<Option<HashMap<PathBuf, Instant>>> = Mutex::new(None);
    {
        let mut tried = TRIED.lock().unwrap_or_else(PoisonError::into_inner);
        let tried = tried.get_or_insert_with(HashMap::new);
        let now = Instant::now();
        if tried
            .get(token_dir)
            .is_some_and(|last| now.duration_since(*last) < ENSURE_RETRY)
        {
            return;
        }
        tried.insert(token_dir.to_path_buf(), now);
    }
    let _ = tokio::time::timeout(ENSURE_TIMEOUT, list_models(token_dir)).await;
}

fn read_endpoints(token_dir: &Path) -> Option<Endpoints> {
    let bytes = crate::vault::read_model_file(&token_dir.join(ENDPOINTS_FILE)).ok()?;
    let endpoints: Endpoints = serde_json::from_slice(&bytes).ok()?;
    validate_endpoints(&endpoints).ok()?;
    Some(endpoints)
}

/// Whether this model is driven over `/responses` here rather than on Rig's
/// route: the account's list offered `/responses` and not
/// `/chat/completions`. A model with both, or one the list never named,
/// stays on Rig's route, which already sends the Codex family to
/// `/responses` on its own.
pub(crate) fn wants_responses(token_dir: &Path, model: &str) -> bool {
    read_endpoints(token_dir)
        .and_then(|endpoints| endpoints.get(model).cloned())
        .is_some_and(|offered| {
            offered.iter().any(|endpoint| endpoint == RESPONSES)
                && !offered.iter().any(|endpoint| endpoint == CHAT_COMPLETIONS)
        })
}

// ---- The model ------------------------------------------------------------

/// `model` on the login in `token_dir`: on `/responses` when `responses`
/// says so, otherwise on the route Rig's Copilot dialect picks for it.
pub(crate) async fn model(
    token_dir: &Path,
    model: &str,
    responses: bool,
) -> Result<rig::DynModel<rig::operation::Completion>, String> {
    let session = session(token_dir).await?;
    let config =
        OpenAIConfig::with_key(&copilot::DIALECT, session.token).with_base_url(session.api_base);
    let config = if responses {
        config.with_route(Route::Responses)
    } else {
        config
    };
    let transport = super::outgoing::Outgoing::signed(
        super::rig_login::copilot(token_dir),
        if responses {
            drop_unique_items
        } else {
            std::convert::identity
        },
    )?;
    // Copilot's chat route ends an empty reply without a finish reason.
    let transport = if responses {
        transport
    } else {
        transport.mending_streams(super::outgoing::done_is_stop)
    };
    Ok(config.connect(transport).completion(model).erase())
}

/// The body with `uniqueItems` gone from every tool's parameters.
///
/// Copilot's `/responses` holds each tool's schema to OpenAI's strict subset
/// whether or not the tool asked for strict mode, and answers a schema with
/// `uniqueItems` in it with a 400, so every turn of a GPT model there failed
/// before it began. A list a tool asks to keep unique still reaches the tool,
/// which reads it as a set.
pub(crate) fn drop_unique_items(body: Bytes) -> Bytes {
    fn drop_in(schema: &mut serde_json::Value) -> bool {
        match schema {
            serde_json::Value::Object(fields) => {
                let dropped = fields.remove("uniqueItems").is_some();
                fields
                    .values_mut()
                    .fold(dropped, |dropped, value| drop_in(value) | dropped)
            }
            serde_json::Value::Array(values) => values
                .iter_mut()
                .fold(false, |dropped, value| drop_in(value) | dropped),
            _ => false,
        }
    }
    let Ok(mut request) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return body;
    };
    let Some(tools) = request
        .get_mut("tools")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return body;
    };
    let changed = tools.iter_mut().fold(false, |changed, tool| {
        tool.get_mut("parameters").is_some_and(drop_in) | changed
    });
    if !changed {
        return body;
    }
    serde_json::to_vec(&request).map_or(body, Bytes::from)
}

/// The headers Copilot's model list is asked with: the editor identity
/// Rig's dialect sends on a model request, for a request a person started.
fn headers(token: &str) -> Vec<(&'static str, String)> {
    vec![
        ("authorization", format!("Bearer {token}")),
        ("copilot-integration-id", INTEGRATION_ID.to_string()),
        ("editor-version", EDITOR_VERSION.to_string()),
        ("editor-plugin-version", EDITOR_PLUGIN_VERSION.to_string()),
        ("user-agent", USER_AGENT.to_string()),
        ("openai-intent", "conversation-panel".to_string()),
        ("x-github-api-version", API_VERSION.to_string()),
        ("x-request-id", uuid::Uuid::new_v4().to_string()),
        (
            "x-vscode-user-agent-library-version",
            "electron-fetch".to_string(),
        ),
        ("x-initiator", "user".to_string()),
    ]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::{Router, http::HeaderMap, routing::get};
    use serde_json::json;
    use std::sync::Arc;

    /// A login directory whose stored token Rig will reuse without the
    /// network: a live token, bound to no GitHub token, aimed at `api_base`.
    pub(crate) fn seeded_login(name: &str, api_base: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hotline-core-copilot-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let expires = chrono::Utc::now().timestamp() + 3600;
        std::fs::write(
            dir.join(crate::providers::rig_login::COPILOT_RECORD),
            json!({
                "token": "copilot-test-token",
                "expires_at": expires,
                "endpoints": {"api": api_base}
            })
            .to_string(),
        )
        .unwrap();
        dir
    }

    pub(crate) fn record_endpoints(dir: &Path, endpoints: serde_json::Value) {
        std::fs::write(dir.join(ENDPOINTS_FILE), endpoints.to_string()).unwrap();
    }

    async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (url, task)
    }

    #[test]
    fn only_a_model_offered_responses_alone_leaves_rigs_route() {
        let dir = seeded_login("route", API_BASE);
        assert!(!wants_responses(&dir, "grok-4.6"), "no list yet");
        record_endpoints(
            &dir,
            json!({
                "grok-4.6": ["/responses"],
                "gpt-5.4": ["/chat/completions", "/responses"],
                "claude-sonnet-5": ["/chat/completions"],
                "odd": []
            }),
        );
        assert!(wants_responses(&dir, "grok-4.6"));
        assert!(!wants_responses(&dir, "gpt-5.4"));
        assert!(!wants_responses(&dir, "claude-sonnet-5"));
        assert!(!wants_responses(&dir, "odd"));
        assert!(!wants_responses(&dir, "never-listed"));
        std::fs::write(dir.join(ENDPOINTS_FILE), b"not json").unwrap();
        assert!(
            !wants_responses(&dir, "grok-4.6"),
            "a broken file is no list"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_live_token_and_host_make_a_session_and_a_bad_host_is_ignored() {
        let context = |token: &str, api_base: Option<&str>| AuthContext {
            api_key: token.to_string().into(),
            account_id: None,
            api_base: api_base.map(str::to_string),
        };
        let session =
            session_of(context("copilot-test-token", Some("http://127.0.0.1:9/"))).unwrap();
        assert_eq!(session.token, "copilot-test-token");
        assert_eq!(session.api_base, "http://127.0.0.1:9");
        assert_eq!(
            session_of(context("t", Some("ftp://x?y")))
                .unwrap()
                .api_base,
            API_BASE
        );
        assert_eq!(session_of(context("t", None)).unwrap().api_base, API_BASE);
        let err = session_of(context(" ", None)).err().unwrap();
        assert!(err.contains("sign-in"), "{err}");
    }

    #[tokio::test]
    async fn a_seeded_login_is_a_session_without_the_network() {
        let dir = seeded_login("session", "http://127.0.0.1:9/");
        let session = session(&dir).await.unwrap();
        assert_eq!(session.token, "copilot-test-token");
        assert_eq!(session.api_base, "http://127.0.0.1:9");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_login_with_no_endpoints_fetches_them_once_and_a_failure_waits() {
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = hits.clone();
        let app = Router::new().route(
            "/models",
            get(move || {
                let counted = counted.clone();
                async move {
                    counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    json!({"data": [{"id":"grok-4.6","supported_endpoints":["/responses"]}]})
                        .to_string()
                }
            }),
        );
        let (url, server) = serve(app).await;
        let dir = seeded_login("ensure", &url);
        assert!(!wants_responses(&dir, "grok-4.6"));
        ensure_endpoints(&dir).await;
        assert!(wants_responses(&dir, "grok-4.6"));
        ensure_endpoints(&dir).await;
        assert_eq!(
            hits.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a list on file is not fetched again"
        );

        let failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = failures.clone();
        let failing = Router::new().route(
            "/models",
            get(move || {
                let counted = counted.clone();
                async move {
                    counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR
                }
            }),
        );
        let (failing_url, failing_server) = serve(failing).await;
        let down = seeded_login("ensure-down", &failing_url);
        ensure_endpoints(&down).await;
        assert!(!wants_responses(&down, "grok-4.6"));
        ensure_endpoints(&down).await;
        assert_eq!(
            failures.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a failed login is not retried at once"
        );
        server.abort();
        failing_server.abort();
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&down);
    }

    #[tokio::test]
    async fn discovery_records_each_models_endpoints_and_skips_embeddings() {
        let (seen, mut headers_rx) = tokio::sync::mpsc::channel::<HeaderMap>(2);
        let app = Router::new().route(
            "/models",
            get(move |headers: HeaderMap| {
                let seen = seen.clone();
                async move {
                    seen.send(headers).await.unwrap();
                    json!({"data": [
                        {"id":"grok-4.6","name":"Grok 4.6","vendor":"xAI","supported_endpoints":["/responses"],"capabilities":{"type":"chat"}},
                        {"id":"claude-sonnet-5","name":"Claude Sonnet 5","supported_endpoints":["/chat/completions","/responses"],"capabilities":{"type":"chat"}},
                        {"id":"gpt-4o-mini","name":"Old one","capabilities":{"type":"chat"}},
                        {"id":"text-embedding-3-small","supported_endpoints":["/embeddings"],"capabilities":{"type":"embeddings"}}
                    ]})
                    .to_string()
                }
            }),
        );
        let (url, server) = serve(app).await;
        let dir = seeded_login("discover", &url);
        let models = list_models(&dir).await.unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["claude-sonnet-5", "gpt-4o-mini", "grok-4.6"]
        );
        assert_eq!(models[2].name.as_deref(), Some("Grok 4.6"));
        let recorded = read_endpoints(&dir).unwrap();
        assert_eq!(recorded["grok-4.6"], ["/responses"]);
        assert_eq!(
            recorded["claude-sonnet-5"],
            ["/chat/completions", "/responses"]
        );
        assert!(!recorded.contains_key("gpt-4o-mini"));
        assert!(!recorded.contains_key("text-embedding-3-small"));
        assert!(wants_responses(&dir, "grok-4.6"));
        assert!(!wants_responses(&dir, "claude-sonnet-5"));
        let sent = headers_rx.recv().await.unwrap();
        assert_eq!(sent["authorization"], "Bearer copilot-test-token");
        assert_eq!(sent["copilot-integration-id"], INTEGRATION_ID);
        assert_eq!(sent["editor-version"], EDITOR_VERSION);
        assert_eq!(sent["x-initiator"], "user");
        server.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_responses_body_reaches_copilot_without_unique_items() {
        let without = json!({"type": "array", "items": {"type": "string", "enum": ["computer"]}});
        let mut unique = without.clone();
        unique["uniqueItems"] = json!(true);
        let body = json!({
            "model": "gpt-5.3-codex",
            "input": [{"role": "user", "content": "uniqueItems stays in what was said"}],
            "tools": [
                {"type": "function", "name": "schedule", "strict": false, "parameters": {
                    "type": "object",
                    "properties": {"when": {"type": "string"}, "without": unique},
                    "anyOf": [{"properties": {"tags": unique}}]
                }},
                {"type": "web_search"}
            ]
        });
        let out: serde_json::Value = serde_json::from_slice(&drop_unique_items(Bytes::from(
            serde_json::to_vec(&body).unwrap(),
        )))
        .unwrap();
        let schedule = &out["tools"][0];
        assert_eq!(schedule["parameters"]["properties"]["without"], without);
        assert_eq!(
            schedule["parameters"]["anyOf"][0]["properties"]["tags"],
            without
        );
        assert_eq!(schedule["strict"], false);
        assert_eq!(out["tools"][1], body["tools"][1]);
        assert_eq!(out["input"], body["input"]);

        let plain = Bytes::from(
            serde_json::to_vec(&json!({"tools": [{"type": "function", "parameters": without}]}))
                .unwrap(),
        );
        assert_eq!(
            drop_unique_items(plain.clone()),
            plain,
            "an untouched body passes as it came"
        );
        let other = Bytes::from_static(b"not json");
        assert_eq!(drop_unique_items(other.clone()), other);
    }
}
