//! MCP OAuth sign-in end to end against a loopback authorization server,
//! through the vault: the client crate's flow, and the values this module
//! reads for it and the registration it saves back.

use super::*;
use crate::log::{Log, StreamId};
use crate::vault::Vault;
use axum::body::Body;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use axum::routing::any;
use axum::{Router, body::to_bytes};
use hotline_mcp::connect_oauth;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::auth::CredentialStore;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

#[derive(Debug)]
struct MockRequest {
    path: String,
    method: String,
    authorization: Option<String>,
    body: String,
}

struct MockOAuthState {
    base: String,
    supports_registration: bool,
    current_access: Mutex<String>,
    registrations: AtomicUsize,
    code_exchanges: AtomicUsize,
    refreshes: AtomicUsize,
    requests: Mutex<Vec<MockRequest>>,
}

struct MockOAuthServer {
    base: String,
    state: Arc<MockOAuthState>,
    shutdown: CancellationToken,
    task: JoinHandle<()>,
}

impl MockOAuthServer {
    async fn start() -> Self {
        Self::start_with_registration(true).await
    }

    async fn start_without_registration() -> Self {
        Self::start_with_registration(false).await
    }

    async fn start_with_registration(supports_registration: bool) -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("mock OAuth listener");
        let address = listener.local_addr().expect("mock OAuth address");
        let base = format!("http://{address}");
        let state = Arc::new(MockOAuthState {
            base: base.clone(),
            supports_registration,
            current_access: Mutex::new("access-1".to_string()),
            registrations: AtomicUsize::new(0),
            code_exchanges: AtomicUsize::new(0),
            refreshes: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        });
        let router = Router::new()
            .fallback(any(mock_request))
            .with_state(state.clone());
        let shutdown = CancellationToken::new();
        let wait = shutdown.clone();
        let task = tokio::spawn(async move {
            let signal = async move { wait.cancelled().await };
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(signal)
                .await;
        });
        Self {
            base,
            state,
            shutdown,
            task,
        }
    }

    fn server_with_auth(&self, auth: HttpAuth) -> McpServer {
        McpServer {
            id: "mock-oauth".to_string(),
            name: "Mock OAuth MCP".to_string(),
            transport: McpTransport::Http {
                url: format!("{}/mcp", self.base),
                auth,
            },
            refuse: None,
        }
    }

    fn server(&self) -> McpServer {
        self.server_with_auth(HttpAuth::Oauth)
    }
}

impl Drop for MockOAuthServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.task.abort();
    }
}

async fn mock_request(
    State(state): State<Arc<MockOAuthState>>,
    request: Request<Body>,
) -> Response {
    let path = request.uri().path().to_string();
    let method = request.method().to_string();
    let authorization = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = to_bytes(request.into_body(), 16 * 1024 * 1024)
        .await
        .expect("mock request body");
    state
        .requests
        .lock()
        .expect("mock request log")
        .push(MockRequest {
            path: path.clone(),
            method: method.clone(),
            authorization: authorization.clone(),
            body: String::from_utf8_lossy(&body).into_owned(),
        });

    match (method.as_str(), path.as_str()) {
        ("GET", "/.well-known/oauth-protected-resource") => json_response(
            StatusCode::OK,
            json!({
                "resource": format!("{}/mcp", state.base),
                "authorization_servers": [format!("{}/as", state.base)],
                "scopes_supported": ["mcp"],
            }),
        ),
        ("GET", "/.well-known/oauth-authorization-server/as") => {
            let mut metadata = json!({
                "issuer": format!("{}/as", state.base),
                "authorization_endpoint": format!("{}/as/authorize", state.base),
                "token_endpoint": format!("{}/as/token", state.base),
                "response_types_supported": ["code"],
                "code_challenge_methods_supported": ["S256"],
                "scopes_supported": ["mcp"],
                "token_endpoint_auth_methods_supported": ["client_secret_post"],
            });
            if state.supports_registration {
                metadata["registration_endpoint"] = json!(format!("{}/as/register", state.base));
            }
            json_response(StatusCode::OK, metadata)
        }
        ("POST", "/as/register") => {
            state.registrations.fetch_add(1, Ordering::SeqCst);
            let registration: Value =
                serde_json::from_slice(&body).expect("mock registration request");
            json_response(
                StatusCode::CREATED,
                json!({
                    "client_id": "mock-client",
                    "client_secret": "mock-client-secret",
                    "redirect_uris": registration["redirect_uris"].clone(),
                }),
            )
        }
        ("POST", "/as/token") => {
            let form: HashMap<String, String> =
                url::form_urlencoded::parse(&body).into_owned().collect();
            match form.get("grant_type").map(String::as_str) {
                Some("authorization_code")
                    if form.get("code").map(String::as_str) == Some("good-code") =>
                {
                    state.code_exchanges.fetch_add(1, Ordering::SeqCst);
                    json_response(
                        StatusCode::OK,
                        json!({
                            "access_token": "access-1",
                            "token_type": "Bearer",
                            "expires_in": 0,
                            "refresh_token": "refresh-1",
                            "scope": "mcp",
                        }),
                    )
                }
                Some("refresh_token") => {
                    let refresh = form.get("refresh_token").map(String::as_str);
                    if refresh.is_some_and(|value| value.starts_with("refresh-")) {
                        let number = state.refreshes.fetch_add(1, Ordering::SeqCst) + 2;
                        let access = format!("access-{number}");
                        let refresh = format!("refresh-{number}");
                        *state.current_access.lock().expect("mock token") = access.clone();
                        json_response(
                            StatusCode::OK,
                            json!({
                                "access_token": access,
                                "token_type": "Bearer",
                                "expires_in": 3600,
                                "refresh_token": refresh,
                                "scope": "mcp",
                            }),
                        )
                    } else {
                        json_response(StatusCode::BAD_REQUEST, json!({ "error": "invalid_grant" }))
                    }
                }
                _ => json_response(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": "invalid_request" }),
                ),
            }
        }
        ("GET", "/mcp") => {
            if valid_access(&state, authorization.as_deref()) {
                response(StatusCode::OK, "application/json", Vec::new())
            } else {
                mcp_challenge(&state)
            }
        }
        ("DELETE", "/mcp") => {
            if valid_access(&state, authorization.as_deref()) {
                response(StatusCode::ACCEPTED, "application/json", Vec::new())
            } else {
                mcp_challenge(&state)
            }
        }
        ("POST", "/mcp") => {
            if !valid_access(&state, authorization.as_deref()) {
                return mcp_challenge(&state);
            }
            let message: Value = serde_json::from_slice(&body).expect("MCP JSON request");
            let id = message.get("id").cloned().unwrap_or(Value::Null);
            match message.get("method").and_then(Value::as_str) {
                Some("initialize") => json_response(
                    StatusCode::OK,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": "2025-11-25",
                            "capabilities": { "tools": {} },
                            "serverInfo": { "name": "mock", "version": "1" },
                        },
                    }),
                ),
                Some("tools/list") => json_response(
                    StatusCode::OK,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "tools": [{
                                "name": "echo",
                                "description": "Returns a deterministic response.",
                                "inputSchema": { "type": "object" },
                            }],
                        },
                    }),
                ),
                Some("tools/call") => json_response(
                    StatusCode::OK,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": "mock-pong" }],
                            "isError": false,
                        },
                    }),
                ),
                Some(method) if method.starts_with("notifications/") => {
                    response(StatusCode::ACCEPTED, "application/json", Vec::new())
                }
                Some(method) => json_response(
                    StatusCode::NOT_FOUND,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": method },
                    }),
                ),
                None => json_response(
                    StatusCode::BAD_REQUEST,
                    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32600 } }),
                ),
            }
        }
        _ => response(StatusCode::NOT_FOUND, "text/plain", Vec::new()),
    }
}

fn valid_access(state: &MockOAuthState, authorization: Option<&str>) -> bool {
    let expected = format!(
        "Bearer {}",
        state.current_access.lock().expect("mock token").as_str()
    );
    authorization == Some(expected.as_str())
}

fn mcp_challenge(state: &MockOAuthState) -> Response {
    let metadata = format!("{}/.well-known/oauth-protected-resource", state.base);
    let value = format!(r#"Bearer resource_metadata="{metadata}", scope="mcp""#);
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(WWW_AUTHENTICATE, value)
        .header(CONTENT_LENGTH, "0")
        .body(Body::empty())
        .expect("mock challenge")
}

fn response(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_LENGTH, body.len())
        .body(Body::from(body))
        .expect("mock response")
}

fn json_response(status: StatusCode, value: Value) -> Response {
    response(
        status,
        "application/json",
        serde_json::to_vec(&value).expect("mock JSON"),
    )
}

fn scratch(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "hotline-oauth-{name}-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("oauth scratch");
    path
}

fn callback_value(started: &Value, code: &str, state: &str) -> String {
    let mut callback =
        Url::parse(started["redirectUri"].as_str().expect("redirect")).expect("redirect URL");
    callback
        .query_pairs_mut()
        .append_pair("code", code)
        .append_pair("state", state);
    callback.to_string()
}

fn state_from(started: &Value) -> String {
    Url::parse(
        started["authorizationUrl"]
            .as_str()
            .expect("authorization URL"),
    )
    .expect("authorization URL")
    .query_pairs()
    .find(|(key, _)| key == "state")
    .map(|(_, value)| value.into_owned())
    .expect("OAuth state")
}

async fn send_native_callback(started: &Value, code: &str, state: &str) -> String {
    let callback = callback_value(started, code, state);
    let callback = Url::parse(&callback).expect("callback URL");
    let port = callback.port().expect("callback port");
    let target = format!(
        "{}?{}",
        callback.path(),
        callback.query().expect("callback query")
    );
    let mut socket = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("native callback listener");
    socket
        .write_all(
            format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .expect("native callback request");
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .expect("native callback response timeout")
        .expect("native callback response");
    String::from_utf8(response).expect("native callback response text")
}

/// The callback page's headers carry the strict policy, its body is the
/// styled page with the given heading, and nothing on it can run.
fn assert_callback_page(response: &str, status: &str, title: &str) {
    let (head, body) = response.split_once("\r\n\r\n").expect("headers end");
    assert!(head.starts_with(&format!("HTTP/1.1 {status}")), "{head}");
    let policy = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Security-Policy: "))
        .expect("Content-Security-Policy header");
    assert_eq!(
        policy,
        "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    );
    assert!(policy.starts_with("default-src 'none';"), "{policy}");
    assert!(!policy.contains("script-src"), "{policy}");
    assert!(head.contains("\r\nCache-Control: no-store"), "{head}");
    assert!(body.contains(&format!("<h1>{title}</h1>")), "{body}");
    assert!(body.contains("prefers-color-scheme:light"), "{body}");
    assert!(!body.to_ascii_lowercase().contains("<script"), "{body}");
}

/// A callback the sign-in cannot accept gets the failure page, under the
/// same policy, and leaves the sign-in waiting; the right one then gets
/// the signed-in page.
#[tokio::test(flavor = "multi_thread")]
async fn the_callback_pages_say_what_happened_under_a_strict_policy() {
    let mock = MockOAuthServer::start().await;
    let root = scratch("callback-page");
    let log = Log::open(&root);
    let vault = Arc::new(Vault::open(&root, log).expect("vault"));
    let server = mock.server();
    let service = McpOAuth::new(vault);
    let started = service.start(&server).await.expect("start OAuth");

    let response = send_native_callback(&started, "good-code", "wrong-state").await;
    assert_callback_page(&response, "400 Bad Request", "Sign-in failed");
    assert!(response.contains("state did not match"), "{response}");
    assert!(!response.contains("good-code"), "{response}");
    assert_eq!(
        service.status(&server).await.expect("pending status")["status"],
        "pending"
    );

    let state = state_from(&started);
    let response = send_native_callback(&started, "good-code", &state).await;
    assert_callback_page(&response, "200 OK", "Signed in");
    assert!(response.contains("You can close this tab."), "{response}");
    assert_eq!(
        service.status(&server).await.expect("signed-in status")["status"],
        "signed_in"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread")]
async fn oauth_discovery_dcr_native_callback_refresh_restart_and_mcp_call() {
    let mock = MockOAuthServer::start().await;
    let root = scratch("flow");
    let log = Log::open(&root);
    let vault = Arc::new(Vault::open(&root, log.clone()).expect("vault"));
    let server = mock.server();
    let service = McpOAuth::new(vault.clone());

    let started = service.start(&server).await.expect("start OAuth");
    assert_eq!(started["status"], "pending");
    assert_eq!(mock.state.registrations.load(Ordering::SeqCst), 1);
    let state = state_from(&started);
    let response = send_native_callback(&started, "good-code", &state).await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert_eq!(
        service.status(&server).await.expect("completed status")["status"],
        "signed_in"
    );
    assert_eq!(mock.state.code_exchanges.load(Ordering::SeqCst), 1);
    let registration = vault
        .mcp_oauth_registration(&server.id, &format!("{}/mcp", mock.base))
        .expect("registration read")
        .expect("saved registration");
    assert_eq!(registration.client_id, "mock-client");
    let isolation = vault
        .mcp_oauth_registration(&server.id, &format!("{}/other", mock.base))
        .expect_err("a registration must be bound to its MCP URL");
    assert!(isolation.to_string().contains("different server URL"));

    let (client, tools) = connect_oauth(&server, &oauth_vault(&vault, &server))
        .await
        .expect("authenticated MCP connection");
    assert_eq!(tools.len(), 1);
    let called = client
        .peer()
        .call_tool(CallToolRequestParams::new("echo"))
        .await
        .expect("authenticated tools/call");
    assert_eq!(called.content[0].as_text().expect("text").text, "mock-pong");
    client.cancel().await.ok();
    assert_eq!(mock.state.refreshes.load(Ordering::SeqCst), 1);

    // A second manager after an application restart reuses the saved
    // registration and credential record instead of registering again.
    let restarted = McpOAuth::new(vault.clone());
    let status = restarted
        .status(&server)
        .await
        .expect("status after restart");
    assert_eq!(status["status"], "signed_in");
    let (_, restarted_tools) = connect_oauth(&server, &oauth_vault(&vault, &server))
        .await
        .expect("reconnected MCP");
    assert_eq!(restarted_tools.len(), 1);
    assert_eq!(mock.state.registrations.load(Ordering::SeqCst), 1);
    {
        let requests = mock.state.requests.lock().expect("mock request log");
        let mcp_posts: Vec<&MockRequest> = requests
            .iter()
            .filter(|request| request.path == "/mcp" && request.method == "POST")
            .collect();
        assert!(!mcp_posts.is_empty());
        assert!(
            mcp_posts
                .iter()
                .all(|request| request.authorization.as_deref() == Some("Bearer access-2"))
        );
        assert!(
            mcp_posts
                .iter()
                .all(|request| !request.body.contains("refresh-"))
        );
    }

    // Independent sessions can decide at the same time that an access
    // token is expired. The vault lock covers the decision as well as the
    // SDK refresh, so the second manager observes the rotated token
    // instead of spending the same refresh token again.
    let store = vault.mcp_credential_store(&server.id, &format!("{}/mcp", mock.base));
    let mut expired = store
        .load()
        .await
        .expect("stored credentials")
        .expect("credentials after reconnect");
    let mut token = serde_json::to_value(
        expired
            .token_response
            .as_ref()
            .expect("access and refresh token"),
    )
    .expect("token JSON");
    token["expires_in"] = json!(0);
    token["refresh_token"] = json!("refresh-2");
    expired.token_response = Some(serde_json::from_value(token).expect("expired token"));
    expired.token_received_at = Some(0);
    store.save(expired).await.expect("expire credentials");
    let first_manager = manager_for_server(&server, &oauth_vault(&vault, &server), None)
        .await
        .expect("first manager");
    let second_manager = manager_for_server(&server, &oauth_vault(&vault, &server), None)
        .await
        .expect("second manager");
    let first_token_lock = vault.mcp_token_lock(&server.id);
    let second_token_lock = vault.mcp_token_lock(&server.id);
    let (first, second) = tokio::join!(
        async {
            let _token_guard = first_token_lock.lock().await;
            first_manager.get_access_token().await
        },
        async {
            let _token_guard = second_token_lock.lock().await;
            second_manager.get_access_token().await
        }
    );
    assert_eq!(first.expect("first concurrent refresh"), "access-3");
    assert_eq!(second.expect("second concurrent refresh"), "access-3");
    assert_eq!(mock.state.refreshes.load(Ordering::SeqCst), 2);

    // A definitive refresh-token rejection is actionable and secret-free
    // in the status surface; it does not silently fall back to anonymous
    // MCP requests.
    let mut rejected = store
        .load()
        .await
        .expect("stored credentials")
        .expect("rotated credentials");
    let mut token = serde_json::to_value(
        rejected
            .token_response
            .as_ref()
            .expect("access and refresh token"),
    )
    .expect("token JSON");
    token["expires_in"] = json!(0);
    token["refresh_token"] = json!("invalid-refresh");
    rejected.token_response = Some(serde_json::from_value(token).expect("rejected token"));
    rejected.token_received_at = Some(0);
    store.save(rejected).await.expect("reject credentials");
    let status = restarted.status(&server).await.expect("refresh status");
    assert_eq!(status["status"], "failed");
    assert!(
        !status["error"]
            .as_str()
            .unwrap_or_default()
            .contains("invalid-refresh")
    );
    let error = connect_oauth(&server, &oauth_vault(&vault, &server))
        .await
        .expect_err("rejected refresh must stop connection");
    assert!(error.contains("sign-in"), "{error}");

    // The room log never receives bearer, refresh or client-secret material.
    let room_text = serde_json::to_string(&log.load(&StreamId::Room)).expect("room JSON");
    assert!(!room_text.contains("access-"));
    assert!(!room_text.contains("refresh-"));
    assert!(!room_text.contains("mock-client-secret"));

    restarted.sign_out(&server).await.expect("sign out");
    assert_eq!(
        restarted.status(&server).await.expect("signed out status")["status"],
        "signed_out"
    );
    let store = vault.mcp_credential_store(&server.id, &format!("{}/mcp", mock.base));
    assert!(store.load().await.expect("empty OAuth store").is_none());

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_dcr_is_reported_without_synthesizing_registration() {
    let mock = MockOAuthServer::start_without_registration().await;
    let root = scratch("missing-dcr");
    let log = Log::open(&root);
    let vault = Arc::new(Vault::open(&root, log).expect("vault"));
    let server = mock.server();
    let service = McpOAuth::new(vault.clone());

    let error = service
        .start(&server)
        .await
        .expect_err("a server without DCR cannot start a new native client");
    assert!(
        error.to_ascii_lowercase().contains("dcr")
            || error.to_ascii_lowercase().contains("registration")
    );
    assert_eq!(mock.state.registrations.load(Ordering::SeqCst), 0);
    assert!(
        vault
            .mcp_oauth_registration(&server.id, &format!("{}/mcp", mock.base))
            .expect("registration read")
            .is_none()
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread")]
async fn configured_client_reuses_without_dcr() {
    let mock = MockOAuthServer::start_without_registration().await;
    let root = scratch("configured-client");
    let log = Log::open(&root);
    let vault = Arc::new(Vault::open(&root, log).expect("vault"));
    let server = mock.server_with_auth(HttpAuth::OauthConfigured {
        scopes: vec!["mcp".to_string()],
        resource: Some(format!("{}/mcp", mock.base)),
        client_id: Some("configured-client".to_string()),
        token_endpoint_auth_method: Some("none".to_string()),
    });
    let service = McpOAuth::new(vault.clone());

    let started = service
        .start(&server)
        .await
        .expect("start configured OAuth");
    let callback = callback_value(&started, "good-code", &state_from(&started));
    service
        .complete(started["loginId"].as_str().expect("login id"), &callback)
        .await
        .expect("complete configured OAuth");
    assert_eq!(mock.state.registrations.load(Ordering::SeqCst), 0);
    assert_eq!(mock.state.code_exchanges.load(Ordering::SeqCst), 1);
    let registration = vault
        .mcp_oauth_registration(&server.id, &format!("{}/mcp", mock.base))
        .expect("registration read")
        .expect("configured client registration");
    assert_eq!(registration.client_id, "configured-client");
    assert_eq!(registration.client_secret, None);

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_state_and_denied_consent_do_not_create_credentials() {
    let mock = MockOAuthServer::start().await;
    let root = scratch("denied");
    let log = Log::open(&root);
    let vault = Arc::new(Vault::open(&root, log).expect("vault"));
    let server = mock.server();
    let service = McpOAuth::new(vault.clone());
    let started = service.start(&server).await.expect("start OAuth");
    let login_id = started["loginId"].as_str().expect("login id");
    let wrong = callback_value(&started, "good-code", "wrong-state");
    let error = service
        .complete(login_id, &wrong)
        .await
        .expect_err("wrong state");
    assert!(error.contains("state did not match"));
    assert_eq!(
        service.status(&server).await.expect("pending status")["status"],
        "pending"
    );

    let state = state_from(&started);
    let mut denied = Url::parse(started["redirectUri"].as_str().expect("redirect")).unwrap();
    denied
        .query_pairs_mut()
        .append_pair("error", "access_denied")
        .append_pair("state", &state);
    let error = service
        .complete(login_id, denied.as_str())
        .await
        .expect_err("denied consent");
    assert!(error.contains("consent was denied"));
    assert_eq!(
        service.status(&server).await.expect("denied status")["status"],
        "failed"
    );
    assert!(
        vault
            .mcp_credential_store(&server.id, &format!("{}/mcp", mock.base))
            .load()
            .await
            .expect("empty OAuth store")
            .is_none()
    );
    let _ = std::fs::remove_dir_all(root);
}

/// `mcp.auth_callback` is what a window on a laptop sends when the desk is
/// on a server: the address the browser landed on, pasted by a person.
/// Whatever they paste, only this sign-in's own callback finishes it, a
/// wrong one leaves the sign-in waiting for the right one, and no answer
/// repeats the address, which carries the code.
#[tokio::test(flavor = "multi_thread")]
async fn a_pasted_address_finishes_only_its_own_sign_in_and_is_never_echoed() {
    let mock = MockOAuthServer::start().await;
    let root = scratch("pasted");
    let log = Log::open(&root);
    let vault = Arc::new(Vault::open(&root, log).expect("vault"));
    let server = mock.server();
    let service = McpOAuth::new(vault.clone());
    let started = service.start(&server).await.expect("start OAuth");
    let login_id = started["loginId"].as_str().expect("login id");
    let state = state_from(&started);
    let redirect = Url::parse(started["redirectUri"].as_str().expect("redirect")).unwrap();
    let variant = |edit: &dyn Fn(&mut Url)| {
        let mut url = redirect.clone();
        url.query_pairs_mut()
            .append_pair("code", "pasted-secret-code")
            .append_pair("state", &state);
        edit(&mut url);
        url.to_string()
    };

    let cases: Vec<(&str, String, &str)> = vec![
        (
            "another path on this desk's port",
            variant(&|url| url.set_path("/callback")),
            "did not match the pending native sign-in",
        ),
        (
            "another server's callback path",
            variant(&|url| url.set_path("/oauth/callback/another-server")),
            "did not match the pending native sign-in",
        ),
        (
            "another host",
            variant(&|url| url.set_host(Some("localhost")).expect("host")),
            "did not match the pending native sign-in",
        ),
        (
            "another port",
            variant(&|url| {
                let port = url.port().expect("port");
                url.set_port(Some(port.wrapping_add(1).max(1024)))
                    .expect("port");
            }),
            "did not match the pending native sign-in",
        ),
        (
            "a fragment",
            variant(&|url| url.set_fragment(Some("frag"))),
            "did not match the pending native sign-in",
        ),
        (
            "another sign-in's state",
            {
                let mut url = redirect.clone();
                url.query_pairs_mut()
                    .append_pair("code", "pasted-secret-code")
                    .append_pair("state", "someone-elses-state");
                url.to_string()
            },
            "state did not match",
        ),
        (
            "no state",
            {
                let mut url = redirect.clone();
                url.query_pairs_mut()
                    .append_pair("code", "pasted-secret-code");
                url.to_string()
            },
            "did not include state",
        ),
        (
            "the authorization page instead of the callback",
            format!(
                "{}&code=pasted-secret-code",
                started["authorizationUrl"]
                    .as_str()
                    .expect("authorization URL")
            ),
            "did not match the pending native sign-in",
        ),
        (
            "something that is not an address",
            "pasted-secret-code and some words".to_string(),
            "",
        ),
    ];
    for (what, pasted, expected) in cases {
        let error = service.complete(login_id, &pasted).await.expect_err(what);
        assert!(error.contains(expected), "{what}: {error}");
        assert!(
            !error.contains("pasted-secret-code") && !error.contains(&pasted),
            "{what}: an answer repeated the address: {error}"
        );
    }

    // A login this desk does not have is refused without touching another.
    let good = variant(&|_| {});
    let error = service
        .complete("not-a-login", &good)
        .await
        .expect_err("unknown login");
    assert!(error.contains("no longer pending"), "{error}");
    assert!(!error.contains("pasted-secret-code"), "{error}");

    // None of that spent the sign-in: it is still waiting, nothing was
    // exchanged, and the right address finishes it.
    assert_eq!(
        service.status(&server).await.expect("pending status")["status"],
        "pending"
    );
    assert_eq!(mock.state.code_exchanges.load(Ordering::SeqCst), 0);
    let signed_in = service
        .complete(login_id, &good.replace("pasted-secret-code", "good-code"))
        .await
        .expect("the right address finishes it");
    assert_eq!(signed_in["status"], "signed_in");
    assert_eq!(mock.state.code_exchanges.load(Ordering::SeqCst), 1);
    let _ = std::fs::remove_dir_all(root);
}
