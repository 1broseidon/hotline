//! Classify at the provider boundary, before diagnostic data becomes display text.
use crate::models::Client;
use crate::providers::discovery::ListedModel;
use crate::session::ProviderAuth;
pub(crate) use hotline_room::secret::redact_text;
use rig::ProviderError;
use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How long a probe's verdict on a sign-in stands, so a run of refused turns
/// asks the provider's model list once rather than once a turn.
const VERDICT_FRESH: Duration = Duration::from_secs(60);

/// How long the probe may take before the notice goes out with the sign-in
/// wording it would have had without one.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    RateLimit,
    Quota,
    Auth,
    /// The provider refused a turn with a 401 or 403, but the same sign-in
    /// still lists its models. The sign-in is fine, so signing in again
    /// cannot help; the provider is turning requests away.
    ProviderRefused,
    /// The sign-in's tokens are in the OS keychain, and the keychain would
    /// not give them up: it is locked, or Hotline was not allowed in. The
    /// sign-in itself is fine, so signing in again cannot help; unlocking
    /// the keychain does.
    Keychain,
    /// An ACP harness's own sign-in — Claude Code's, Codex's — is gone or
    /// expired. Nothing in Hotline's Settings fixes it; the harness's login
    /// does.
    AgentAuth,
    /// An ACP harness answered that the model needs a newer version of it:
    /// Codex's "requires a newer version of Codex", sent as a 400. The model
    /// and the sign-in are fine; the harness on this computer is behind.
    HarnessOutdated,
    Context,
    Replay,
    Configuration,
    Modality,
    InvalidRequest,
    Acp,
    Transport,
    Provider,
    /// The Chat budget has no room for a paid request. Sending again does
    /// not help until the budget is raised or a new day or month begins.
    Budget,
    /// An ACP harness's command never finished starting: it did not answer
    /// `initialize` and open a session within the start bound. Sending
    /// another message starts it again.
    Startup,
    /// No provider is connected on this desk, so Hotline Agent has no model
    /// to run. Said when the teammate comes up, before anything is sent; a
    /// provider in Settings › Providers fixes it.
    NoProvider,
    /// The account the teammate's model names is revoked, deleted or has
    /// lost its secret. The teammate does not run on another account of the
    /// provider until the person picks one.
    AccountGone,
    /// The account behind the turn has used what its plan allows until it
    /// resets: a ChatGPT sign-in's `usage_limit_reached`, or Claude Code's or
    /// Codex's own "You've hit your limit". Sending again on the same account
    /// fails the same way; another account may not.
    UsageLimit,
    Unknown,
}

/// How Rig names a failure of the store a sign-in's tokens are kept in
/// (`AuthError::Store`): for ChatGPT and Copilot, the OS keychain.
const KEYCHAIN_FAILED: &str = "token store failed";

/// How an agent's out-of-usage sentence begins, so a failed turn that says
/// one can be told from any other failure: `USAGE_LIMIT_ERROR_PREFIXES` of
/// `@anthropic-ai/claude-agent-sdk` 0.3.270, the copy bundled by
/// `@agentclientprotocol/claude-agent-acp` 0.78.0, in the SDK's order. Codex's
/// `UsageLimitReachedError` begins with the first of them too.
const USAGE_LIMIT_PREFIXES: &[&str] = &[
    "You've hit your",
    "You've reached your",
    "You're out of usage credits",
    "Your org is out of usage \u{b7} add funds to continue",
    "Your org is out of usage \u{b7} contact your admin",
    "Your seat type doesn't include usage credits",
    "Your seat type doesn't include usage",
    "Your usage allocation has been disabled by your admin",
    "Your group's usage limit is set to $0",
    "Fable 5 requires usage credits",
    "You're out of extra usage",
    "Your seat type doesn't include extra usage",
];

/// The most of an agent's usage-limit sentence the card repeats.
const USAGE_LIMIT_BYTES: usize = 500;

/// What a stream the provider never finished reads as.
pub(crate) const TRUNCATED: &str = "The model stream ended without a complete response.";

/// What a message to Hotline Agent is refused with while no provider is
/// connected. The words go back to the composer; nothing reaches the tape.
pub(crate) const NO_PROVIDER: &str =
    "Hotline Agent unavailable: connect a provider in Settings › Providers.";

/// The whole structured diagnosis of a failed turn. It is large, and the rig
/// driver returns it by value anyway: a turn returns it at most once, so
/// boxing it buys nothing, and each `result_large_err` there says so.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Failure {
    pub kind: Kind,
    /// Named for the harness when the harness is the problem, so it is not
    /// always a literal.
    pub title: Cow<'static, str>,
    /// The agent's own words for a usage limit, so not always a literal.
    pub summary: Cow<'static, str>,
    pub details: String,
    pub phase: &'static str,
    pub status: Option<u16>,
    pub code: Option<String>,
    pub retry_after_seconds: Option<u64>,
    pub tools_may_have_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sign_in: Option<super::auth::SignIn>,
    /// The model a teammate on a vanished or spent account was on, so the
    /// card can offer the other accounts that serve it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The agent a harness teammate was on when its account went or ran
    /// out (`claude-acp`, `claude-acp#<account>`), so the card can offer the
    /// agent's other accounts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// When a usage limit lifts, in Unix milliseconds, when the provider
    /// said. The card shows it as a time of day, which a count of seconds
    /// written on the tape could not stay true to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

impl Failure {
    pub fn provider(error: ProviderError, phase: &'static str) -> Self {
        // A stream cut before the provider ended it is a transport failure
        // worth another try, whatever cut it.
        if matches!(error, ProviderError::Truncated) {
            return Self::classify(TRUNCATED, Some(502), phase);
        }
        let status = error
            .provider_response_status()
            .map(|status| status.as_u16());
        let body = error
            .provider_response_body()
            .map_or_else(|| error.to_string(), str::to_owned);
        let mut failure = Self::classify(&body, status, phase);
        // A sign-in's tokens are read as the request is signed, so a keychain
        // that refuses them fails the request before it leaves: no reply, but
        // nothing another try past a locked keychain gets through either.
        if status.is_none()
            && matches!(error, ProviderError::Http(_))
            && failure.kind != Kind::Keychain
        {
            failure.set_kind(Kind::Transport);
        }
        failure.retry_after_seconds = error
            .provider_response_headers()
            .and_then(|headers| headers.get("retry-after"))
            .and_then(|value| value.to_str().ok())
            .and_then(|value| {
                value.parse().ok().or_else(|| {
                    chrono::DateTime::parse_from_rfc2822(value)
                        .ok()
                        .map(|date| {
                            (date.timestamp() - chrono::Utc::now().timestamp()).max(0) as u64
                        })
                })
            })
            .or(failure.retry_after_seconds);
        failure
    }

    pub fn classify(body: &str, status: Option<u16>, phase: &'static str) -> Self {
        let value = serde_json::from_str::<Value>(body).ok();
        let code = value
            .as_ref()
            .and_then(|v| {
                v.pointer("/error/code")
                    .or_else(|| v.pointer("/error/type"))
                    .or_else(|| v.get("code"))
            })
            .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned));
        let lower = body.to_lowercase();
        let contains = |words: &[&str]| words.iter().any(|word| lower.contains(word));
        let refused_replay_field = status == Some(400)
            && value
                .as_ref()
                .and_then(|value| value.pointer("/error/param"))
                .and_then(Value::as_str)
                .is_some_and(|param| {
                    param.starts_with("input[")
                        && param
                            .rsplit_once('.')
                            .is_some_and(|(_, field)| field == "id" || field == "encrypted_content")
                });
        // Body shape from openai/codex, codex-rs/codex-api/src/api_bridge.rs, UsageErrorBody.
        let chatgpt_usage_limit = status == Some(429)
            && value
                .as_ref()
                .and_then(|value| value.pointer("/error/type"))
                .and_then(Value::as_str)
                == Some("usage_limit_reached");
        let kind = if contains(&[KEYCHAIN_FAILED]) {
            Kind::Keychain
        } else if contains(&[
            "insufficient_quota",
            "quota_exceeded",
            "exhausted quota",
            "credit balance",
            "spending limit",
            "payment required",
        ]) || status == Some(402)
        {
            Kind::Quota
        } else if matches!(status, Some(401 | 403))
            || contains(&[
                "invalid_api_key",
                "authentication_error",
                "token expired",
                "token revoked",
            ])
        {
            Kind::Auth
        } else if contains(&[
            "context_length_exceeded",
            "context window",
            "maximum context length",
            "prompt is too long",
        ]) {
            Kind::Context
        } else if refused_replay_field
            || contains(&[
                "invalid encrypted",
                "invalid signature",
                "thinking signature",
                "reasoning item",
                "item with id",
            ])
        {
            Kind::Replay
        } else if chatgpt_usage_limit {
            Kind::UsageLimit
        } else if status == Some(429)
            || contains(&["rate_limit", "rate limit", "too many requests"])
        {
            Kind::RateLimit
        } else if matches!(status, Some(408 | 502 | 503 | 504)) {
            Kind::Transport
        } else if status.is_some_and(|s| s >= 500) {
            Kind::Provider
        } else if contains(&[
            "does not support image",
            "does not support images",
            "unsupported image",
            "unsupported modality",
            "image input is not supported",
        ]) {
            Kind::Modality
        } else if contains(&[
            "invalid effort",
            "unsupported parameter",
            "unsupported model",
            "model_not_found",
            "unknown model",
        ]) {
            Kind::Configuration
        } else if status.is_some_and(|s| (400..500).contains(&s)) {
            Kind::InvalidRequest
        } else if phase == "acp_prompt" {
            Kind::Acp
        } else {
            Kind::Unknown
        };
        // A harness out of usage says so in its own sentence, which often
        // carries a "rate_limit" or ACP's -32000 beside it; the sentence is
        // what the card repeats, so it is read before either. A harness too
        // old for the model it was asked to run says so in a 400 that would
        // otherwise read as the model's settings or a bad request; it is the
        // harness that has to be updated. A harness that lost its sign-in
        // says so in its own words, and ACP's `auth_required` is code -32000.
        // Either way it is the harness's login that has to run again, not a
        // Hotline credential.
        let agent_usage_limit = phase
            .starts_with("acp")
            .then(|| usage_limit_text(body))
            .flatten();
        let kind = if agent_usage_limit.is_some() {
            Kind::UsageLimit
        } else if phase.starts_with("acp")
            && matches!(
                kind,
                Kind::Configuration | Kind::InvalidRequest | Kind::Acp | Kind::Unknown
            )
            && contains(&[
                "requires a newer version",
                "not supported by this version",
                "not supported in this version",
                "not supported by this harness version",
                "upgrade to the latest",
            ])
        {
            Kind::HarnessOutdated
        } else if phase.starts_with("acp")
            && (kind == Kind::Auth
                || contains(&[
                    "auth_required",
                    "authentication required",
                    "authentication_failed",
                    "failed to authenticate",
                    "oauth session expired",
                    "not logged in",
                    "please run /login",
                    "\"code\":-32000",
                ]))
        {
            Kind::AgentAuth
        } else {
            kind
        };
        let mut failure = Self::of(kind, sanitize(body), phase);
        failure.status = status;
        failure.code = code.map(|code| sanitize(&code));
        failure.retry_after_seconds = value.as_ref().and_then(|value| retry_hint(value, 0));
        if let Some(text) = agent_usage_limit {
            failure.summary = Cow::Owned(text);
        } else if kind == Kind::UsageLimit {
            failure.title = Cow::Borrowed("ChatGPT usage limit reached");
            failure.summary = Cow::Borrowed("This ChatGPT sign-in reached its usage limit.");
            failure.resets_at = value.as_ref().and_then(resets_at);
        }
        failure
    }

    /// A paid request the Chat budget refused, before it was sent.
    pub fn budget(refused: crate::voice::ledger::Exhausted) -> Self {
        Self::of(Kind::Budget, refused.to_string(), "budget")
    }

    /// An agent whose command did not finish starting in time. The details
    /// are the sentence naming the command, which is what the person needs to
    /// try it in a terminal.
    pub fn startup(details: String) -> Self {
        Self::of(Kind::Startup, sanitize(&details), "acp_start")
    }

    /// Hotline Agent on a desk with no provider connected. Not a turn that
    /// failed: there was no turn, and there is no request to show.
    pub fn no_provider() -> Self {
        Self::of(
            Kind::NoProvider,
            "No provider is connected on this desk, so there is no model to run.".into(),
            "start",
        )
    }

    /// A teammate on an account that is no longer there. Not a turn that
    /// failed at the provider: nothing was sent.
    pub fn account_gone(details: String, model: Option<&str>) -> Self {
        let mut failure = Self::of(Kind::AccountGone, details, "start");
        failure.model = model.map(|model| crate::models::base_id(model).into_owned());
        failure
    }

    fn of(kind: Kind, details: String, phase: &'static str) -> Self {
        let mut failure = Self {
            kind,
            title: Cow::Borrowed(""),
            summary: Cow::Borrowed(""),
            details,
            phase,
            status: None,
            code: None,
            retry_after_seconds: None,
            tools_may_have_run: false,
            sign_in: None,
            model: None,
            backend: None,
            resets_at: None,
        };
        failure.set_kind(kind);
        failure
    }

    /// A 401 or 403 on a turn says the provider refused, not why. When the
    /// teammate's provider is a sign-in with a model list, the list is asked
    /// once with the same sign-in: if it still answers, the sign-in is fine and
    /// the notice says the provider is refusing, instead of sending the person
    /// to sign in again. Any other provider keeps the sign-in wording.
    pub async fn checked_against_sign_in(
        self,
        model: &str,
        keys: &HashMap<String, ProviderAuth>,
    ) -> Self {
        if self.kind != Kind::Auth {
            return self;
        }
        let Some((provider, _)) = model.split_once('/') else {
            return self;
        };
        let (Some(wiring), Some(auth)) = (crate::models::wiring(provider), keys.get(provider))
        else {
            return self;
        };
        match (wiring.client, auth) {
            (Client::ChatGpt, ProviderAuth::Login { token_dir }) => {
                let listing = crate::providers::chatgpt::list_models(token_dir);
                self.refused_if_listed(provider, "ChatGPT is refusing requests", listing)
                    .await
            }
            (Client::Copilot, ProviderAuth::Login { token_dir }) => {
                let listing = crate::providers::copilot::list_models(token_dir);
                self.refused_if_listed(provider, "GitHub Copilot is refusing requests", listing)
                    .await
            }
            (Client::XAi, ProviderAuth::StoredLogin { tokens }) => {
                let listing = crate::providers::xai::list_models(tokens);
                self.refused_if_listed(provider, "xAI is refusing requests", listing)
                    .await
            }
            _ => self,
        }
    }

    /// The probe itself. `listing` is only awaited when no verdict on this
    /// provider's sign-in is fresher than [`VERDICT_FRESH`]; one that fails or
    /// does not answer in time leaves the sign-in wording.
    async fn refused_if_listed(
        mut self,
        provider: &str,
        title: &'static str,
        listing: impl Future<Output = Result<Vec<ListedModel>, String>>,
    ) -> Self {
        static VERDICTS: Mutex<Option<HashMap<String, (Instant, bool)>>> = Mutex::new(None);
        let fresh = {
            let verdicts = VERDICTS.lock().unwrap_or_else(PoisonError::into_inner);
            verdicts
                .as_ref()
                .and_then(|verdicts| verdicts.get(provider))
                .filter(|(asked, _)| asked.elapsed() < VERDICT_FRESH)
                .map(|(_, listed)| *listed)
        };
        let listed = if let Some(listed) = fresh {
            listed
        } else {
            let listed = matches!(
                tokio::time::timeout(PROBE_TIMEOUT, listing).await,
                Ok(Ok(_))
            );
            VERDICTS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_or_insert_with(HashMap::new)
                .insert(provider.to_string(), (Instant::now(), listed));
            listed
        };
        if listed {
            self.set_kind(Kind::ProviderRefused);
            self.title = title.into();
        }
        self
    }

    pub fn after_tools(mut self, may_have_run: bool) -> Self {
        self.tools_may_have_run = may_have_run;
        self
    }

    pub fn redact_value(&mut self, value: &str) {
        if !value.is_empty() {
            self.details = self.details.replace(value, "[redacted]");
            self.code = self
                .code
                .take()
                .map(|code| code.replace(value, "[redacted]"));
        }
    }

    /// The card for a harness too old for its model names the harness, and
    /// the version it reported when it started, so the person knows what to
    /// update. A usage limit names whose limit it is.
    pub fn name_the_harness(&mut self, name: &str, version: Option<&str>) {
        if name.trim().is_empty() {
            return;
        }
        match self.kind {
            Kind::HarnessOutdated => {
                self.title = match version.filter(|version| !version.trim().is_empty()) {
                    Some(version) => format!("{name} {version} is too old for this model"),
                    None => format!("{name} is too old for this model"),
                }
                .into();
            }
            Kind::UsageLimit => self.title = format!("{name} usage limit reached").into(),
            _ => {}
        }
    }

    fn set_kind(&mut self, kind: Kind) {
        self.kind = kind;
        let (title, summary) = match kind {
            Kind::RateLimit => (
                "Provider rate limit",
                "The provider is temporarily limiting requests.",
            ),
            Kind::Quota => (
                "Provider quota exhausted",
                "Check your provider balance or usage limit before continuing.",
            ),
            Kind::Auth => (
                "Provider sign-in required",
                "The provider refused this connection. Check its credentials in Settings.",
            ),
            Kind::ProviderRefused => (
                "Provider is refusing requests",
                "Your sign-in still works, so signing in again won't help. Try again later or switch this teammate to another provider.",
            ),
            Kind::Keychain => (
                "Keychain locked",
                "Hotline could not read this sign-in from your keychain. Unlock your keychain, or allow Hotline into it, then send another message.",
            ),
            Kind::AgentAuth => (
                "Agent sign-in expired",
                "This teammate's harness is signed out. Sign in to it again on the computer running Hotline, then send another message.",
            ),
            Kind::HarnessOutdated => (
                "Agent too old for this model",
                "Update this teammate's harness on the computer running Hotline, or switch it to a model its version supports, then send another message.",
            ),
            Kind::AccountGone => (
                "Account no longer connected",
                "This teammate's account was removed or signed out. Pick another account for it; it won't move to one on its own.",
            ),
            Kind::UsageLimit => (
                "Usage limit reached",
                "This account has used what its plan allows until it resets. Move this teammate to another account, or wait.",
            ),
            Kind::Context => (
                "Context limit reached",
                "The conversation is larger than this model can accept.",
            ),
            Kind::Replay => (
                "Conversation replay refused",
                "The provider could not reuse its saved response state.",
            ),
            Kind::InvalidRequest => (
                "Provider rejected the request",
                "Check the selected model, settings, and supported input types.",
            ),
            Kind::Configuration => (
                "Model settings refused",
                "Check the selected model and its settings before continuing.",
            ),
            Kind::Modality => (
                "Input type unsupported",
                "This model cannot accept one of the supplied input types.",
            ),
            Kind::Acp => (
                "Agent connection failed",
                "The agent could not finish. Its failed session will be replaced when you send another message.",
            ),
            Kind::Transport => (
                "Provider connection interrupted",
                "The request could not complete over the network.",
            ),
            Kind::Provider => (
                "Provider unavailable",
                "The provider encountered an error while answering.",
            ),
            Kind::Budget => (
                "Chat budget spent",
                "This teammate's model is paid per token, and the Chat budget has no room for another request. Raise it in Settings › Budgets.",
            ),
            Kind::Startup => (
                "Agent did not start",
                "Its command never answered, so it was stopped. Sending another message starts it again.",
            ),
            Kind::NoProvider => (
                "Hotline Agent unavailable",
                "Connect a provider in Settings › Providers to run a model.",
            ),
            Kind::Unknown => (
                "Turn failed",
                "The activity could not finish. Expand the details for the reported error.",
            ),
        };
        self.title = Cow::Borrowed(title);
        self.summary = Cow::Borrowed(summary);
    }

    pub fn retry_delay(&self, attempt: u32) -> Option<Duration> {
        if attempt >= 3
            || !matches!(
                self.kind,
                Kind::RateLimit | Kind::Transport | Kind::Provider
            )
        {
            return None;
        }
        let seconds = self.retry_after_seconds.unwrap_or(1 << attempt);
        // A long provider cooldown requires another operator message, not an unbounded turn.
        if seconds > 30 {
            return None;
        }
        let jitter = u64::from(uuid::Uuid::new_v4().as_bytes()[0]);
        Some(Duration::from_millis(seconds * 1000 + jitter))
    }

    pub fn notice(&self) -> String {
        // Notice keeps the byte-compatible tape shape; older clients still show useful text.
        format!(
            "{}: {}\n\n{}",
            self.title,
            self.summary,
            serde_json::json!({"hotlineFailure": self})
        )
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::classify(&message, None, "local")
    }
}
impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::classify(message, None, "local")
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.title, self.details)
    }
}

fn retry_hint(value: &Value, depth: usize) -> Option<u64> {
    if depth > 8 {
        return None;
    }
    match value {
        Value::Object(map) => map
            .iter()
            .filter_map(|(key, value)| {
                if matches!(
                    key.to_lowercase().as_str(),
                    "retry-after" | "retry_after_seconds" | "retryafterseconds"
                ) {
                    value
                        .as_u64()
                        .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
                } else {
                    retry_hint(value, depth + 1)
                }
            })
            .max(),
        Value::Array(values) => values
            .iter()
            .filter_map(|value| retry_hint(value, depth + 1))
            .max(),
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| retry_hint(&value, depth + 1)),
        _ => None,
    }
}

/// The agent's usage-limit sentence in `text`: from its earliest prefix to
/// the end of that line, or to the quote that ends it when it arrived as a
/// JSON string, without the `: {` an error's data object opens with.
fn usage_limit_text(text: &str) -> Option<String> {
    let start = USAGE_LIMIT_PREFIXES
        .iter()
        .filter_map(|prefix| text.find(prefix))
        .min()?;
    let rest = &text[start..];
    let line = rest[..rest.find(['\n', '"']).unwrap_or(rest.len())].trim_end();
    let mut sentence = redact_text(line.strip_suffix(": {").unwrap_or(line).trim());
    sentence.truncate(sentence.floor_char_boundary(USAGE_LIMIT_BYTES));
    Some(sentence)
}

/// A ChatGPT usage limit's `resets_at`, Unix seconds, as milliseconds, while
/// it is still ahead.
fn resets_at(value: &Value) -> Option<i64> {
    let seconds = value.pointer("/error/resets_at")?.as_i64()?;
    (seconds > chrono::Utc::now().timestamp()).then(|| seconds.saturating_mul(1000))
}

fn secret_key(key: &str) -> bool {
    let key = key.to_lowercase().replace(['-', '_'], "");
    [
        "authorization",
        "apikey",
        "accesstoken",
        "refreshtoken",
        "password",
        "secret",
        "cookie",
    ]
    .iter()
    .any(|word| key.contains(word))
}

fn clean(value: &mut Value, depth: usize) {
    if depth > 12 {
        *value = Value::String("[nested details omitted]".into());
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if secret_key(key) {
                    *value = Value::String("[redacted]".into());
                } else {
                    clean(value, depth + 1);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                clean(value, depth + 1);
            }
        }
        Value::String(text) => {
            if let Ok(mut nested) = serde_json::from_str::<Value>(text) {
                clean(&mut nested, depth + 1);
                *value = nested;
            } else {
                *text = redact_text(text);
            }
        }
        _ => {}
    }
}

pub(crate) fn sanitize(body: &str) -> String {
    let mut output = if let Ok(mut value) = serde_json::from_str::<Value>(body) {
        clean(&mut value, 0);
        serde_json::to_string_pretty(&value).unwrap_or_default()
    } else {
        redact_text(body)
    };
    if output.len() > 32 * 1024 {
        let mut end = 32 * 1024;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
        output.push_str("\n[details truncated]");
    }
    output
}

#[cfg(test)]
mod tests;
