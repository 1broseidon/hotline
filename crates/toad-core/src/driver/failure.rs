//! Classify at the provider boundary, before diagnostic data becomes display text.
use rig::completion::CompletionError;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    RateLimit,
    Quota,
    Auth,
    Context,
    Replay,
    Configuration,
    Modality,
    InvalidRequest,
    Acp,
    Transport,
    Provider,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Failure {
    pub kind: Kind,
    pub title: &'static str,
    pub summary: &'static str,
    pub details: String,
    pub phase: &'static str,
    pub status: Option<u16>,
    pub code: Option<String>,
    pub retry_after_seconds: Option<u64>,
    pub tools_may_have_run: bool,
}

impl Failure {
    pub fn provider(error: CompletionError, phase: &'static str) -> Self {
        let status = error
            .provider_response_status()
            .map(|status| status.as_u16());
        let body = error
            .provider_response_body()
            .map(str::to_owned)
            .unwrap_or_else(|| error.to_string());
        let mut failure = Self::classify(&body, status, phase);
        if status.is_none() && matches!(error, CompletionError::HttpError(_)) {
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
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            });
        let lower = body.to_lowercase();
        let contains = |words: &[&str]| words.iter().any(|word| lower.contains(word));
        let refused_replay_field = status == Some(400)
            && value
                .as_ref()
                .and_then(|value| value.pointer("/error/param"))
                .and_then(Value::as_str)
                .is_some_and(|param| {
                    param.starts_with("input[")
                        && (param.ends_with(".id") || param.ends_with(".encrypted_content"))
                });
        let kind = if contains(&[
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
        let retry_after_seconds = value.as_ref().and_then(|value| retry_hint(value, 0));
        let mut failure = Self {
            kind,
            title: "",
            summary: "",
            details: sanitize(body),
            phase,
            status,
            code: code.map(|code| sanitize(&code)),
            retry_after_seconds,
            tools_may_have_run: false,
        };
        failure.set_kind(kind);
        failure
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

    fn set_kind(&mut self, kind: Kind) {
        self.kind = kind;
        (self.title, self.summary) = match kind {
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
            Kind::Unknown => (
                "Turn failed",
                "The activity could not finish. Expand the details for the reported error.",
            ),
        };
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
            serde_json::json!({"toadFailure": self})
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

fn redact_text(text: &str) -> String {
    let mut output = text.to_string();
    for prefix in ["Bearer ", "bearer ", "sk-", "sk_", "ghp_", "github_pat_"] {
        let mut offset = 0;
        while let Some(found) = output[offset..].find(prefix) {
            let start = offset + found;
            let end = output[start..]
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '}'))
                .map_or(output.len(), |n| start + n);
            let end = if prefix.ends_with(' ') {
                output[start + prefix.len()..]
                    .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '}'))
                    .map_or(output.len(), |n| start + prefix.len() + n)
            } else {
                end
            };
            output.replace_range(start..end, "[redacted]");
            offset = start + "[redacted]".len();
        }
    }
    output
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

    #[test]
    fn retry_hints_survive_and_retry_budget_is_bounded() {
        let mut headers = http::HeaderMap::new();
        headers.insert("retry-after", "60".parse().unwrap());
        let error = CompletionError::ProviderResponse(
            rig::ProviderResponseError::new(
                http::StatusCode::TOO_MANY_REQUESTS,
                r#"{"error":{"code":"rate_limit_exceeded"}}"#,
            )
            .with_headers(Some(Box::new(headers))),
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
