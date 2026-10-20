use super::*;

mod vault;
use axum::{Router, routing::get, routing::post};
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use bytes::Bytes;
use rig::http_client::{self, HttpClientExt, LazyBody, MultipartForm, StreamingResponse};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vault::LoginDir;

/// The auth servers Rig addresses, played by one local stub: ChatGPT's token
/// endpoint and Copilot's token exchange, each counting what reached it.
struct AuthServer {
    base: String,
    refreshes: Arc<AtomicUsize>,
    exchanges: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}

impl AuthServer {
    /// ChatGPT's token endpoint spends `refresh_token` once: the first
    /// refresh with it is answered with fresh tokens, and any later one is
    /// refused as a spent token, as OpenAI refuses a rotated one.
    async fn serve() -> Self {
        let refreshes = Arc::new(AtomicUsize::new(0));
        let exchanges = Arc::new(AtomicUsize::new(0));
        let counted = refreshes.clone();
        let exchanged = exchanges.clone();
        let app = Router::new()
            .route(
                "/oauth/token",
                post(move |body: Bytes| {
                    let counted = counted.clone();
                    async move {
                        let form: HashMap<String, String> =
                            url::form_urlencoded::parse(&body).into_owned().collect();
                        assert_eq!(form["grant_type"], "refresh_token");
                        // Wide enough that a second client asks while the
                        // first is still being answered.
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        if form["refresh_token"] == "refresh-1"
                            && counted.fetch_add(1, Ordering::SeqCst) == 0
                        {
                            (
                                http::StatusCode::OK,
                                json!({"access_token": access_token("access-2", 3600),
                                    "refresh_token": "refresh-2"})
                                .to_string(),
                            )
                        } else {
                            (
                                http::StatusCode::BAD_REQUEST,
                                json!({"error": "invalid_grant"}).to_string(),
                            )
                        }
                    }
                }),
            )
            .route(
                "/copilot_internal/v2/token",
                get(move || {
                    exchanged.fetch_add(1, Ordering::SeqCst);
                    async { http::StatusCode::UNAUTHORIZED }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            base,
            refreshes,
            exchanges,
            server,
        }
    }

    fn http(&self) -> ToStub {
        ToStub {
            base: self.base.clone(),
            http: ReqwestClient::default(),
        }
    }
}

impl Drop for AuthServer {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// Sends every request Rig makes to the stub, keeping its path.
#[derive(Clone)]
struct ToStub {
    base: String,
    http: ReqwestClient,
}

impl ToStub {
    fn moved<T>(&self, request: http::Request<T>) -> http::Request<T> {
        let (mut parts, body) = request.into_parts();
        let path = parts
            .uri
            .path_and_query()
            .map_or("/", http::uri::PathAndQuery::as_str);
        parts.uri = format!("{}{path}", self.base)
            .parse()
            .expect("the stub's address and a request path make a URI");
        http::Request::from_parts(parts, body)
    }
}

impl HttpClientExt for ToStub {
    fn send<T, U>(
        &self,
        request: http::Request<T>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        self.http.send(self.moved(request))
    }

    fn send_multipart<U>(
        &self,
        request: http::Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        self.http.send_multipart(self.moved(request))
    }

    fn send_streaming<T>(
        &self,
        request: http::Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + Send
    where
        T: Into<Bytes> + Send,
    {
        self.http.send_streaming(self.moved(request))
    }
}

/// A ChatGPT access token, as OpenAI issues one: a JWT whose claims carry
/// its expiry and the account it belongs to.
fn access_token(name: &str, lifetime: i64) -> String {
    let claims = json!({
        "exp": chrono::Utc::now().timestamp() + lifetime,
        "https://api.openai.com/auth": {"chatgpt_account_id": "account-1"},
    });
    let claims = BASE64_URL_SAFE_NO_PAD.encode(claims.to_string());
    format!("e30.{claims}.{name}")
}

/// What Rig 0.42 left in a ChatGPT login directory: `auth.json`, every
/// field written out.
fn chatgpt_0_42(dir: &Path, access: &str, expires_at: i64) -> String {
    let record = json!({
        "access_token": access,
        "refresh_token": "refresh-1",
        "id_token": null,
        "expires_at": expires_at,
        "account_id": "account-1",
    })
    .to_string();
    std::fs::write(dir.join(CHATGPT_RECORD), &record).unwrap();
    record
}

/// What Rig 0.42 left in a Copilot login directory: the GitHub token as
/// plain text, and the API token exchanged for it, bound to it by the
/// fingerprint Rig takes of the GitHub token.
fn copilot_0_42(dir: &Path, github_token: &str) -> Value {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    github_token.hash(&mut hasher);
    let record = json!({
        "token": "copilot-api-token",
        "expires_at": chrono::Utc::now().timestamp() + 3600,
        "endpoints": {"api": "https://api.individual.githubcopilot.com"},
        "bootstrap_token_fingerprint": format!("{:016x}", hasher.finish()),
    });
    std::fs::write(dir.join(LEGACY_GITHUB_TOKEN), format!("{github_token}\n")).unwrap();
    std::fs::write(dir.join(LEGACY_API_KEY), record.to_string()).unwrap();
    record
}

/// A Copilot sign-in made before Rig 0.45 is the same sign-in after it: its
/// two files become `copilot.json`, field for field, and the live API token
/// in it is served with no exchange and no new sign-in.
#[tokio::test]
async fn rig_login_a_0_42_copilot_dir_migrates_to_copilot_json_and_stays_signed_in() {
    let auth = AuthServer::serve().await;
    let dir = LoginDir::new(Arc::new(crate::credentials::MemoryStore::default()));
    let legacy = copilot_0_42(dir.path(), "github-token-1");

    let context = copilot(dir.path())
        .auth_context(&auth.http())
        .await
        .unwrap();
    assert_eq!(context.api_key.expose(), "copilot-api-token");
    assert_eq!(
        context.api_base.as_deref(),
        Some("https://api.individual.githubcopilot.com")
    );
    assert_eq!(auth.exchanges.load(Ordering::SeqCst), 0, "no exchange");

    let record: Value = serde_json::from_str(
        &LoginDirStore::new(dir.path())
            .load(copilot::PROVIDER_NAME)
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(record["github_token"], "github-token-1");
    for field in [
        "token",
        "expires_at",
        "endpoints",
        "bootstrap_token_fingerprint",
    ] {
        assert_eq!(record[field], legacy[field], "{field}");
    }
    assert!(!dir.path().join(LEGACY_GITHUB_TOKEN).exists());
    assert!(!dir.path().join(LEGACY_API_KEY).exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(COPILOT_RECORD))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // Moved once: a second store over the directory reads the record as it is.
    let again = LoginDirStore::new(dir.path())
        .load(copilot::PROVIDER_NAME)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&again).unwrap(), record);
}

/// The old files are removed only once the new reference is on disk: a write
/// that fails leaves the sign-in where it was, to be moved on a later read.
#[cfg(unix)]
#[tokio::test]
async fn rig_login_a_failed_copilot_write_keeps_the_0_42_files() {
    use std::os::unix::fs::PermissionsExt;
    let dir = LoginDir::new(Arc::new(crate::credentials::MemoryStore::default()));
    copilot_0_42(dir.path(), "github-token-1");
    let store = LoginDirStore::new(dir.path());

    let readable = std::fs::Permissions::from_mode(0o500);
    std::fs::set_permissions(dir.path(), readable).unwrap();
    let failed = store.load(copilot::PROVIDER_NAME).await;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        failed.is_err(),
        "a record that cannot be written is an error"
    );
    assert!(!dir.path().join(COPILOT_RECORD).exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join(LEGACY_GITHUB_TOKEN)).unwrap(),
        "github-token-1\n"
    );
    assert!(dir.path().join(LEGACY_API_KEY).exists());

    let moved = store.load(copilot::PROVIDER_NAME).await.unwrap().unwrap();
    assert!(moved.contains("github-token-1"));
    assert!(!dir.path().join(LEGACY_GITHUB_TOKEN).exists());
    assert!(!dir.path().join(LEGACY_API_KEY).exists());
}

/// A ChatGPT sign-in made before Rig 0.45 keeps its live token without a
/// refresh when its plaintext record moves to credential storage.
#[tokio::test]
async fn rig_login_a_0_42_chatgpt_auth_json_keeps_its_tokens() {
    let auth = AuthServer::serve().await;
    let dir = LoginDir::new(Arc::new(crate::credentials::MemoryStore::default()));
    let access = access_token("access-1", 3600);
    let written = chatgpt_0_42(dir.path(), &access, chrono::Utc::now().timestamp() + 3600);

    let context = chatgpt(dir.path())
        .auth_context(&auth.http())
        .await
        .unwrap();
    assert_eq!(context.api_key.expose(), access);
    assert_eq!(context.account_id.as_deref(), Some("account-1"));
    assert_eq!(auth.refreshes.load(Ordering::SeqCst), 0, "no refresh");
    assert_eq!(
        LoginDirStore::new(dir.path())
            .load(chatgpt::PROVIDER_NAME)
            .await
            .unwrap()
            .unwrap(),
        written
    );
}

/// Every client of one sign-in that finds its token expired at the same
/// moment refreshes it once between them: the clients the registry hands
/// out share one authenticator, and another authenticator over the same
/// directory waits on the record's lock and reads what the first saved. A
/// rotated refresh token spent twice would sign the person out.
#[tokio::test]
async fn rig_login_two_clients_refreshing_at_once_spend_the_refresh_token_once() {
    let auth = AuthServer::serve().await;
    let dir = LoginDir::new(Arc::new(crate::credentials::MemoryStore::default()));
    chatgpt_0_42(
        dir.path(),
        &access_token("access-1", -60),
        chrono::Utc::now().timestamp() - 60,
    );
    let turn = chatgpt(dir.path());
    let voice = chatgpt(dir.path());
    let sign_in = chatgpt::auth::Authenticator::new(
        chatgpt::auth::AuthSource::OAuth,
        LoginDirStore::new(dir.path()),
        DeviceCodeHandler::default(),
        false,
    );
    let http = auth.http();
    let contexts = futures_util::future::join_all([
        turn.auth_context(&http),
        voice.auth_context(&http),
        sign_in.auth_context(&http),
    ])
    .await;
    assert_eq!(auth.refreshes.load(Ordering::SeqCst), 1, "refreshed once");
    for context in contexts {
        let context = context.unwrap();
        assert!(context.api_key.expose().ends_with(".access-2"));
        assert_eq!(context.account_id.as_deref(), Some("account-1"));
    }
    let saved: Value = serde_json::from_str(
        &LoginDirStore::new(dir.path())
            .load(chatgpt::PROVIDER_NAME)
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(saved["refresh_token"], "refresh-2");
    assert!(
        saved["access_token"]
            .as_str()
            .unwrap()
            .ends_with(".access-2")
    );
}
