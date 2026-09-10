//! xAI's subscription device flow, with bearer refresh at Rig's HTTP boundary.
//! Rig still constructs requests and decodes responses, including tool streams.

use crate::credentials::CredentialFile;
use bytes::Bytes;
use oauth2::{
    AuthType, ClientId, DeviceAuthorizationUrl, RefreshToken, RequestTokenError, Scope,
    StandardDeviceAuthorizationResponse, TokenResponse, TokenUrl,
    basic::{BasicClient, BasicTokenResponse},
};
use rig::http_client::{
    Error, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
    bearer_auth_header,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex as AsyncMutex;

// Public device client used by Grok CLI and Pi. This is an identifier, not a secret.
const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const DEVICE_URL: &str = "https://auth.x.ai/oauth2/device/code";
const TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
const API_URL: &str = "https://api.x.ai";
const SCOPES: &str = "openid profile email offline_access grok-cli:access api:access";

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not prepare Grok sign-in.".into())
}

// oauth2's bundled adapter uses another reqwest version. Keep the application's
// client and let oauth2 own form encoding, polling and protocol validation.
async fn oauth_request(
    http: &reqwest::Client,
    request: oauth2::HttpRequest,
) -> Result<oauth2::HttpResponse, std::io::Error> {
    let (parts, body) = request.into_parts();
    let response = http
        .request(parts.method, parts.uri.to_string())
        .headers(parts.headers)
        .body(body)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| std::io::Error::other("Grok authorization server could not be reached"))?;
    let mut result = http::Response::builder().status(response.status());
    *result.headers_mut().expect("valid response builder") = response.headers().clone();
    let body = response
        .bytes()
        .await
        .map_err(|_| std::io::Error::other("Grok authorization response could not be read"))?;
    result.body(body.to_vec()).map_err(std::io::Error::other)
}

struct OAuthHttp(reqwest::Client);

impl<'a> oauth2::AsyncHttpClient<'a> for OAuthHttp {
    type Error = std::io::Error;
    type Future = std::pin::Pin<
        Box<dyn Future<Output = Result<oauth2::HttpResponse, Self::Error>> + Send + 'a>,
    >;

    fn call(&'a self, request: oauth2::HttpRequest) -> Self::Future {
        Box::pin(oauth_request(&self.0, request))
    }
}

fn oauth_failure(code: Option<&str>) -> String {
    match code {
        Some("access_denied" | "authorization_denied") => "Grok sign-in was declined. Try again.",
        Some("expired_token") => "Grok sign-in expired. Start sign-in again.",
        Some("invalid_grant") => "Grok sign-in is no longer valid. Sign in again.",
        _ => "Grok authorization failed. Check your connection and try signing in again.",
    }
    .into()
}

fn poll_wait(interval: Duration) -> tokio::time::Sleep {
    // xAI can report zero; use the device-flow default instead of busy polling.
    tokio::time::sleep(if interval.is_zero() {
        Duration::from_secs(5)
    } else {
        interval
    })
}

pub(crate) async fn login(
    token_dir: &CredentialFile,
    emit: impl FnOnce(String, String),
) -> Result<(), String> {
    login_at(token_dir, emit, DEVICE_URL, TOKEN_URL).await
}

async fn login_at(
    token_dir: &CredentialFile,
    emit: impl FnOnce(String, String),
    device_url: &str,
    token_url: &str,
) -> Result<(), String> {
    let client = BasicClient::new(ClientId::new(CLIENT_ID.into()))
        .set_auth_type(AuthType::RequestBody)
        .set_token_uri(TokenUrl::new(token_url.into()).map_err(|_| oauth_failure(None))?)
        .set_device_authorization_url(
            DeviceAuthorizationUrl::new(device_url.into()).map_err(|_| oauth_failure(None))?,
        );
    let send = OAuthHttp(http_client()?);
    let details: StandardDeviceAuthorizationResponse = client
        .exchange_device_code()
        .add_scopes(
            SCOPES
                .split_whitespace()
                .map(|scope| Scope::new(scope.into())),
        )
        .add_extra_param("referrer", "toad")
        .request_async(&send)
        .await
        .map_err(|_| oauth_failure(None))?;
    let verification = details
        .verification_uri_complete()
        .map(|uri| uri.secret().as_str())
        .unwrap_or_else(|| details.verification_uri().as_str());
    let url = url::Url::parse(verification).map_err(|_| oauth_failure(None))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || details.device_code().secret().trim().is_empty()
        || details.user_code().secret().trim().is_empty()
        || details.expires_in().is_zero()
    {
        return Err("Grok returned an invalid sign-in prompt.".into());
    }
    emit(details.user_code().secret().clone(), url.to_string());
    let authorize = async {
        // RFC 8628 requires a wait before the first poll too.
        poll_wait(details.interval()).await;
        client
            .exchange_device_access_token(&details)
            .request_async(&send, poll_wait, None)
            .await
            .map_err(|error| match error {
                RequestTokenError::ServerResponse(response) => {
                    oauth_failure(Some(response.error().as_ref()))
                }
                _ => oauth_failure(None),
            })
    };
    let response = tokio::time::timeout(details.expires_in(), authorize)
        .await
        .map_err(|_| oauth_failure(Some("expired_token")))??;
    Tokens::from_response(response, None)?.save(token_dir)
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    refresh_at: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Tokens {
    fn from_response(
        response: BasicTokenResponse,
        previous: Option<&Self>,
    ) -> Result<Self, String> {
        let access_token = response.access_token().secret().clone();
        let refresh_token = response
            .refresh_token()
            .map(|token| token.secret().clone())
            .or_else(|| previous.map(|tokens| tokens.refresh_token.clone()))
            .ok_or_else(|| "Grok returned no refresh token. Sign in again.".to_string())?;
        let lifetime = response
            .expires_in()
            .unwrap_or(Duration::from_secs(3600))
            .as_secs();
        if access_token.trim().is_empty() || refresh_token.trim().is_empty() || lifetime == 0 {
            return Err("Grok returned invalid sign-in tokens. Sign in again.".into());
        }
        let refresh_at = now()
            .checked_add(lifetime - (lifetime / 10).min(300))
            .ok_or_else(|| oauth_failure(None))?;
        Ok(Self {
            access_token,
            refresh_token,
            refresh_at,
        })
    }

    fn read(dir: &CredentialFile) -> Result<Self, String> {
        let bytes = dir
            .read()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Grok sign-in is missing. Sign in again.".to_string())?;
        let tokens: Self = serde_json::from_slice(&bytes)
            .map_err(|_| "Grok sign-in is unreadable. Sign in again.".to_string())?;
        if tokens.access_token.trim().is_empty() || tokens.refresh_token.trim().is_empty() {
            return Err("Grok sign-in is incomplete. Sign in again.".into());
        }
        Ok(tokens)
    }

    fn save(&self, dir: &CredentialFile) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|_| oauth_failure(None))?;
        dir.write(&bytes)
            .map_err(|error| format!("Could not save Grok sign-in: {error}"))
    }
}

struct TokenStore {
    dir: CredentialFile,
    refresh: AsyncMutex<()>,
}

impl TokenStore {
    fn shared(dir: &CredentialFile) -> Arc<Self> {
        // Each teammate builds its own Rig client. They must share the refresh
        // lock or a rotated refresh token can be spent twice.
        static STORES: OnceLock<Mutex<HashMap<PathBuf, Weak<TokenStore>>>> = OnceLock::new();
        let mut stores = STORES
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        stores.retain(|_, store| store.strong_count() > 0);
        if let Some(store) = stores.get(dir.path()).and_then(Weak::upgrade) {
            return store;
        }
        let store = Arc::new(Self {
            dir: dir.clone(),
            refresh: AsyncMutex::new(()),
        });
        stores.insert(dir.path().into(), Arc::downgrade(&store));
        store
    }

    async fn tokens(
        &self,
        http: &reqwest::Client,
        token_url: &str,
        rejected: Option<&Tokens>,
    ) -> Result<Tokens, String> {
        let _refresh = self.refresh.lock().await;
        let current = Tokens::read(&self.dir)?;
        let needs_refresh = rejected.map_or(current.refresh_at <= now(), |old| old == &current);
        if !needs_refresh {
            return Ok(current);
        }
        let client = BasicClient::new(ClientId::new(CLIENT_ID.into()))
            .set_auth_type(AuthType::RequestBody)
            .set_token_uri(TokenUrl::new(token_url.into()).map_err(|_| oauth_failure(None))?);
        let refresh_token = RefreshToken::new(current.refresh_token.clone());
        let send = OAuthHttp(http.clone());
        let response = client
            .exchange_refresh_token(&refresh_token)
            .request_async(&send)
            .await
            .map_err(|error| match error {
                RequestTokenError::ServerResponse(response) => {
                    oauth_failure(Some(response.error().as_ref()))
                }
                _ => oauth_failure(None),
            })?;
        let next = Tokens::from_response(response, Some(&current))?;
        next.save(&self.dir)?;
        Ok(next)
    }
}

#[derive(Clone, Default)]
pub(crate) struct SubscriptionHttp {
    http: reqwest::Client,
    store: Option<Arc<TokenStore>>,
    token_url: String,
    api_url: String,
}

impl std::fmt::Debug for SubscriptionHttp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GrokSubscriptionHttp")
            .finish_non_exhaustive()
    }
}

fn transport_error(message: String) -> Error {
    Error::Instance(Box::new(std::io::Error::other(message)))
}

fn status(error: &Error) -> Option<http::StatusCode> {
    match error {
        Error::InvalidStatusCode(code)
        | Error::InvalidStatusCodeWithMessage(code, _)
        | Error::InvalidStatusCodeWithDetails { status: code, .. } => Some(*code),
        _ => None,
    }
}

fn subscription_error(error: Error) -> Error {
    let message = match status(&error).map(|code| code.as_u16()) {
        Some(401) => "Grok rejected your subscription sign-in after refresh. Sign in again.",
        Some(402) => "Grok requires a subscription with access to this model. Check your plan.",
        Some(403) => {
            "Your Grok subscription does not allow this request or model. Check your plan or choose another model."
        }
        Some(429) => {
            "Grok is rate limiting this subscription or its usage limit has been reached. Try again later."
        }
        _ => return error,
    };
    match error {
        Error::InvalidStatusCodeWithDetails {
            status, headers, ..
        } => Error::InvalidStatusCodeWithDetails {
            status,
            headers,
            body: message.into(),
        },
        other => Error::InvalidStatusCodeWithMessage(
            status(&other).expect("matched status"),
            message.into(),
        ),
    }
}

impl SubscriptionHttp {
    fn new(dir: &CredentialFile) -> Result<Self, String> {
        Tokens::read(dir)?;
        Ok(Self {
            http: http_client()?,
            store: Some(TokenStore::shared(dir)),
            token_url: TOKEN_URL.into(),
            api_url: API_URL.into(),
        })
    }

    async fn request<T, R, F, Fut>(&self, mut request: Request<T>, send: F) -> Result<R, Error>
    where
        T: Clone,
        F: Fn(Request<T>) -> Fut,
        Fut: Future<Output = Result<R, Error>>,
    {
        let origin = url::Url::parse(&request.uri().to_string())
            .map_err(|_| transport_error("Invalid Grok request URL.".into()))?;
        if origin.origin().ascii_serialization() != self.api_url {
            return Err(transport_error(
                "Refusing to send Grok subscription tokens to another server.".into(),
            ));
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| transport_error("Grok needs a subscription sign-in.".into()))?;
        let tokens = store
            .tokens(&self.http, &self.token_url, None)
            .await
            .map_err(transport_error)?;
        bearer_auth_header(request.headers_mut(), &tokens.access_token)?;
        match send(request.clone()).await {
            Err(error) if status(&error) == Some(http::StatusCode::UNAUTHORIZED) => {
                let refreshed = store
                    .tokens(&self.http, &self.token_url, Some(&tokens))
                    .await
                    .map_err(transport_error)?;
                bearer_auth_header(request.headers_mut(), &refreshed.access_token)?;
                send(request).await.map_err(subscription_error)
            }
            result => result.map_err(subscription_error),
        }
    }
}

impl HttpClientExt for SubscriptionHttp {
    fn send<T, U>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = Result<Response<LazyBody<U>>, Error>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        let this = self.clone();
        let request = request.map(Into::into);
        async move {
            this.request(request, |req| HttpClientExt::send(&this.http, req))
                .await
        }
    }

    fn send_multipart<U>(
        &self,
        request: Request<MultipartForm>,
    ) -> impl Future<Output = Result<Response<LazyBody<U>>, Error>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        let this = self.clone();
        async move {
            this.request(request, |req| {
                HttpClientExt::send_multipart(&this.http, req)
            })
            .await
        }
    }

    fn send_streaming<T>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = Result<StreamingResponse, Error>> + Send
    where
        T: Into<Bytes> + Send,
    {
        let request = request.map(Into::into);
        async move {
            self.request(request, |req| {
                HttpClientExt::send_streaming(&self.http, req)
            })
            .await
        }
    }
}

pub(crate) fn client(
    dir: &CredentialFile,
) -> Result<rig::providers::xai::Client<SubscriptionHttp>, String> {
    rig::providers::xai::Client::builder()
        // The transport supplies the current bearer immediately before sending.
        .api_key("oauth")
        .http_client(SubscriptionHttp::new(dir)?)
        .build()
        .map_err(|_| "Could not prepare the Grok subscription client.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn record(dir: &Path) -> CredentialFile {
        crate::credentials::CredentialFiles::new(dir.into(), crate::credentials::default_store())
            .file(dir.join("auth.json"))
    }
    use axum::{Router, response::IntoResponse, routing::post};
    use futures_util::StreamExt;
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("toad-grok-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&dir).unwrap();
            Self(dir)
        }
        fn tokens(&self, expired: bool) {
            Tokens {
                access_token: "old-access".into(),
                refresh_token: "old-refresh".into(),
                refresh_at: if expired { 0 } else { now() + 3600 },
            }
            .save(&record(&self.0))
            .unwrap();
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn serve(app: Router) -> (String, super::super::CallbackServer) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = super::super::CallbackServer(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        (url, task)
    }

    fn form(body: &[u8]) -> HashMap<String, String> {
        url::form_urlencoded::parse(body).into_owned().collect()
    }

    fn json_response(value: Value) -> impl IntoResponse {
        ([("content-type", "application/json")], value.to_string())
    }

    fn device(expires_in: u64) -> Value {
        json!({"device_code":"private-device", "user_code":"USER-CODE",
            "verification_uri":"https://auth.x.ai/activate", "expires_in":expires_in,
            "verification_uri_complete":"https://auth.x.ai/activate?code=USER-CODE", "interval":1})
    }

    fn issued() -> Value {
        json!({"access_token":"new-access", "refresh_token":"new-refresh",
            "token_type":"Bearer", "expires_in":3600})
    }

    fn transport(dir: &Path, url: &str) -> SubscriptionHttp {
        let mut http = SubscriptionHttp::new(&record(dir)).unwrap();
        http.api_url = url.into();
        http.token_url = format!("{url}/token");
        http
    }

    fn request(url: &str) -> Request<Bytes> {
        Request::builder()
            .method("POST")
            .uri(format!("{url}/v1/responses"))
            .header("authorization", "Bearer oauth")
            .body(Bytes::from_static(b"{}"))
            .unwrap()
    }

    #[tokio::test]
    async fn native_rig_xai_client_uses_subscription_auth_and_responses_format() {
        use rig::prelude::*;
        let scratch = Scratch::new();
        scratch.tokens(false);
        let app = Router::new().route("/v1/responses", post(|headers: http::HeaderMap, bytes: Bytes| async move {
            assert_eq!(headers["authorization"], "Bearer old-access");
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(request["model"], "grok-4");
            assert!(request["input"].to_string().contains("Say hello"));
            assert!(request.get("messages").is_none());
            json_response(json!({"id":"resp_1", "object":"response", "created_at":1,
                "status":"completed", "model":"grok-4", "output":[{"type":"message", "id":"msg_1", "role":"assistant", "status":"completed",
                    "content":[{"type":"output_text", "text":"Hello from Grok.", "annotations":[]}]}],
                "usage":{"input_tokens":3, "output_tokens":4, "total_tokens":7}}))
        }));
        let (url, _server) = serve(app).await;
        let client = rig::providers::xai::Client::builder()
            .api_key("oauth")
            .base_url(&url)
            .http_client(transport(&scratch.0, &url))
            .build()
            .unwrap();
        let agent = client.agent("grok-4").build();
        assert_eq!(agent.prompt("Say hello").await.unwrap(), "Hello from Grok.");
    }

    #[tokio::test]
    async fn device_login_obeys_pending_and_saves_only_private_tokens() {
        let scratch = Scratch::new();
        let polls = Arc::new(AtomicUsize::new(0));
        let count = polls.clone();
        let app = Router::new()
            .route(
                "/device",
                post(|body: Bytes| async move {
                    let fields = form(&body);
                    assert_eq!(fields["client_id"], CLIENT_ID);
                    assert_eq!(fields["scope"], SCOPES);
                    assert_eq!(fields["referrer"], "toad");
                    json_response(device(30))
                }),
            )
            .route(
                "/token",
                post(move |body: Bytes| {
                    let count = count.clone();
                    async move {
                        let fields = form(&body);
                        assert_eq!(
                            fields["grant_type"],
                            "urn:ietf:params:oauth:grant-type:device_code"
                        );
                        assert_eq!(fields["device_code"], "private-device");
                        assert_eq!(fields["client_id"], CLIENT_ID);
                        if count.fetch_add(1, Ordering::SeqCst) == 0 {
                            (
                                http::StatusCode::BAD_REQUEST,
                                json_response(json!({"error":"authorization_pending"})),
                            )
                        } else {
                            (http::StatusCode::OK, json_response(issued()))
                        }
                    }
                }),
            );
        let (url, _server) = serve(app).await;
        let started = std::time::Instant::now();
        login_at(
            &record(&scratch.0),
            |code, url| {
                assert_eq!(code, "USER-CODE");
                assert_eq!(url, "https://auth.x.ai/activate?code=USER-CODE");
                assert!(!url.contains("private-device"));
            },
            &format!("{url}/device"),
            &format!("{url}/token"),
        )
        .await
        .unwrap();
        assert!(started.elapsed() >= Duration::from_secs(2));
        assert_eq!(polls.load(Ordering::SeqCst), 2);
        let saved = Tokens::read(&record(&scratch.0)).unwrap();
        assert_eq!(saved.access_token, "new-access");
        assert_eq!(saved.refresh_token, "new-refresh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(scratch.0.join("auth.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn declined_expired_and_cancelled_logins_never_save_tokens() {
        for (error, expected) in [
            ("authorization_denied", "declined"),
            ("expired_token", "expired"),
        ] {
            let scratch = Scratch::new();
            let app = Router::new()
                .route("/device", post(|| async { json_response(device(30)) }))
                .route(
                    "/token",
                    post(move || async move {
                        (
                            http::StatusCode::BAD_REQUEST,
                            json_response(
                                json!({"error":error, "error_description":"private-response"}),
                            ),
                        )
                    }),
                );
            let (url, _server) = serve(app).await;
            let error = login_at(
                &record(&scratch.0),
                |_, _| {},
                &format!("{url}/device"),
                &format!("{url}/token"),
            )
            .await
            .unwrap_err();
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("private-response"));
            assert!(!scratch.0.join("auth.json").exists());
        }
        let scratch = Scratch::new();
        let app = Router::new().route(
            "/device",
            post(|| async {
                let mut prompt = device(30);
                prompt["interval"] = json!(0);
                json_response(prompt)
            }),
        );
        let (url, _server) = serve(app).await;
        let (tx, rx) = tokio::sync::oneshot::channel();
        let dir = scratch.0.clone();
        let task = tokio::spawn(async move {
            login_at(
                &record(&dir),
                |_, _| {
                    tx.send(()).unwrap();
                },
                &format!("{url}/device"),
                &format!("{url}/token"),
            )
            .await
        });
        rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!scratch.0.join("auth.json").exists());
    }

    #[tokio::test]
    async fn unsafe_prompts_and_device_expiry_stop_before_token_exchange() {
        let scratch = Scratch::new();
        let app = Router::new()
            .route(
                "/device",
                post(|| async {
                    let mut value = device(30);
                    value["verification_uri_complete"] = json!("http://example.com/activate");
                    json_response(value)
                }),
            )
            .route(
                "/expires",
                post(|| async {
                    let mut value = device(1);
                    value["interval"] = json!(2);
                    json_response(value)
                }),
            );
        let (url, _server) = serve(app).await;
        let error = login_at(
            &record(&scratch.0),
            |_, _| panic!("unsafe prompt"),
            &format!("{url}/device"),
            &format!("{url}/token"),
        )
        .await
        .unwrap_err();
        assert!(error.contains("invalid"));
        let error = login_at(
            &record(&scratch.0),
            |_, _| {},
            &format!("{url}/expires"),
            &format!("{url}/token"),
        )
        .await
        .unwrap_err();
        assert!(error.contains("expired"), "{error}");
        assert!(!scratch.0.join("auth.json").exists());
    }

    #[tokio::test]
    async fn teammates_refresh_once_and_send_the_new_bearer() {
        let scratch = Scratch::new();
        scratch.tokens(true);
        let refreshes = Arc::new(AtomicUsize::new(0));
        let count = refreshes.clone();
        let app = Router::new()
            .route(
                "/token",
                post(move |body: Bytes| {
                    let count = count.clone();
                    async move {
                        assert_eq!(form(&body)["refresh_token"], "old-refresh");
                        assert_eq!(form(&body)["grant_type"], "refresh_token");
                        count.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        json_response(issued())
                    }
                }),
            )
            .route(
                "/v1/responses",
                post(|headers: http::HeaderMap| async move {
                    assert_eq!(headers["authorization"], "Bearer new-access");
                    json_response(json!({"ok":true}))
                }),
            );
        let (url, _server) = serve(app).await;
        let jobs = (0..8).map(|_| {
            let http = transport(&scratch.0, &url);
            let req = request(&url);
            async move {
                let response = HttpClientExt::send::<_, Bytes>(&http, req).await.unwrap();
                assert_eq!(
                    response.into_body().await.unwrap(),
                    Bytes::from_static(b"{\"ok\":true}")
                );
            }
        });
        futures_util::future::join_all(jobs).await;
        assert_eq!(refreshes.load(Ordering::SeqCst), 1);
        assert_eq!(
            Tokens::read(&record(&scratch.0)).unwrap().refresh_token,
            "new-refresh"
        );
    }

    #[tokio::test]
    async fn unauthorized_stream_refreshes_once_and_preserves_stream_bytes() {
        let scratch = Scratch::new();
        scratch.tokens(false);
        let refreshes = Arc::new(AtomicUsize::new(0));
        let count = refreshes.clone();
        let requests = Arc::new(AtomicUsize::new(0));
        let calls = requests.clone();
        let app = Router::new()
            .route(
                "/token",
                post(move || {
                    count.fetch_add(1, Ordering::SeqCst);
                    async {
                        let mut response = issued();
                        response.as_object_mut().unwrap().remove("refresh_token");
                        json_response(response)
                    }
                }),
            )
            .route(
                "/v1/responses",
                post(move |headers: http::HeaderMap, body: Bytes| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async move {
                        assert_eq!(body, Bytes::from_static(b"{}"));
                        if headers["authorization"] == "Bearer old-access" {
                            (
                                http::StatusCode::UNAUTHORIZED,
                                "old-access must not be displayed",
                            )
                                .into_response()
                        } else {
                            assert_eq!(headers["authorization"], "Bearer new-access");
                            (
                                [("content-type", "text/event-stream")],
                                "data: hello\n\ndata: [DONE]\n\n",
                            )
                                .into_response()
                        }
                    }
                }),
            );
        let (url, _server) = serve(app).await;
        let http = transport(&scratch.0, &url);
        let mut stream = HttpClientExt::send_streaming(&http, request(&url))
            .await
            .unwrap()
            .into_body();
        let mut output = Vec::new();
        while let Some(chunk) = stream.next().await {
            output.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(output, b"data: hello\n\ndata: [DONE]\n\n");
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert_eq!(refreshes.load(Ordering::SeqCst), 1);
        assert_eq!(
            Tokens::read(&record(&scratch.0)).unwrap().refresh_token,
            "old-refresh"
        );
    }

    #[tokio::test]
    async fn subscription_errors_never_retry_as_api_keys() {
        for code in [401, 402, 403, 429] {
            let scratch = Scratch::new();
            scratch.tokens(false);
            let requests = Arc::new(AtomicUsize::new(0));
            let calls = requests.clone();
            let app = Router::new()
                .route("/token", post(|| async { json_response(issued()) }))
                .route(
                    "/v1/responses",
                    post(move |headers: http::HeaderMap| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        async move {
                            assert!(matches!(
                                headers["authorization"].to_str().unwrap(),
                                "Bearer old-access" | "Bearer new-access"
                            ));
                            (
                                http::StatusCode::from_u16(code).unwrap(),
                                "sensitive provider response",
                            )
                        }
                    }),
                );
            let (url, _server) = serve(app).await;
            let http = transport(&scratch.0, &url);
            let error = match HttpClientExt::send::<_, Bytes>(&http, request(&url)).await {
                Ok(_) => panic!("expected subscription refusal"),
                Err(error) => error,
            };
            assert_eq!(status(&error).unwrap().as_u16(), code);
            assert!(error.to_string().contains("subscription"));
            assert!(!error.to_string().contains("sensitive provider response"));
            assert_eq!(
                requests.load(Ordering::SeqCst),
                if code == 401 { 2 } else { 1 }
            );
        }
    }

    #[tokio::test]
    async fn signout_during_refresh_cannot_recreate_tokens_or_send_a_request() {
        let scratch = Scratch::new();
        scratch.tokens(true);
        let dir = scratch.0.clone();
        let app = Router::new().route(
            "/token",
            post(move || {
                let dir = dir.clone();
                async move {
                    std::fs::remove_dir_all(dir).unwrap();
                    json_response(issued())
                }
            }),
        );
        let (url, _server) = serve(app).await;
        let http = transport(&scratch.0, &url);
        let error = match HttpClientExt::send::<_, Bytes>(&http, request(&url)).await {
            Ok(_) => panic!("request must not run after signout"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("save Grok sign-in"));
        assert!(!scratch.0.exists());
    }

    #[tokio::test]
    async fn invalid_refresh_and_wrong_origin_fail_without_exposing_credentials() {
        let scratch = Scratch::new();
        scratch.tokens(true);
        let app = Router::new().route(
            "/token",
            post(|| async {
                (
                    http::StatusCode::BAD_REQUEST,
                    json_response(
                        json!({"error":"invalid_grant", "error_description":"old-refresh"}),
                    ),
                )
            }),
        );
        let (url, _server) = serve(app).await;
        let http = transport(&scratch.0, &url);
        for (req, expected) in [
            (request(&url), "no longer valid"),
            (request("https://example.com"), "another server"),
        ] {
            let error = match HttpClientExt::send::<_, Bytes>(&http, req).await {
                Ok(_) => panic!("must refuse"),
                Err(error) => error,
            };
            assert!(error.to_string().contains(expected), "{error}");
            assert!(!error.to_string().contains("old-refresh"));
        }
        assert!(!format!("{http:?}").contains("old-access"));
    }
}
