use super::*;
use crate::credentials::MemoryStore;
use axum::{Router, routing::post};
use bytes::Bytes;
use serde_json::{Value, json};
use std::collections::HashMap;

const INSTALL_ID: &str = "e61bbe28-07ef-466d-8e5d-a344f94ab305";
const GRANTED: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

/// A vault with one login begun in it, on a store of its own.
struct Desk {
    root: tempfile::TempDir,
    vault: crate::vault::Vault,
    id: String,
    dir: PathBuf,
}

impl Desk {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let log = crate::log::Log::open(root.path());
        let vault = crate::vault::Vault::open_with_store(
            root.path(),
            log,
            Arc::new(MemoryStore::default()),
        )
        .unwrap();
        let (id, dir) = vault.begin_login().unwrap();
        Self {
            root,
            vault,
            id,
            dir,
        }
    }
}

/// OpenAI's token endpoint, played locally: each request's form is kept,
/// and each is answered with the next of `answers`.
struct TokenServer {
    endpoints: Endpoints,
    forms: Arc<Mutex<Vec<HashMap<String, String>>>>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for TokenServer {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl TokenServer {
    async fn serve(answers: Vec<Value>, delay: Duration) -> Self {
        let forms = Arc::new(Mutex::new(Vec::new()));
        let answers = Arc::new(Mutex::new(answers.into_iter()));
        let kept = forms.clone();
        let app = Router::new().route(
            TOKEN_PATH,
            post(move |body: Bytes| {
                let kept = kept.clone();
                let answers = answers.clone();
                async move {
                    let form = url::form_urlencoded::parse(&body).into_owned().collect();
                    kept.lock().unwrap().push(form);
                    tokio::time::sleep(delay).await;
                    let answer = answers.lock().unwrap().next();
                    match answer {
                        Some(answer) => (http::StatusCode::OK, answer.to_string()),
                        None => (
                            http::StatusCode::BAD_REQUEST,
                            json!({"error": "refresh_token_invalidated"}).to_string(),
                        ),
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            endpoints: Endpoints::at(&origin, 0),
            forms,
            server,
        }
    }

    fn forms(&self) -> Vec<HashMap<String, String>> {
        self.forms.lock().unwrap().clone()
    }
}

fn tokens(access: &str, refresh: &str, scope: &str) -> Value {
    json!({
        "access_token": access,
        "refresh_token": refresh,
        "expires_in": 3600,
        "id_token": "id-token",
        "scope": scope,
    })
}

/// A sign-in running on its own task, with the page it opened and where a
/// paste goes.
struct Signing {
    prompt: Prompt,
    pastes: mpsc::Sender<Paste>,
    done: tokio::task::JoinHandle<Result<(), String>>,
}

async fn start(desk: &Desk, endpoints: &Endpoints) -> Result<Signing, String> {
    let (shown, prompt) = oneshot::channel();
    let (pastes, pasted) = mpsc::channel(4);
    let dir = desk.dir.clone();
    let endpoints = endpoints.clone();
    let done = tokio::spawn(async move {
        sign_in(
            &dir,
            INSTALL_ID,
            "Hotline on test-desk",
            &endpoints,
            move |prompt| {
                let _ = shown.send(prompt);
            },
            pasted,
        )
        .await
    });
    match prompt.await {
        Ok(prompt) => Ok(Signing {
            prompt,
            pastes,
            done,
        }),
        Err(_) => Err(done.await.unwrap().unwrap_err()),
    }
}

impl Signing {
    fn param(&self, name: &str) -> String {
        Url::parse(&self.prompt.authorize_url)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default()
    }

    /// The address the browser comes back to, with these parameters.
    fn callback(&self, params: &[(&str, &str)]) -> String {
        let mut url = Url::parse(&self.prompt.redirect_uri).unwrap();
        url.query_pairs_mut().extend_pairs(params);
        url.to_string()
    }

    fn granted(&self, client_id: &str) -> String {
        let state = self.param("state");
        self.callback(&[
            ("code", "the-code"),
            ("state", &state),
            ("client_id", client_id),
        ])
    }

    async fn paste(&self, url: &str) -> Result<(), String> {
        let (reply, verdict) = oneshot::channel();
        self.pastes
            .send(Paste {
                url: url.to_string(),
                reply,
            })
            .await
            .unwrap();
        verdict.await.unwrap()
    }
}

async fn browse(url: &str) -> (u16, String) {
    let response = reqwest::get(url).await.unwrap();
    (response.status().as_u16(), response.text().await.unwrap())
}

#[tokio::test]
async fn a_browser_sign_in_keeps_the_issued_client_and_this_desks_host() {
    let desk = Desk::new();
    let auth = TokenServer::serve(
        vec![tokens("access-1", "refresh-1", GRANTED)],
        Duration::ZERO,
    )
    .await;
    let signing = start(&desk, &auth.endpoints).await.unwrap();

    assert_eq!(signing.param("client_id"), "dynamic_agent_client");
    assert_eq!(signing.param("agent_name_hint"), "Hotline on test-desk");
    assert_eq!(
        signing.param("ext_agent_host_id"),
        format!("urn:uuid:{INSTALL_ID}")
    );
    assert_eq!(signing.param("scope"), GRANTED);
    assert_eq!(signing.param("resource"), "https://api.openai.com/v1");
    assert_eq!(signing.param("response_type"), "code");
    assert_eq!(signing.param("code_challenge_method"), "S256");
    assert_eq!(signing.param("redirect_uri"), signing.prompt.redirect_uri);
    for random in ["state", "nonce", "code_challenge"] {
        assert!(signing.param(random).len() >= 43, "{random}");
    }
    assert!(
        signing
            .prompt
            .authorize_url
            .starts_with(&format!("{}/api/accounts/authorize?", auth.endpoints.auth))
    );

    let (status, page) = browse(&signing.granted("oaiapp_issued")).await;
    assert_eq!(status, 200, "{page}");
    signing.done.await.unwrap().unwrap();

    let forms = auth.forms();
    assert_eq!(forms.len(), 1);
    assert_eq!(forms[0]["grant_type"], "authorization_code");
    assert_eq!(forms[0]["client_id"], "oaiapp_issued");
    assert_eq!(forms[0]["code"], "the-code");
    assert_eq!(forms[0]["redirect_uri"], signing.prompt.redirect_uri);
    assert_eq!(forms[0]["resource"], "https://api.openai.com/v1");
    assert!(forms[0]["code_verifier"].len() >= 43);

    // The vault round trip: a reference on disk, the tokens in the store,
    // and the same record read back.
    assert!(is_agent(&desk.dir));
    let on_disk = std::fs::read_to_string(desk.dir.join(AGENT_RECORD)).unwrap();
    assert!(!on_disk.contains("access-1") && !on_disk.contains("oaiapp_issued"));
    let record = load(&desk.dir).await.unwrap().unwrap();
    assert_eq!(record.access_token, "access-1");
    assert_eq!(record.refresh_token, "refresh-1");
    assert_eq!(record.client_id, "oaiapp_issued");
    assert_eq!(record.host_id, format!("urn:uuid:{INSTALL_ID}"));
    assert!(record.scopes.iter().any(|scope| scope == DIRECT_SCOPE));
    assert!(record.fresh());
    let credential = desk
        .vault
        .finish_login(&desk.id, "openai-codex", "ChatGPT")
        .unwrap();
    let context = super::super::chatgpt(&desk.dir).context().await.unwrap();
    assert_eq!(context.api_key.expose(), "access-1");
    assert_eq!(context.account_id, None);

    // Deleting the login takes the tokens out of the store too.
    desk.vault.delete(&credential.id).unwrap();
    assert!(!desk.dir.exists());
}

#[tokio::test]
async fn a_callback_from_another_attempt_is_refused_and_the_sign_in_waits_on() {
    let desk = Desk::new();
    let auth = TokenServer::serve(
        vec![tokens("access-1", "refresh-1", GRANTED)],
        Duration::ZERO,
    )
    .await;
    let signing = start(&desk, &auth.endpoints).await.unwrap();

    let stale = signing.callback(&[
        ("code", "stale-code"),
        ("state", "another-attempt"),
        ("client_id", "oaiapp_stale"),
    ]);
    let (status, page) = browse(&stale).await;
    assert_eq!(status, 400);
    assert!(page.contains("another sign-in attempt"), "{page}");
    let (status, _) = browse(&signing.callback(&[("code", "no-state")])).await;
    assert_eq!(status, 400);
    let elsewhere = signing
        .prompt
        .redirect_uri
        .replace(CALLBACK_PATH, "/favicon.ico");
    assert_eq!(browse(&elsewhere).await.0, 404);
    assert!(!signing.done.is_finished());
    assert_eq!(auth.forms().len(), 0, "nothing was exchanged");

    assert_eq!(browse(&signing.granted("oaiapp_issued")).await.0, 200);
    signing.done.await.unwrap().unwrap();
    assert_eq!(auth.forms()[0]["code"], "the-code");
}

#[tokio::test]
async fn a_callback_without_an_issued_client_id_fails_before_any_exchange() {
    let desk = Desk::new();
    let auth = TokenServer::serve(
        vec![tokens("access-1", "refresh-1", GRANTED)],
        Duration::ZERO,
    )
    .await;
    let signing = start(&desk, &auth.endpoints).await.unwrap();
    let state = signing.param("state");
    let (status, _) = browse(&signing.callback(&[("code", "the-code"), ("state", &state)])).await;
    assert_eq!(status, 400);
    let error = signing.done.await.unwrap().unwrap_err();
    assert!(error.contains("issued client ID"), "{error}");
    assert_eq!(auth.forms().len(), 0);
    assert!(!is_agent(&desk.dir));
}

#[tokio::test]
async fn a_declined_sign_in_fails() {
    let desk = Desk::new();
    let auth = TokenServer::serve(Vec::new(), Duration::ZERO).await;
    let signing = start(&desk, &auth.endpoints).await.unwrap();
    let state = signing.param("state");
    browse(&signing.callback(&[("error", "access_denied"), ("state", &state)])).await;
    let error = signing.done.await.unwrap().unwrap_err();
    assert!(error.contains("access_denied"), "{error}");
}

#[tokio::test]
async fn a_grant_without_direct_use_signs_nobody_in() {
    let desk = Desk::new();
    let narrow = "openid profile email offline_access resource.invoke";
    let auth = TokenServer::serve(
        vec![tokens("access-1", "refresh-1", narrow)],
        Duration::ZERO,
    )
    .await;
    let signing = start(&desk, &auth.endpoints).await.unwrap();
    browse(&signing.granted("oaiapp_issued")).await;
    let error = signing.done.await.unwrap().unwrap_err();
    assert!(error.contains("chatgpt.tokens.use.direct"), "{error}");
    assert!(!is_agent(&desk.dir), "no record without the scope");
}

#[tokio::test]
async fn a_pasted_redirect_signs_in_and_a_wrong_one_is_refused() {
    let desk = Desk::new();
    let auth = TokenServer::serve(
        vec![tokens("access-1", "refresh-1", GRANTED)],
        Duration::ZERO,
    )
    .await;
    let signing = start(&desk, &auth.endpoints).await.unwrap();

    let refused = signing.paste("the-code").await.unwrap_err();
    assert!(refused.contains("whole address"), "{refused}");
    let elsewhere = signing
        .granted("oaiapp_issued")
        .replace("127.0.0.1", "localhost");
    assert!(
        signing
            .paste(&elsewhere)
            .await
            .unwrap_err()
            .contains("must start with")
    );
    let stale = signing.callback(&[
        ("code", "stale"),
        ("state", "another-attempt"),
        ("client_id", "oaiapp_issued"),
    ]);
    assert!(
        signing
            .paste(&stale)
            .await
            .unwrap_err()
            .contains("another sign-in attempt")
    );
    let state = signing.param("state");
    let unissued = signing.callback(&[("code", "the-code"), ("state", &state)]);
    assert!(
        signing
            .paste(&unissued)
            .await
            .unwrap_err()
            .contains("issued client ID")
    );
    assert!(
        !signing.done.is_finished(),
        "a refused paste leaves the sign-in waiting"
    );

    signing
        .paste(&format!("  {}\n", signing.granted("oaiapp_pasted")))
        .await
        .unwrap();
    signing.done.await.unwrap().unwrap();
    assert_eq!(auth.forms()[0]["client_id"], "oaiapp_pasted");
    assert_eq!(
        load(&desk.dir).await.unwrap().unwrap().client_id,
        "oaiapp_pasted"
    );
}

#[tokio::test]
async fn a_port_in_use_fails_clearly() {
    let desk = Desk::new();
    let taken = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = taken.local_addr().unwrap().port();
    let auth = TokenServer::serve(Vec::new(), Duration::ZERO).await;
    let endpoints = Endpoints::at(&auth.endpoints.auth, port);
    let error = start(&desk, &endpoints).await.err().unwrap();
    assert!(error.contains(&format!("Port {port} is in use")), "{error}");
    assert!(error.contains("device code"), "{error}");
}

/// A browser opens spare connections ahead of time. Once the sign-in is
/// over they are closed, so none of them can carry the next sign-in's
/// callback to this one's listener.
#[tokio::test]
async fn spare_browser_connections_close_with_the_sign_in() {
    let desk = Desk::new();
    let auth = TokenServer::serve(
        vec![tokens("access-1", "refresh-1", GRANTED)],
        Duration::ZERO,
    )
    .await;
    let signing = start(&desk, &auth.endpoints).await.unwrap();
    let address = Url::parse(&signing.prompt.redirect_uri).unwrap();
    let port = address.port().unwrap();
    let mut spare = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    assert!(crate::listening::held(port));

    assert_eq!(browse(&signing.granted("oaiapp_issued")).await.0, 200);
    signing.done.await.unwrap().unwrap();

    let mut byte = [0_u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(5), spare.read(&mut byte))
        .await
        .expect("the spare connection was closed");
    assert!(matches!(read, Ok(0) | Err(_)), "{read:?}");
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err()
    );
    assert!(!crate::listening::held(port));
}

async fn signed_in(desk: &Desk, access: &str, refresh: &str, expires_at: i64) {
    let record = Record {
        access_token: access.into(),
        refresh_token: refresh.into(),
        expires_at,
        client_id: "oaiapp_existing".into(),
        host_id: format!("urn:uuid:{INSTALL_ID}"),
        scopes: GRANTED.split(' ').map(str::to_string).collect(),
    };
    save(&desk.dir, &record).await.unwrap();
}

#[tokio::test]
async fn a_refresh_names_the_issued_client_and_the_resource() {
    let desk = Desk::new();
    signed_in(&desk, "old-access", "old-refresh", 0).await;
    let auth = TokenServer::serve(
        vec![tokens("new-access", "new-refresh", GRANTED)],
        Duration::ZERO,
    )
    .await;
    let authenticator = Authenticator::with(&desk.dir, auth.endpoints.clone());

    let before = chrono::Utc::now().timestamp();
    let context = authenticator.auth_context().await.unwrap();
    assert_eq!(context.api_key.expose(), "new-access");
    let forms = auth.forms();
    assert_eq!(forms.len(), 1);
    assert_eq!(forms[0]["grant_type"], "refresh_token");
    assert_eq!(forms[0]["client_id"], "oaiapp_existing");
    assert_eq!(forms[0]["refresh_token"], "old-refresh");
    assert_eq!(forms[0]["resource"], "https://api.openai.com/v1");
    assert!(!forms[0].contains_key("scope"));

    let record = load(&desk.dir).await.unwrap().unwrap();
    assert_eq!(record.refresh_token, "new-refresh");
    assert_eq!(record.client_id, "oaiapp_existing");
    assert_eq!(record.host_id, format!("urn:uuid:{INSTALL_ID}"));
    // An hour, less the margin that refreshes it early.
    assert!(
        (before + 3600 - 180..=chrono::Utc::now().timestamp() + 3600 - 180)
            .contains(&record.expires_at)
    );

    // Fresh now: the next request refreshes nothing.
    authenticator.auth_context().await.unwrap();
    assert_eq!(auth.forms().len(), 1);
}

#[tokio::test]
async fn a_refresh_that_does_not_rotate_the_refresh_token_is_refused() {
    let desk = Desk::new();
    signed_in(&desk, "old-access", "old-refresh", 0).await;
    let mut unrotated = tokens("new-access", "", GRANTED);
    unrotated["refresh_token"] = Value::Null;
    let auth = TokenServer::serve(vec![unrotated], Duration::ZERO).await;
    let error = Authenticator::with(&desk.dir, auth.endpoints.clone())
        .auth_context()
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("refresh_token"), "{error}");
    assert_eq!(
        load(&desk.dir).await.unwrap().unwrap().refresh_token,
        "old-refresh"
    );
}

/// Pi's `refresh_token_invalidated`: a request cancelled while its refresh
/// was out must not throw away the rotated token OpenAI sent back, or the
/// next request spends the old one and the sign-in is gone.
#[tokio::test]
async fn a_refresh_outlives_the_request_that_was_cancelled_waiting_for_it() {
    let desk = Desk::new();
    signed_in(&desk, "old-access", "old-refresh", 0).await;
    let auth = TokenServer::serve(
        vec![tokens("new-access", "new-refresh", GRANTED)],
        Duration::from_millis(300),
    )
    .await;
    let authenticator = Authenticator::with(&desk.dir, auth.endpoints.clone());
    let cancelled =
        tokio::time::timeout(Duration::from_millis(50), authenticator.auth_context()).await;
    assert!(
        cancelled.is_err(),
        "the request gave up before the refresh came back"
    );

    // Another caller waits on the same lock, and finds the rotated token.
    let context = Authenticator::with(&desk.dir, auth.endpoints.clone())
        .auth_context()
        .await
        .unwrap();
    assert_eq!(context.api_key.expose(), "new-access");
    assert_eq!(
        auth.forms().len(),
        1,
        "the old refresh token was spent once"
    );
    assert_eq!(
        load(&desk.dir).await.unwrap().unwrap().refresh_token,
        "new-refresh"
    );
}

#[tokio::test]
async fn a_refresh_after_sign_out_does_not_bring_the_login_back() {
    let desk = Desk::new();
    signed_in(&desk, "old-access", "old-refresh", 0).await;
    let credential = desk
        .vault
        .finish_login(&desk.id, "openai-codex", "ChatGPT")
        .unwrap();
    let auth = TokenServer::serve(
        vec![tokens("new-access", "new-refresh", GRANTED)],
        Duration::from_millis(200),
    )
    .await;
    let authenticator = Authenticator::with(&desk.dir, auth.endpoints.clone());
    let refreshing = tokio::spawn(async move { authenticator.auth_context().await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    desk.vault.revoke(&credential.id).unwrap();
    assert!(refreshing.await.unwrap().is_err());
    assert_eq!(
        load(&desk.dir).await.unwrap().unwrap().access_token,
        "old-access"
    );
}

#[test]
fn the_desk_names_itself_to_chatgpt() {
    let name = agent_name();
    assert!(
        name == "Hotline" || name.starts_with("Hotline on "),
        "{name}"
    );
    assert_eq!(
        agent_host("E61BBE28-07EF-466D-8E5D-A344F94AB305").unwrap(),
        format!("urn:uuid:{INSTALL_ID}")
    );
    assert!(agent_host("not-a-uuid").is_err());
}

#[test]
fn the_install_id_is_made_once_and_kept() {
    let desk = Desk::new();
    let first = desk.vault.agent_install_id().unwrap();
    assert!(uuid::Uuid::parse_str(&first).is_ok());
    assert_eq!(desk.vault.agent_install_id().unwrap(), first);
    let reopened = crate::vault::Vault::open_with_store(
        desk.root.path(),
        crate::log::Log::open(desk.root.path()),
        Arc::new(MemoryStore::default()),
    )
    .unwrap();
    assert_eq!(reopened.agent_install_id().unwrap(), first);
}
