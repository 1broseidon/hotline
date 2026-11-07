//! Sign in with ChatGPT the way OpenAI offers it to agents: Hotline asks
//! for a client of its own (`dynamic_agent_client`), named for this desk in
//! the person's ChatGPT connected apps, and the token it gets back is sent
//! straight to the public Responses API on `api.openai.com/v1`.
//!
//! The person signs in in a browser, which comes back to a listener on
//! `127.0.0.1:1455` with the code, the state and the client id OpenAI
//! issued for this sign-in. A browser on another machine cannot reach that
//! listener (a server desk, or a window on a phone), so the address the
//! browser ended on can be pasted instead. Every later refresh names the
//! issued client id, so it is kept with the tokens, and so is the desk's
//! install id, which OpenAI reads as the agent's host. Pi
//! (`badlogic/pi-mono`, `openai-chatgpt.ts`) is the reference.

use super::{AuthContext, live, login_lock, record_lock};
use crate::credentials::CredentialFile;
use oauth2::PkceCodeChallenge;
use rig::providers::chatgpt::auth::AuthError;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use url::Url;

/// The record a login directory keeps an agent sign-in in. Its presence is
/// what makes a ChatGPT login an agent sign-in rather than a Codex one.
pub(crate) const AGENT_RECORD: &str = "agent.json";

/// Every sign-in registers a new client under this id; OpenAI returns the
/// id it issued in the callback.
const DYNAMIC_CLIENT_ID: &str = "dynamic_agent_client";
const AUTH_ORIGIN: &str = "https://auth.openai.com";
const AUTHORIZE_PATH: &str = "/api/accounts/authorize";
const TOKEN_PATH: &str = "/api/accounts/oauth/token";
/// The API the token is for, named in every request to the token endpoint.
pub(crate) const RESOURCE: &str = "https://api.openai.com/v1";
/// The callback port OpenAI allows for this client.
pub(crate) const CALLBACK_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";
/// The scope that lets the token be used on the API directly. A grant
/// without it signs nobody in.
const DIRECT_SCOPE: &str = "chatgpt.tokens.use.direct";
const SCOPE: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// A token is refreshed this long before it expires, so no request starts
/// with one about to lapse.
const EXPIRY_MARGIN_SECS: i64 = 180;
/// How long a refresh may take. It runs to the end even when the request
/// that needed it is cancelled: OpenAI may have rotated the refresh token
/// already, and dropping the answer would leave the login with a spent one.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(30);
/// How much of a callback request is read before it is refused.
const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// Where a sign-in goes: OpenAI's auth server and the callback port, or a
/// test's stub and a free port.
#[derive(Clone, Debug)]
pub(crate) struct Endpoints {
    auth: String,
    port: u16,
}

impl Endpoints {
    fn official() -> Self {
        Self {
            auth: AUTH_ORIGIN.into(),
            port: CALLBACK_PORT,
        }
    }

    /// The endpoints a new sign-in goes to. A harness build can point them
    /// at a local stub, with the callback on a free port; a shipping build
    /// cannot.
    pub(crate) fn for_sign_in() -> Self {
        #[cfg(feature = "test-support")]
        if let Some(origin) =
            crate::provider_fixture::origin(crate::provider_fixture::CHATGPT_SIGN_IN)
        {
            return Self {
                auth: origin,
                port: 0,
            };
        }
        Self::official()
    }

    /// The endpoints the login in `dir` refreshes at. A harness build can
    /// point one fixture login at a local stub; a shipping build cannot.
    fn for_login(dir: &Path) -> Self {
        #[cfg(feature = "test-support")]
        if let Some(origin) = crate::provider_fixture::origin(&dir.to_string_lossy()) {
            return Self {
                auth: origin,
                port: 0,
            };
        }
        let _ = dir;
        Self::official()
    }

    #[cfg(test)]
    pub(crate) fn at(auth: &str, port: u16) -> Self {
        Self {
            auth: auth.trim_end_matches('/').into(),
            port,
        }
    }

    fn token_url(&self) -> String {
        format!("{}{TOKEN_PATH}", self.auth)
    }
}

/// What the vault keeps for an agent sign-in, in the OS credential store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Record {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    /// When the access token stops being used, in Unix seconds: the real
    /// expiry less a margin.
    pub(crate) expires_at: i64,
    /// The client OpenAI issued this sign-in. Every refresh names it.
    pub(crate) client_id: String,
    /// `urn:uuid:` and this desk's install id.
    pub(crate) host_id: String,
    pub(crate) scopes: Vec<String>,
}

impl Record {
    fn fresh(&self) -> bool {
        chrono::Utc::now().timestamp() < self.expires_at
    }

    fn context(&self) -> AuthContext {
        AuthContext {
            api_key: self.access_token.clone().into(),
            account_id: None,
            api_base: None,
        }
    }
}

/// What the token endpoint answers, each field checked before it is kept.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<f64>,
    id_token: Option<String>,
    scope: Option<String>,
}

impl TokenResponse {
    /// The record this answer makes. A refresh must rotate the refresh
    /// token, and every answer must grant direct use: a token without it
    /// would be refused by the API on the first turn.
    fn record(self, client_id: &str, host_id: &str) -> Result<Record, String> {
        let present = |value: Option<String>, field: &str| {
            value
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("OpenAI's token response has no {field}."))
        };
        let access_token = present(self.access_token, "access_token")?;
        let refresh_token = present(self.refresh_token, "refresh_token")?;
        let scope = present(self.scope, "scope")?;
        let expires_in = self
            .expires_in
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .ok_or("OpenAI's token response has no valid expires_in.")?;
        let scopes: Vec<String> = scope.split_whitespace().map(str::to_string).collect();
        if !scopes.iter().any(|scope| scope == DIRECT_SCOPE) {
            return Err(format!(
                "ChatGPT did not grant {DIRECT_SCOPE}, so this sign-in cannot be used with the OpenAI API. Try again, or sign in with a device code."
            ));
        }
        Ok(Record {
            access_token,
            refresh_token,
            expires_at: chrono::Utc::now().timestamp() + expires_in as i64 - EXPIRY_MARGIN_SECS,
            client_id: client_id.to_string(),
            host_id: host_id.to_string(),
            scopes,
        })
    }
}

/// The code a callback carries and the client OpenAI issued with it.
#[derive(Debug, PartialEq)]
pub(crate) struct Grant {
    code: String,
    client_id: String,
}

/// What one request to the callback means for the sign-in.
#[derive(Debug, PartialEq)]
enum Verdict {
    /// Signed in: exchange the code.
    Granted(Grant),
    /// This sign-in failed and will not recover by waiting.
    Failed(String),
    /// Not this sign-in's callback (another path, another attempt's state):
    /// answered, and the sign-in keeps waiting.
    Ignored(&'static str, String),
}

/// What the callback address `target` (path and query) says to the
/// sign-in whose state is `state`. The state is checked first and exactly,
/// so a page left open from an earlier attempt can neither finish nor fail
/// this one.
fn verdict(target: &str, state: &str) -> Verdict {
    let Ok(url) = Url::parse("http://127.0.0.1").and_then(|base| base.join(target)) else {
        return Verdict::Ignored("400 Bad Request", "That is not a sign-in address.".into());
    };
    if url.path() != CALLBACK_PATH {
        return Verdict::Ignored("404 Not Found", "Nothing is here.".into());
    }
    let param = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    if param("state").as_deref() != Some(state) {
        return Verdict::Ignored(
            "400 Bad Request",
            "This page belongs to another sign-in attempt. Return to Hotline and start again."
                .into(),
        );
    }
    if let Some(error) = param("error") {
        return Verdict::Failed(format!("ChatGPT did not sign you in ({error})."));
    }
    let Some(code) = param("code") else {
        return Verdict::Failed("ChatGPT's callback carried no sign-in code.".into());
    };
    let Some(client_id) = param("client_id") else {
        return Verdict::Failed(
            "ChatGPT's callback did not contain an issued client ID, so the sign-in cannot be kept. Try again."
                .into(),
        );
    };
    Verdict::Granted(Grant { code, client_id })
}

/// The grant in an address the person pasted: the whole address the
/// browser ended on, which must be this sign-in's redirect address.
fn pasted(input: &str, redirect_uri: &str, state: &str) -> Result<Grant, String> {
    let url = Url::parse(input.trim())
        .map_err(|_| "Paste the whole address your browser ended on.".to_string())?;
    let expected = Url::parse(redirect_uri).map_err(|error| error.to_string())?;
    if url.origin() != expected.origin() || url.path() != expected.path() {
        return Err(format!(
            "The pasted address must start with {redirect_uri}."
        ));
    }
    let target = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    };
    match verdict(&target, state) {
        Verdict::Granted(grant) => Ok(grant),
        Verdict::Failed(message) | Verdict::Ignored(_, message) => Err(message),
    }
}

/// A pasted address on its way to the sign-in, and where its verdict goes:
/// a paste that is not this sign-in's is refused and the sign-in waits on.
pub(crate) struct Paste {
    pub(crate) url: String,
    pub(crate) reply: oneshot::Sender<Result<(), String>>,
}

/// What the person is shown: the page to sign in on, and the address a
/// paste must start with.
pub(crate) struct Prompt {
    pub(crate) authorize_url: String,
    pub(crate) redirect_uri: String,
}

/// The loopback listener a sign-in's browser comes back to. Closing it
/// stops the listener and closes every connection it accepted. Browsers
/// open spare connections ahead of time; one left open would answer the
/// next sign-in's callback with this one's state, and refuse it. A sign-in
/// dropped part-way (cancelled, timed out) aborts it instead, which closes
/// the same things a moment later.
struct Callback {
    stop: Option<oneshot::Sender<()>>,
    server: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Callback {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
        }
    }
}

impl Callback {
    /// Listens on `port` for this sign-in, and says the address the browser
    /// comes back to. A port someone else holds is refused here, clearly:
    /// carrying on would send the browser's callback to whatever holds it,
    /// which would refuse it as another sign-in's.
    async fn listen(
        port: u16,
        state: String,
        verdicts: mpsc::Sender<Result<Grant, String>>,
    ) -> Result<(Self, String), String> {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AddrInUse {
                    format!(
                        "Port {port} is in use, so ChatGPT can't hand the sign-in back to Hotline. Another sign-in may be waiting on it, such as the Codex CLI's. Finish or cancel it and try again, or sign in with a device code."
                    )
                } else {
                    format!("Could not open the sign-in callback on port {port}: {error}")
                }
            })?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let held = crate::listening::hold(port);
        let (stop, mut stopped) = oneshot::channel();
        let server = tokio::spawn(async move {
            let _held = held;
            // Owned here, so ending this task ends them all.
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut stopped => break,
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { continue };
                        connections.spawn(answer(socket, state.clone(), verdicts.clone()));
                    }
                    Some(_) = connections.join_next(), if !connections.is_empty() => {}
                }
            }
            connections.shutdown().await;
        });
        Ok((
            Self {
                stop: Some(stop),
                server: Some(server),
            },
            format!("http://127.0.0.1:{port}{CALLBACK_PATH}"),
        ))
    }

    /// Stops listening and closes every connection, before it returns.
    async fn close(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(server) = self.server.take() {
            let _ = server.await;
        }
    }
}

/// Answers one connection's request, and hands the sign-in its verdict.
async fn answer(
    mut socket: tokio::net::TcpStream,
    state: String,
    verdicts: mpsc::Sender<Result<Grant, String>>,
) {
    let Some(target) = request_target(&mut socket).await else {
        return;
    };
    let (status, title, message) = match verdict(&target, &state) {
        Verdict::Granted(grant) => {
            let _ = verdicts.send(Ok(grant)).await;
            (
                "200 OK",
                "Signed in",
                "Return to Hotline. You can close this page.".to_string(),
            )
        }
        Verdict::Failed(message) => {
            let _ = verdicts.send(Err(message.clone())).await;
            ("400 Bad Request", "Sign-in failed", message)
        }
        Verdict::Ignored(status, message) => (status, "Not this sign-in", message),
    };
    let body = page(title, &message);
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;
}

/// The path and query of a GET request, read off the socket. Anything else
/// is closed unanswered.
async fn request_target(socket: &mut tokio::net::TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut buffer = [0_u8; 2048];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = socket.read(&mut buffer).await.ok()?;
        if read == 0 || head.len() + read > MAX_REQUEST_BYTES {
            return None;
        }
        head.extend_from_slice(&buffer[..read]);
    }
    let line = String::from_utf8_lossy(&head);
    let mut words = line.lines().next()?.split_whitespace();
    (words.next()? == "GET").then_some(())?;
    words.next().map(str::to_string)
}

fn page(title: &str, message: &str) -> String {
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    format!(
        "<!doctype html><meta charset=utf-8><title>{0}</title><body style=\"font:16px system-ui;margin:3em\"><h1 style=\"font-size:20px\">{0}</h1><p>{1}</p>",
        escape(title),
        escape(message)
    )
}

/// What ChatGPT's connected apps call this desk's sign-in: Hotline, on the
/// machine's name, so two desks signed in to one account are told apart.
pub(crate) fn agent_name() -> String {
    let desk = crate::remote::desktop_name();
    if desk == "Hotline desktop" {
        "Hotline".into()
    } else {
        format!("Hotline on {desk}")
    }
}

/// The agent's host as OpenAI names it: this desk's install id as a URN.
fn agent_host(install_id: &str) -> Result<String, String> {
    uuid::Uuid::parse_str(install_id)
        .map(|id| format!("urn:uuid:{}", id.hyphenated()))
        .map_err(|_| "This desk has no install id for Sign in with ChatGPT.".to_string())
}

fn random() -> String {
    use base64::Engine;
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).expect("the OS random source");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn authorize_url(
    endpoints: &Endpoints,
    redirect_uri: &str,
    host: &str,
    agent_name: &str,
    state: &str,
    nonce: &str,
    challenge: &PkceCodeChallenge,
) -> String {
    let mut url = Url::parse(&format!("{}{AUTHORIZE_PATH}", endpoints.auth))
        .expect("the auth origin is a URL");
    url.query_pairs_mut()
        .append_pair("client_id", DYNAMIC_CLIENT_ID)
        .append_pair("agent_name_hint", agent_name)
        .append_pair("ext_agent_host_id", host)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("resource", RESOURCE)
        .append_pair("scope", SCOPE)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge.as_str())
        .append_pair("code_challenge_method", "S256")
        .append_pair("nonce", nonce);
    url.to_string()
}

/// Signs the new login in `dir` in to ChatGPT as an agent named
/// `agent_name`, on the desk whose install id is `install_id`. `show` is
/// handed the page to open once the callback is listening; `pastes` brings
/// any address the person pastes instead. The record is written only once
/// the grant has been exchanged and checked.
pub(crate) async fn sign_in(
    dir: &Path,
    install_id: &str,
    agent_name: &str,
    endpoints: &Endpoints,
    show: impl FnOnce(Prompt),
    mut pastes: mpsc::Receiver<Paste>,
) -> Result<(), String> {
    let host = agent_host(install_id)?;
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let state = random();
    let nonce = random();
    let (verdicts, mut callbacks) = mpsc::channel(4);
    let (callback, redirect_uri) =
        Callback::listen(endpoints.port, state.clone(), verdicts).await?;
    show(Prompt {
        authorize_url: authorize_url(
            endpoints,
            &redirect_uri,
            &host,
            agent_name,
            &state,
            &nonce,
            &challenge,
        ),
        redirect_uri: redirect_uri.clone(),
    });
    let grant = loop {
        tokio::select! {
            Some(verdict) = callbacks.recv() => break verdict?,
            Some(paste) = pastes.recv() => match pasted(&paste.url, &redirect_uri, &state) {
                Ok(grant) => {
                    let _ = paste.reply.send(Ok(()));
                    break grant;
                }
                Err(error) => {
                    let _ = paste.reply.send(Err(error));
                }
            },
            else => return Err("The sign-in callback closed.".into()),
        }
    };
    callback.close().await;
    let answer = token_request(
        endpoints,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &grant.client_id),
            ("code", &grant.code),
            ("code_verifier", verifier.secret()),
            ("redirect_uri", &redirect_uri),
            ("resource", RESOURCE),
        ],
    )
    .await?;
    // Hotline reads nothing from the ID token; its presence is part of
    // what a complete answer is.
    if answer
        .id_token
        .as_deref()
        .is_none_or(|token| token.trim().is_empty())
    {
        return Err("OpenAI's token response has no id_token.".into());
    }
    let record = answer.record(&grant.client_id, &host)?;
    save(dir, &record).await.map_err(|error| error.to_string())
}

/// A fresh record for `record`, from the token endpoint, under the client
/// it was issued.
async fn refresh(endpoints: &Endpoints, record: &Record) -> Result<Record, String> {
    if record.client_id.trim().is_empty() {
        return Err(
            "This ChatGPT sign-in has no issued client ID. Sign in again under Settings → Providers."
                .into(),
        );
    }
    token_request(
        endpoints,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &record.client_id),
            ("refresh_token", &record.refresh_token),
            ("resource", RESOURCE),
        ],
    )
    .await?
    .record(&record.client_id, &record.host_id)
}

async fn token_request(
    endpoints: &Endpoints,
    form: &[(&str, &str)],
) -> Result<TokenResponse, String> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form)
        .finish();
    let response = reqwest::Client::builder()
        .timeout(REFRESH_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?
        .post(endpoints.token_url())
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|_| "Could not reach OpenAI's sign-in server. Try again.".to_string())?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let reason = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| {
                value["error"]
                    .as_str()
                    .or_else(|| value["error"]["code"].as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        return Err(format!(
            "OpenAI refused the ChatGPT sign-in ({reason}). Sign in again under Settings → Providers."
        ));
    }
    serde_json::from_str(&text).map_err(|_| "OpenAI's token response could not be read.".into())
}

fn file(dir: &Path) -> CredentialFile {
    super::LoginDirStore::new(dir).file(AGENT_RECORD)
}

/// The record in `dir`, from the credential store, off the async workers.
async fn load(dir: &Path) -> io::Result<Option<Record>> {
    let file = file(dir);
    let bytes = tokio::task::spawn_blocking(move || file.read())
        .await
        .map_err(io::Error::other)??;
    bytes
        .map(|bytes| serde_json::from_slice(&bytes).map_err(io::Error::other))
        .transpose()
}

/// Writes `record`, unless the login was revoked or deleted meanwhile: a
/// refresh that finishes after a sign-out must not bring it back.
async fn save(dir: &Path, record: &Record) -> io::Result<()> {
    let file = file(dir);
    let dir = dir.to_path_buf();
    let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
    tokio::task::spawn_blocking(move || {
        let lock = login_lock(&dir);
        let _writing = lock.lock().unwrap_or_else(PoisonError::into_inner);
        live(&dir).map_err(io::Error::other)?;
        file.write(&bytes)
    })
    .await
    .map_err(io::Error::other)?
}

/// Whether the login in `dir` is an agent sign-in. The reference file is
/// enough to tell: reading it would reach into the keychain.
pub(crate) fn is_agent(dir: &Path) -> bool {
    dir.join(AGENT_RECORD).symlink_metadata().is_ok()
}

/// One agent sign-in's live token, shared by every caller of the login:
/// turns, discovery, image generation and the failure probe. It refreshes
/// once for all of them, under the record's lock.
#[derive(Clone)]
pub(crate) struct Authenticator {
    dir: PathBuf,
    endpoints: Endpoints,
    held: Arc<Mutex<Option<Record>>>,
}

impl Authenticator {
    pub(crate) fn new(dir: &Path) -> Self {
        Self::with(dir, Endpoints::for_login(dir))
    }

    fn with(dir: &Path, endpoints: Endpoints) -> Self {
        Self {
            dir: dir.to_path_buf(),
            endpoints,
            held: Arc::default(),
        }
    }

    fn held(&self) -> Option<Record> {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .filter(Record::fresh)
    }

    fn hold(&self, record: Option<Record>) {
        *self.held.lock().unwrap_or_else(PoisonError::into_inner) = record;
    }

    /// Drops the token held in memory, so the next request reads the store.
    pub(crate) fn forget(&self) {
        self.hold(None);
    }

    /// The token to send now, refreshed first when it has expired. The
    /// refresh runs on its own task holding the record's lock: if the
    /// request that asked is dropped part-way, the rotated tokens are still
    /// saved, and the next request does not spend the old refresh token.
    #[expect(
        clippy::result_large_err,
        reason = "CredentialSource returns Rig authentication errors unchanged"
    )]
    pub(crate) async fn auth_context(&self) -> Result<AuthContext, AuthError> {
        if let Some(record) = self.held() {
            return Ok(record.context());
        }
        let lock = record_lock(&self.dir.join(AGENT_RECORD)).lock_owned().await;
        let record = load(&self.dir)
            .await
            .map_err(|error| AuthError::Store(error.into()))?
            .ok_or_else(|| {
                AuthError::Message(
                    "ChatGPT sign-in required. Sign in again under Settings → Providers.".into(),
                )
            })?;
        if record.fresh() {
            self.hold(Some(record.clone()));
            return Ok(record.context());
        }
        let dir = self.dir.clone();
        let endpoints = self.endpoints.clone();
        let refreshing = tokio::spawn(async move {
            let _locked = lock;
            let refreshed = refresh(&endpoints, &record).await?;
            save(&dir, &refreshed)
                .await
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(refreshed)
        });
        let refreshed = refreshing
            .await
            .map_err(|error| AuthError::Message(error.to_string()))?
            .map_err(AuthError::Message)?;
        self.hold(Some(refreshed.clone()));
        Ok(refreshed.context())
    }
}

impl rig::client::signed_in::CredentialSource for Authenticator {
    async fn auth_context(
        &self,
        _http: &rig::http_client::DynHttpClient,
    ) -> Result<AuthContext, AuthError> {
        Authenticator::auth_context(self).await
    }
}

/// A signed-in agent login in `dir` holding `access`, good for an hour,
/// for the tests of what the login's requests carry.
#[cfg(test)]
pub(crate) async fn seed(dir: &Path, access: &str) {
    let record = Record {
        access_token: access.into(),
        refresh_token: "agent-refresh".into(),
        expires_at: chrono::Utc::now().timestamp() + 3600,
        client_id: "oaiapp_seeded".into(),
        host_id: "urn:uuid:e61bbe28-07ef-466d-8e5d-a344f94ab305".into(),
        scopes: SCOPE.split(' ').map(str::to_string).collect(),
    };
    save(dir, &record).await.unwrap();
}

#[cfg(test)]
mod tests;
