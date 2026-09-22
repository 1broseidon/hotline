//! GitHub Copilot's second route.
//!
//! Rig sends every Copilot model except the Codex family to
//! `/chat/completions`. Copilot's own model list says which models answer
//! only on `/responses` (the newer OpenAI reasoning models and Grok, at the
//! time of writing), and a chat request to one of those is refused with
//! `unsupported_api_for_model`. Discovery records each model's endpoints
//! beside the login; a model that lists `/responses` and not
//! `/chat/completions` is then driven through Rig's OpenAI Responses client,
//! with a transport that signs each request the way Copilot's own client
//! does. Sign-in, token refresh and the stored token stay Rig's.

use super::discovery::{self, DiscoveryHttp, ListedModel};
use bytes::Bytes;
use futures_util::StreamExt;
use rig::http_client::{self, HttpClientExt, LazyBody, MultipartForm, StreamingResponse};
use rig::providers::{copilot, openai};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const API_BASE: &str = "https://api.githubcopilot.com";
/// The per-model endpoints Copilot advertised, beside the login.
const ENDPOINTS_FILE: &str = "endpoints.json";
const RESPONSES: &str = "/responses";
const CHAT_COMPLETIONS: &str = "/chat/completions";
const MAX_ENDPOINTS: usize = 16;
const MAX_ENDPOINT_LEN: usize = 64;

// The identity Rig 0.42 presents to Copilot. Copilot refuses a request
// that does not look like an editor's, so the transport here says the same.
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

/// The record Rig writes beside the login when it exchanges the GitHub
/// token: the Copilot API token and, sometimes, the API host it belongs to.
#[derive(Deserialize)]
struct ApiKeyRecord {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    endpoints: Option<ApiEndpoints>,
}

#[derive(Deserialize)]
struct ApiEndpoints {
    #[serde(default)]
    api: Option<String>,
}

fn rig_client(token_dir: &Path) -> Result<copilot::Client, String> {
    copilot::Client::builder()
        .oauth()
        .token_dir(token_dir)
        .allow_device_flow(false)
        .build()
        .map_err(|_| {
            "Could not prepare the GitHub Copilot sign-in. Sign in again under Settings → Providers."
                .to_string()
        })
}

/// A usable token, refreshed by Rig when the stored one has expired. With no
/// sign-in on disk this fails at once, naming the sign-in, and never starts
/// a device flow.
pub(crate) async fn session(token_dir: &Path) -> Result<Session, String> {
    rig_client(token_dir)?
        .authorize()
        .await
        .map_err(|error| error.to_string())?;
    read_session(token_dir)
}

fn read_session(token_dir: &Path) -> Result<Session, String> {
    let bytes = crate::vault::read_model_file(&token_dir.join("api-key.json")).map_err(
        |_| "GitHub Copilot sign-in required. Sign in again under Settings → Providers.",
    )?;
    let record: ApiKeyRecord = serde_json::from_slice(&bytes)
        .map_err(|_| "The GitHub Copilot sign-in could not be read. Sign in again.")?;
    let token = record
        .token
        .filter(|token| !token.trim().is_empty())
        .ok_or("GitHub Copilot sign-in required. Sign in again under Settings → Providers.")?;
    let api_base = record
        .endpoints
        .and_then(|endpoints| endpoints.api)
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
    for (name, value) in headers(&session.token, RequestFacts::default()) {
        request = request.header(name, value);
    }
    let request = request.body(Bytes::new()).map_err(|_| failed())?;
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        DiscoveryHttp::default().send::<Bytes, Vec<u8>>(request),
    )
    .await
    .map_err(|_| "Model discovery timed out; the previous list is unchanged.".to_string())?
    .map_err(|_| failed())?;
    let body = response.into_body().await.map_err(|_| failed())?;
    let listed: ListModelsResponse = serde_json::from_slice(&body).map_err(|_| failed())?;
    let mut endpoints = Endpoints::new();
    let mut models = Vec::new();
    for entry in listed.data {
        if matches!(
            entry.capabilities.and_then(|caps| caps.kind).as_deref(),
            Some("embedding" | "embeddings")
        ) {
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

// ---- The Responses route -------------------------------------------------

/// Rig's OpenAI Responses client aimed at Copilot. System instructions ride
/// as `input` messages, as they do on Rig's own Copilot Responses route.
pub(crate) fn responses_client(
    token_dir: &Path,
    session: &Session,
) -> Result<openai::Client<Transport>, String> {
    openai::Client::builder()
        .api_key(session.token.as_str())
        .base_url(&session.api_base)
        .http_client(Transport::new(token_dir)?)
        .build()
        .map(|client| client.with_system_instructions_as_messages())
        .map_err(|_| "Could not prepare the GitHub Copilot connection.".to_string())
}

/// The transport under that client. Every request is signed afresh: Rig
/// refreshes the stored token when it has expired, and the bearer the
/// client was built with is replaced by the live one, so a turn that
/// outlives a token does not end in a 401.
#[derive(Clone, Default)]
pub(crate) struct Transport {
    login: Option<Arc<Login>>,
    http: reqwest::Client,
}

struct Login {
    client: copilot::Client,
    token_dir: PathBuf,
}

impl fmt::Debug for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CopilotTransport")
            .field(
                "token_dir",
                &self.login.as_ref().map(|login| login.token_dir.as_path()),
            )
            .finish()
    }
}

impl Transport {
    fn new(token_dir: &Path) -> Result<Self, String> {
        Ok(Self {
            login: Some(Arc::new(Login {
                client: rig_client(token_dir)?,
                token_dir: token_dir.to_path_buf(),
            })),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "Could not prepare the GitHub Copilot connection.".to_string())?,
        })
    }

    async fn signed(&self, request: http::Request<Bytes>) -> http_client::Result<reqwest::Request> {
        let login = self
            .login
            .as_ref()
            .ok_or_else(|| refused("This transport has no GitHub Copilot sign-in."))?;
        login
            .client
            .authorize()
            .await
            .map_err(|error| refused_with(error.to_string()))?;
        let session = read_session(&login.token_dir).map_err(refused_with)?;
        let (mut parts, body) = request.into_parts();
        let facts = RequestFacts::from_body(&body);
        for (name, value) in headers(&session.token, facts) {
            let value = http::HeaderValue::from_str(&value)?;
            parts.headers.insert(name, value);
        }
        self.http
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(body)
            .build()
            .map_err(|error| http_client::Error::Instance(error.into()))
    }
}

fn refused(message: &'static str) -> http_client::Error {
    http_client::Error::Instance(Box::new(std::io::Error::other(message)))
}

fn refused_with(message: String) -> http_client::Error {
    http_client::Error::Instance(Box::new(std::io::Error::other(message)))
}

async fn non_success(response: reqwest::Response) -> http_client::Error {
    let status = response.status();
    let headers = Box::new(response.headers().clone());
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| format!("failed to read error response body: {error}"));
    http_client::Error::InvalidStatusCodeWithDetails {
        status,
        body,
        headers,
    }
}

impl HttpClientExt for Transport {
    fn send<T, U>(
        &self,
        request: http::Request<T>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        let this = self.clone();
        let request = request.map(Into::into);
        async move {
            let request = this.signed(request).await?;
            let response = this
                .http
                .execute(request)
                .await
                .map_err(|error| http_client::Error::Instance(error.into()))?;
            if !response.status().is_success() {
                return Err(non_success(response).await);
            }
            let mut result = http::Response::builder().status(response.status());
            if let Some(headers) = result.headers_mut() {
                *headers = response.headers().clone();
            }
            let body: LazyBody<U> = Box::pin(async move {
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|error| http_client::Error::Instance(error.into()))?;
                Ok(U::from(bytes))
            });
            result.body(body).map_err(http_client::Error::Protocol)
        }
    }

    fn send_multipart<U>(
        &self,
        _: http::Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        std::future::ready(Err(refused("GitHub Copilot requests are never multipart.")))
    }

    async fn send_streaming<T>(
        &self,
        request: http::Request<T>,
    ) -> http_client::Result<StreamingResponse>
    where
        T: Into<Bytes> + Send,
    {
        let request = self.signed(request.map(Into::into)).await?;
        let response = self
            .http
            .execute(request)
            .await
            .map_err(|error| http_client::Error::Instance(error.into()))?;
        if !response.status().is_success() {
            return Err(non_success(response).await);
        }
        let mut result = http::Response::builder()
            .status(response.status())
            .version(response.version());
        if let Some(headers) = result.headers_mut() {
            *headers = response.headers().clone();
        }
        let stream: http_client::sse::BoxedStream = Box::pin(
            response
                .bytes_stream()
                .map(|chunk| chunk.map_err(|error| http_client::Error::Instance(Box::new(error)))),
        );
        result.body(stream).map_err(http_client::Error::Protocol)
    }
}

/// What Copilot reads off a request's headers about the turn it belongs to.
/// `X-Initiator: agent` is a continuation after the model's own tool calls,
/// which Copilot bills differently from a person's message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RequestFacts {
    continuation: bool,
    vision: bool,
}

impl RequestFacts {
    /// Read off a Responses body: any assistant item or tool output in
    /// `input` makes the request a continuation; any image makes it vision.
    fn from_body(body: &[u8]) -> Self {
        let Ok(body) = serde_json::from_slice::<serde_json::Value>(body) else {
            return Self::default();
        };
        let items = body["input"].as_array().cloned().unwrap_or_default();
        let continuation = items.iter().any(|item| {
            item["role"] == "assistant"
                || matches!(
                    item["type"].as_str(),
                    Some("function_call" | "function_call_output")
                )
        });
        let vision = items.iter().any(|item| {
            item["content"]
                .as_array()
                .is_some_and(|parts| parts.iter().any(|part| part["type"] == "input_image"))
        });
        Self {
            continuation,
            vision,
        }
    }
}

fn headers(token: &str, facts: RequestFacts) -> Vec<(&'static str, String)> {
    let mut headers = vec![
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
        (
            "x-initiator",
            if facts.continuation { "agent" } else { "user" }.to_string(),
        ),
    ];
    if facts.vision {
        headers.push(("copilot-vision-request", "true".to_string()));
    }
    headers
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::{
        Router,
        http::HeaderMap,
        routing::{get, post},
    };
    use serde_json::json;

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
            dir.join("api-key.json"),
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
    fn a_stored_token_and_host_make_a_session_and_a_bad_host_is_ignored() {
        let dir = seeded_login("session", "http://127.0.0.1:9/");
        let session = read_session(&dir).unwrap();
        assert_eq!(session.token, "copilot-test-token");
        assert_eq!(session.api_base, "http://127.0.0.1:9");
        std::fs::write(
            dir.join("api-key.json"),
            json!({"token": "t", "endpoints": {"api": "ftp://x?y"}}).to_string(),
        )
        .unwrap();
        assert_eq!(read_session(&dir).unwrap().api_base, API_BASE);
        std::fs::write(dir.join("api-key.json"), b"{}").unwrap();
        let err = read_session(&dir).err().unwrap();
        assert!(err.contains("sign-in"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_continuation_and_an_image_are_read_off_the_body() {
        let user = json!({"input": [{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}]});
        assert_eq!(
            RequestFacts::from_body(user.to_string().as_bytes()),
            RequestFacts::default()
        );
        let after_tool = json!({"input": [
            {"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]},
            {"type":"function_call","name":"read_file","arguments":"{}","call_id":"c1"},
            {"type":"function_call_output","call_id":"c1","output":"..."}
        ]});
        assert_eq!(
            RequestFacts::from_body(after_tool.to_string().as_bytes()),
            RequestFacts {
                continuation: true,
                vision: false
            }
        );
        let with_image = json!({"input": [{"type":"message","role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,AA=="}]}]});
        assert_eq!(
            RequestFacts::from_body(with_image.to_string().as_bytes()),
            RequestFacts {
                continuation: false,
                vision: true
            }
        );
        assert_eq!(RequestFacts::from_body(b"nope"), RequestFacts::default());
        let named = headers(
            "tok",
            RequestFacts {
                continuation: true,
                vision: true,
            },
        );
        assert!(named.contains(&("x-initiator", "agent".to_string())));
        assert!(named.contains(&("copilot-vision-request", "true".to_string())));
        assert!(named.contains(&("authorization", "Bearer tok".to_string())));
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

    #[tokio::test]
    async fn the_transport_signs_each_request_and_streams_the_reply() {
        let (seen, mut headers_rx) = tokio::sync::mpsc::channel::<HeaderMap>(4);
        let app = Router::new().route(
            "/responses",
            post(move |headers: HeaderMap, body: Bytes| {
                let seen = seen.clone();
                async move {
                    seen.send(headers).await.unwrap();
                    let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    if request["stream"] == true {
                        (
                            [("content-type", "text/event-stream")],
                            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hel\"}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\n".to_string(),
                        )
                    } else {
                        (
                            [("content-type", "application/json")],
                            json!({"id":"resp_1","output":[]}).to_string(),
                        )
                    }
                }
            }),
        );
        let (url, server) = serve(app).await;
        let dir = seeded_login("transport", &url);
        let transport = Transport::new(&dir).unwrap();
        let plain = http::Request::post(format!("{url}/responses"))
            .header("authorization", "Bearer stale-bearer-from-build-time")
            .body(Bytes::from(
                json!({"model":"grok-4.6","input":[{"type":"function_call_output","call_id":"c","output":"x"}]}).to_string(),
            ))
            .unwrap();
        let response = transport.send::<Bytes, Vec<u8>>(plain).await.unwrap();
        let body = response.into_body().await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"],
            "resp_1"
        );
        let sent = headers_rx.recv().await.unwrap();
        assert_eq!(
            sent["authorization"], "Bearer copilot-test-token",
            "the live token replaces the built-in bearer"
        );
        assert_eq!(sent["x-initiator"], "agent");
        assert_eq!(sent["openai-intent"], "conversation-panel");

        let streamed = http::Request::post(format!("{url}/responses"))
            .body(Bytes::from(
                json!({"model":"grok-4.6","stream":true,"input":[]}).to_string(),
            ))
            .unwrap();
        let response = transport.send_streaming(streamed).await.unwrap();
        let mut text = String::new();
        let mut stream = response.into_body();
        while let Some(chunk) = stream.next().await {
            text.push_str(std::str::from_utf8(&chunk.unwrap()).unwrap());
        }
        assert!(text.contains("\"delta\":\"hel\"") && text.contains("\"delta\":\"lo\""));
        assert_eq!(headers_rx.recv().await.unwrap()["x-initiator"], "user");

        let stale = Transport::default();
        let err = stale
            .send::<Bytes, Vec<u8>>(
                http::Request::post(format!("{url}/responses"))
                    .body(Bytes::new())
                    .unwrap(),
            )
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("sign-in"), "{err}");
        server.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
