//! Classify at the provider boundary, before diagnostic data becomes display text.
use crate::accounts::resolve::AccountRefusal;
use crate::models::Client;
use crate::providers::discovery::ListedModel;
use crate::session::ProviderAuth;
pub(crate) use hotline_room::secret::redact_text;
use rig::ProviderError;
use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    UsageLimit,
    AccountUnavailable,
    Unknown,
}

const USAGE_LIMIT_BYTES: usize = 500;

/// How Rig names a failure of the store a sign-in's tokens are kept in
/// (`AuthError::Store`): for ChatGPT and Copilot, the OS keychain.
const KEYCHAIN_FAILED: &str = "token store failed";

/// How the vault's own store says the OS keychain is locked, when a key's
/// secret is read for a turn (`hotline_support::credentials`), in lower case.
const KEYCHAIN_LOCKED: &str = "os credential storage is locked";

/// What a stream the provider never finished reads as.
pub(crate) const TRUNCATED: &str = "The model stream ended without a complete response.";

/// What a message to Hotline Agent is refused with while no provider is
/// connected. The words go back to the composer; nothing reaches the tape.
pub(crate) const NO_PROVIDER: &str =
    "Hotline Agent unavailable: connect a provider in Settings › Providers.";

/// What a turn reads as when the account its teammate is set to cannot sign
/// it. No other credential of the provider is ever used in its place.
pub(crate) const ACCOUNT_SIGNED_OUT: &str =
    "The account this teammate is set to is signed out or gone. Choose another account.";

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
    /// Skipped when absent, so a System default teammate's notice is the
    /// bytes it was before accounts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
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
        let kind = if contains(&[KEYCHAIN_FAILED, KEYCHAIN_LOCKED]) {
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
            || body == ACCOUNT_SIGNED_OUT
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
        // A harness too old for the model it was asked to run says so in a
        // 400 that would otherwise read as the model's settings or a bad
        // request; it is the harness that has to be updated. A harness that
        // lost its sign-in says so in its own words, and ACP's
        // `auth_required` is code -32000. Either way it is the harness's
        // login that has to run again, not a Hotline credential.
        let kind = if phase.starts_with("acp")
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
            ]) {
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
        let retry_after_seconds = value.as_ref().and_then(|value| retry_hint(value, 0));
        let mut failure = Self::of(kind, sanitize(body), phase);
        failure.status = status;
        failure.code = code.map(|code| sanitize(&code));
        failure.retry_after_seconds = retry_after_seconds;
        failure
    }

    /// `message` is the harness's error and `stderr_hint` the stderr tail
    /// behind it. A usage limit is decided before [`Failure::classify`], whose
    /// sign-in rewrite must never apply to it, and its summary comes from
    /// `message` alone.
    pub fn classify_acp(
        message: &str,
        stderr_hint: &str,
        phase: &'static str,
        usage_limit_prefixes: &[&str],
    ) -> Self {
        let body = format!("{message}{stderr_hint}");
        let limit = phase
            .starts_with("acp")
            .then(|| usage_limit_text(message, usage_limit_prefixes))
            .flatten();
        let Some(text) = limit else {
            return Self::classify(&body, None, phase);
        };
        let mut failure = Self::of(Kind::UsageLimit, sanitize(&body), phase);
        failure.summary = Cow::Owned(text);
        failure
    }

    /// The account goes by its id alone: its name and folder may only be in
    /// a private `sentence`.
    pub fn account_unavailable(
        refusal: &AccountRefusal,
        agent: &str,
        sentence: String,
        phase: &'static str,
    ) -> Self {
        let (account_id, code) = match refusal {
            AccountRefusal::UnknownAccount { account_id } => (account_id, "unknown_account"),
            AccountRefusal::FolderUnavailable { account_id, .. } => {
                (account_id, "folder_unavailable")
            }
        };
        let mut failure = Self::of(Kind::AccountUnavailable, sentence.clone(), phase);
        failure.title = format!("{agent} account unavailable").into();
        failure.summary = Cow::Owned(sentence);
        failure.code = Some(code.to_string());
        failure.account_id = Some(account_id.clone());
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
            account_id: None,
        };
        failure.set_kind(kind);
        failure
    }

    /// A 401 or 403 on a turn says the provider refused, not why. When the
    /// turn was signed by a sign-in with a model list, the list is asked once
    /// with that same sign-in: if it still answers, the sign-in is fine and
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
                self.refused_if_listed(token_dir, "ChatGPT is refusing requests", listing)
                    .await
            }
            (Client::Copilot, ProviderAuth::Login { token_dir }) => {
                let listing = crate::providers::copilot::list_models(token_dir);
                self.refused_if_listed(token_dir, "GitHub Copilot is refusing requests", listing)
                    .await
            }
            (Client::XAi, ProviderAuth::StoredLogin { tokens }) => {
                let listing = crate::providers::xai::list_models(tokens);
                self.refused_if_listed(tokens.path(), "xAI is refusing requests", listing)
                    .await
            }
            _ => self,
        }
    }

    /// The probe itself. `listing` is only awaited when no verdict on this
    /// sign-in is fresher than [`VERDICT_FRESH`]; one that fails or does not
    /// answer in time leaves the sign-in wording. A verdict is kept by where
    /// the sign-in lives, since a provider may have several.
    async fn refused_if_listed(
        mut self,
        sign_in: &Path,
        title: &'static str,
        listing: impl Future<Output = Result<Vec<ListedModel>, String>>,
    ) -> Self {
        static VERDICTS: Mutex<Option<HashMap<PathBuf, (Instant, bool)>>> = Mutex::new(None);
        let fresh = {
            let verdicts = VERDICTS.lock().unwrap_or_else(PoisonError::into_inner);
            verdicts
                .as_ref()
                .and_then(|verdicts| verdicts.get(sign_in))
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
                .insert(sign_in.to_path_buf(), (Instant::now(), listed));
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
            Kind::UsageLimit => (
                "Agent usage limit reached",
                "The agent stopped at its usage limit.",
            ),
            Kind::AccountUnavailable => (
                "Agent account unavailable",
                "This teammate's agent account needs attention at the desk.",
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

/// The agent's usage-limit sentence in `message`: from its earliest prefix to
/// the end of that line, or to the quote that ends it when it arrived as a
/// JSON string, without the `: {` an error's data object opens with.
fn usage_limit_text(message: &str, prefixes: &[&str]) -> Option<String> {
    let start = prefixes
        .iter()
        .filter_map(|prefix| message.find(prefix))
        .min()?;
    let rest = &message[start..];
    let line = &rest[..rest.find(['\n', '"']).unwrap_or(rest.len())];
    let line = line.trim_end();
    let mut text = redact_text(line.strip_suffix(": {").unwrap_or(line).trim());
    text.truncate(text.floor_char_boundary(USAGE_LIMIT_BYTES));
    Some(text)
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
mod tests {
    use super::*;

    #[test]
    fn quota_auth_and_bad_settings_are_not_retried_or_mistaken_for_replay() {
        for (body, status, kind) in [
            (
                r#"{"error":{"code":"insufficient_quota"}}"#,
                429,
                Kind::Quota,
            ),
            (
                r#"{"error":{"message":"invalid effort setting"}}"#,
                400,
                Kind::Configuration,
            ),
            (r#"{"error":{"code":"invalid_api_key"}}"#, 401, Kind::Auth),
        ] {
            let failure = Failure::classify(body, Some(status), "request");
            assert_eq!(failure.kind, kind);
            assert!(failure.retry_delay(0).is_none());
        }
    }

    /// Claude Code's expired OAuth, as it reached a phone, and ACP's own
    /// `auth_required`, are the harness's sign-in; a provider's 401 outside
    /// an ACP session stays a Hotline credential.
    #[test]
    fn a_harness_that_lost_its_sign_in_is_told_apart_from_a_provider_key() {
        let expired = Failure::classify(
            r#"Internal error: Failed to authenticate: OAuth session expired and could not be refreshed: {
  "errorKind": "authentication_failed"
} The backend said: [session/create] sessionId=f1333926 phase=models durationMs=1"#,
            None,
            "acp_prompt",
        );
        assert_eq!(expired.kind, Kind::AgentAuth);
        assert_eq!(expired.title, "Agent sign-in expired");
        assert!(expired.retry_delay(0).is_none());
        let required = Failure::classify(
            r#"The agent would not open a session: {"code":-32000,"message":"Authentication required"}"#,
            None,
            "acp_start",
        );
        assert_eq!(required.kind, Kind::AgentAuth);
        let provider = Failure::classify(
            r#"{"error":{"code":"invalid_api_key"}}"#,
            Some(401),
            "request",
        );
        assert_eq!(provider.kind, Kind::Auth);
        let other = Failure::classify("Internal error: tool crashed", None, "acp_prompt");
        assert_eq!(other.kind, Kind::Acp);
    }

    /// Codex asked for a model newer than itself answers with a 400 that says
    /// so. That is the harness's version, not the model's settings, the
    /// request's shape or a sign-in, and the card says to update it.
    #[test]
    fn a_harness_too_old_for_its_model_says_to_update_it() {
        let codex = r#"Internal error: unexpected status 400 Bad Request: {"detail":"The 'gpt-5.5-codex' model requires a newer version of Codex. Please upgrade to the latest app or CLI and try again."}"#;
        let mut outdated = Failure::classify(codex, None, "acp_prompt");
        assert_eq!(outdated.kind, Kind::HarnessOutdated);
        assert!(outdated.summary.contains("Update"), "{}", outdated.summary);
        assert!(outdated.retry_delay(0).is_none());
        outdated.name_the_harness("Codex", Some("1.12.0"));
        assert_eq!(outdated.title, "Codex 1.12.0 is too old for this model");
        assert!(outdated.notice().contains(r#""kind":"harness_outdated""#));

        let with_status = Failure::classify(
            r#"{"error":{"message":"model not supported by this harness version"}}"#,
            Some(400),
            "acp_prompt",
        );
        assert_eq!(with_status.kind, Kind::HarnessOutdated);

        // Outside an ACP harness the same words are the provider's, and an
        // ordinary unsupported model is still the model's settings.
        assert_eq!(
            Failure::classify(codex, Some(400), "request").kind,
            Kind::InvalidRequest
        );
        assert_eq!(
            Failure::classify("unsupported model: nonesuch", None, "acp_prompt").kind,
            Kind::Configuration
        );
        // And a name is only put on that card.
        let mut other = Failure::classify("Internal error: tool crashed", None, "acp_prompt");
        other.name_the_harness("Codex", Some("1.12.0"));
        assert_eq!(other.title, "Agent connection failed");
    }

    /// OpenAI's 25 September 2026 outage: a ChatGPT sign-in that still listed
    /// its models had every turn refused with `invalid_api_key`. A stub that
    /// lists models and refuses `/responses` reads as the provider refusing;
    /// a token the stub refuses everywhere still reads as a sign-in to fix.
    #[tokio::test]
    async fn a_provider_refusing_turns_is_told_apart_from_a_sign_in_that_no_longer_works() {
        use crate::providers::rig_login;
        use axum::{Router, http::StatusCode, routing::get, routing::post};

        let refused = || {
            (
                StatusCode::UNAUTHORIZED,
                [("content-type", "application/json")],
                serde_json::json!({"error": {"code": "invalid_api_key",
                    "message": "Incorrect API key provided."}})
                .to_string(),
            )
        };
        let listed = || {
            (
                StatusCode::OK,
                [("content-type", "application/json")],
                serde_json::json!({"models": [{"slug": "gpt-6-luna", "visibility": "list"}]})
                    .to_string(),
            )
        };
        let app = Router::new()
            .route("/working/models", get(move || async move { listed() }))
            .route("/working/responses", post(move || async move { refused() }))
            .route("/revoked/models", get(move || async move { refused() }))
            .route("/revoked/responses", post(move || async move { refused() }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let mut verdicts = Vec::new();
        for sign_in in ["working", "revoked"] {
            let dir = tempfile::tempdir().unwrap();
            let far = chrono::Utc::now().timestamp() + 3600;
            std::fs::write(
                dir.path().join("auth.json"),
                serde_json::json!({"access_token": "chatgpt-access", "refresh_token": "r",
                    "expires_at": far, "account_id": "acct-1"})
                .to_string(),
            )
            .unwrap();
            let signed = crate::providers::outgoing::Outgoing::signed(
                rig_login::chatgpt(dir.path()),
                std::convert::identity,
            )
            .unwrap();
            let model = rig::providers::openai::OpenAIConfig::with_key(
                &rig::providers::chatgpt::DIALECT,
                "",
            )
            .with_auth(rig::providers::openai::wire::Auth::OptionalBearer)
            .with_base_url(format!("{base}/{sign_in}"))
            .connect(signed)
            .completion("gpt-6-luna");
            let error = model.call("hello").await.unwrap_err();
            let failure = Failure::provider(error, "request");
            assert_eq!(failure.kind, Kind::Auth);
            let models_url = format!("{base}/{sign_in}/models");
            let listing =
                crate::providers::chatgpt::list_models_at(dir.path(), &models_url, "0.161.2");
            let failure = failure
                .refused_if_listed(dir.path(), "ChatGPT is refusing requests", listing)
                .await;
            verdicts.push(failure);
        }
        server.abort();

        let [working, revoked] = verdicts.try_into().unwrap();
        assert_eq!(working.kind, Kind::ProviderRefused);
        assert_eq!(working.title, "ChatGPT is refusing requests");
        assert!(working.summary.contains("signing in again won't help"));
        assert!(!working.notice().contains("Settings"));
        assert!(working.details.contains("invalid_api_key"));
        assert!(working.retry_delay(0).is_none());
        assert_eq!(revoked.kind, Kind::Auth);
        assert_eq!(revoked.title, "Provider sign-in required");
    }

    /// A run of refused turns asks the model list once: the second failure
    /// within the verdict's life takes the first one's answer.
    #[tokio::test]
    async fn a_run_of_refused_turns_probes_the_sign_in_once() {
        let asked = std::sync::atomic::AtomicUsize::new(0);
        let probe = || async {
            asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Vec::new())
        };
        let refused = || {
            Failure::classify(
                r#"{"error":{"code":"invalid_api_key"}}"#,
                Some(401),
                "request",
            )
        };
        let sign_in = std::env::temp_dir().join(format!("probe-once-{}", uuid::Uuid::new_v4()));
        let first = refused()
            .refused_if_listed(&sign_in, "X is refusing requests", probe())
            .await;
        let second = refused()
            .refused_if_listed(&sign_in, "X is refusing requests", probe())
            .await;
        assert_eq!(first.kind, Kind::ProviderRefused);
        assert_eq!(second.kind, Kind::ProviderRefused);
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// Two Copilot sign-ins on one stub, of which only the first still lists
    /// its models: the second is probed itself, not answered by the first.
    #[tokio::test]
    async fn two_sign_ins_of_one_provider_do_not_share_a_verdict() {
        use crate::providers::copilot::tests::seeded_login;
        use axum::{Router, routing::get};
        let app = Router::new().route(
            "/listed/models",
            get(|| async { r#"{"data":[{"id":"gpt-5-mini"}]}"# }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut kinds = Vec::new();
        for sign_in in ["listed", "gone"] {
            let token_dir =
                seeded_login(&format!("verdict-{sign_in}"), &format!("{base}/{sign_in}"));
            let keys = HashMap::from([(
                "github-copilot".to_string(),
                ProviderAuth::Login { token_dir },
            )]);
            let refused = Failure::classify(
                r#"{"error":{"code":"invalid_api_key"}}"#,
                Some(401),
                "request",
            );
            kinds.push(
                refused
                    .checked_against_sign_in("github-copilot/gpt-5-mini", &keys)
                    .await
                    .kind,
            );
        }
        server.abort();
        assert_eq!(kinds, [Kind::ProviderRefused, Kind::Auth]);
    }

    /// A provider with no model list to ask, or a failure that is not a
    /// refused sign-in, keeps the wording it was classified with.
    #[tokio::test]
    async fn only_a_refused_sign_in_with_a_model_list_is_probed() {
        let keys = HashMap::from([("openai".to_string(), ProviderAuth::ApiKey("sk-test".into()))]);
        let refused = Failure::classify(
            r#"{"error":{"code":"invalid_api_key"}}"#,
            Some(401),
            "request",
        );
        let kept = refused
            .checked_against_sign_in("openai/gpt-6-sol", &keys)
            .await;
        assert_eq!(kept.kind, Kind::Auth);
        let busy = Failure::classify("busy", Some(503), "request");
        let kept = busy
            .checked_against_sign_in("openai/gpt-6-sol", &keys)
            .await;
        assert_eq!(kept.kind, Kind::Transport);
    }

    #[test]
    fn retry_hints_survive_and_retry_budget_is_bounded() {
        let mut headers = http::HeaderMap::new();
        headers.insert("retry-after", "60".parse().unwrap());
        let error = ProviderError::ProviderResponse(
            rig::ProviderResponseError::new(
                http::StatusCode::TOO_MANY_REQUESTS,
                r#"{"error":{"code":"rate_limit_exceeded"}}"#,
            )
            .with_headers(Some(headers)),
        );
        let failure = Failure::provider(error, "request");
        assert_eq!(failure.kind, Kind::RateLimit);
        assert_eq!(failure.retry_after_seconds, Some(60));
        assert!(failure.retry_delay(0).is_none());
        let failure = Failure::classify("busy", Some(503), "stream");
        assert!(failure.retry_delay(0).is_some());
        assert!(failure.retry_delay(3).is_none());
    }

    #[test]
    fn a_refused_responses_item_is_repaired_but_an_arbitrary_bad_setting_is_not() {
        let replay = Failure::classify(
            r#"{"error":{"code":"invalid_value","param":"input[2].id","message":"Expected an ID without colons"}}"#,
            Some(400),
            "request",
        );
        assert_eq!(replay.kind, Kind::Replay);
        let setting = Failure::classify(
            r#"{"error":{"code":"invalid_value","param":"temperature","message":"Invalid value"}}"#,
            Some(400),
            "request",
        );
        assert_eq!(setting.kind, Kind::InvalidRequest);
    }

    const LIMITED: &str = "Internal error: You've hit your limit · resets 3pm (Europe/Lisbon): {\n  \"errorKind\": \"rate_limit\"\n}";

    #[test]
    fn an_agents_usage_limit_is_its_own_kind_in_its_own_words() {
        let prefixes = crate::accounts::support::CLAUDE.usage_limit_prefixes;
        let limited = Failure::classify_acp(LIMITED, "", "acp_prompt", prefixes);
        assert_eq!(
            (limited.kind, limited.summary.as_ref()),
            (
                Kind::UsageLimit,
                "You've hit your limit · resets 3pm (Europe/Lisbon)"
            )
        );
        assert!(limited.notice().contains(r#""kind":"usage_limit""#));
        assert_eq!(limited.retry_delay(0), None);
        assert_ne!(
            Failure::classify_acp(LIMITED, "", "acp_prompt", &[]).kind,
            Kind::UsageLimit
        );
        assert_ne!(
            Failure::classify_acp(LIMITED, "", "request", prefixes).kind,
            Kind::UsageLimit
        );
        assert_eq!(
            Failure::classify_acp(
                r#"{"code":-32000,"message":"Authentication required"}"#,
                "",
                "acp_prompt",
                prefixes
            )
            .kind,
            Kind::AgentAuth
        );
        let coded = Failure::classify_acp(
            r#"{"code":-32000,"message":"You've hit your limit · resets 3pm"}"#,
            "",
            "acp_prompt",
            prefixes,
        );
        assert_eq!(
            (coded.kind, coded.summary.as_ref()),
            (Kind::UsageLimit, "You've hit your limit · resets 3pm")
        );
        let hinted = Failure::classify_acp(
            "Internal error: You've hit your limit · resets 3pm",
            " The backend said: retrying in 5s",
            "acp_prompt",
            prefixes,
        );
        assert_eq!(hinted.summary, "You've hit your limit · resets 3pm");
        assert!(
            hinted.details.contains("retrying in 5s"),
            "{}",
            hinted.details
        );
    }

    #[test]
    fn an_account_refusal_card_names_the_account_by_id_alone() {
        let refusal = AccountRefusal::FolderUnavailable {
            account_id: "a1".into(),
            name: "Account 1".into(),
            path: "C:/d/agent-accounts/a1".into(),
            kind: std::io::ErrorKind::PermissionDenied,
        };
        let notice = Failure::account_unavailable(
            &refusal,
            "Claude Code",
            refusal.public_sentence("Ada", "Claude Code"),
            "acp_start",
        )
        .notice();
        for part in [
            r#""kind":"account_unavailable""#,
            r#""accountId":"a1""#,
            r#""code":"folder_unavailable""#,
            "Ada can't start: its Claude Code account needs attention at the desk.",
        ] {
            assert!(notice.contains(part), "{notice}");
        }
        assert!(!notice.contains("Account 1") && !notice.contains("agent-accounts"));
        assert!(
            !Failure::classify("x", None, "local")
                .notice()
                .contains("accountId")
        );
    }

    #[test]
    fn nested_vendor_json_and_bearer_tokens_are_redacted_before_the_tape() {
        let body = serde_json::json!({"error":{"metadata":{"raw":r#"{"api_key":"private","message":"Bearer secretvalue"}"#}, "code":"rate_limit_exceeded"}}).to_string();
        let notice = Failure::classify(&body, Some(429), "stream").notice();
        assert!(!notice.contains("private"));
        assert!(!notice.contains("secretvalue"));
        assert!(notice.contains("rate_limit_exceeded"));
        assert!(notice.contains("[redacted]"));
    }
}
