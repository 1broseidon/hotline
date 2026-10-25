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
        let model =
            rig::providers::openai::OpenAIConfig::with_key(&rig::providers::chatgpt::DIALECT, "")
                .with_auth(rig::providers::openai::wire::Auth::OptionalBearer)
                .with_base_url(format!("{base}/{sign_in}"))
                .connect(signed)
                .completion("gpt-6-luna");
        let error = model.call("hello").await.unwrap_err();
        let failure = Failure::provider(error, "request");
        assert_eq!(failure.kind, Kind::Auth);
        let models_url = format!("{base}/{sign_in}/models");
        let listing = crate::providers::chatgpt::list_models_at(dir.path(), &models_url, "0.161.2");
        // Each case is its own provider to the verdict cache.
        let failure = failure
            .refused_if_listed(
                &format!("chatgpt-{sign_in}"),
                "ChatGPT is refusing requests",
                listing,
            )
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
    let provider = format!("probe-once-{}", uuid::Uuid::new_v4());
    let first = refused()
        .refused_if_listed(&provider, "X is refusing requests", probe())
        .await;
    let second = refused()
        .refused_if_listed(&provider, "X is refusing requests", probe())
        .await;
    assert_eq!(first.kind, Kind::ProviderRefused);
    assert_eq!(second.kind, Kind::ProviderRefused);
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
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

/// Claude Code out of usage, as the adapter forwards it: the sentence comes
/// with a "rate_limit" and sometimes ACP's -32000, and is still the account's
/// limit in the agent's own words, never a wait or a sign-in.
const LIMITED: &str = "Internal error: You've hit your limit \u{b7} resets 3pm (Europe/Lisbon): {\n  \"errorKind\": \"rate_limit\"\n}";

#[test]
fn an_agents_usage_limit_is_its_own_kind_in_its_own_words() {
    let mut limited = Failure::classify(LIMITED, None, "acp_prompt");
    assert_eq!(
        (limited.kind, limited.summary.as_ref()),
        (
            Kind::UsageLimit,
            "You've hit your limit \u{b7} resets 3pm (Europe/Lisbon)"
        )
    );
    assert!(limited.notice().contains(r#""kind":"usage_limit""#));
    assert_eq!(limited.retry_delay(0), None);
    limited.name_the_harness("Claude Code", Some("2.1.0"));
    assert_eq!(limited.title, "Claude Code usage limit reached");

    // Outside a harness the same words are a provider's, read as before.
    assert_eq!(
        Failure::classify(LIMITED, None, "request").kind,
        Kind::RateLimit
    );
    let coded = Failure::classify(
        r#"{"code":-32000,"message":"You've hit your limit · resets 3pm"}"#,
        None,
        "acp_prompt",
    );
    assert_eq!(
        (coded.kind, coded.summary.as_ref()),
        (Kind::UsageLimit, "You've hit your limit \u{b7} resets 3pm")
    );
    let codex = Failure::classify(
        "Internal error: You've hit your usage limit. Upgrade to Pro (https://openai.com/chatgpt/pricing) or try again at 3:45 PM.\n The backend said: retrying in 5s",
        None,
        "acp_prompt",
    );
    assert_eq!(codex.kind, Kind::UsageLimit);
    assert!(
        codex.summary.ends_with("try again at 3:45 PM."),
        "{}",
        codex.summary
    );
    assert!(
        codex.details.contains("retrying in 5s"),
        "{}",
        codex.details
    );
    assert_eq!(
        Failure::classify(
            r#"{"code":-32000,"message":"Authentication required"}"#,
            None,
            "acp_prompt",
        )
        .kind,
        Kind::AgentAuth
    );
}

/// The ChatGPT sign-in's limit, in the body codex-rs reads: a usage limit
/// with the time it resets, never a rate limit to wait out and send again,
/// and the body's reset stands beside whatever a retry-after header says.
#[test]
fn a_chatgpt_usage_limit_says_when_it_resets_whatever_the_header_says() {
    let resets = chrono::Utc::now().timestamp() + 3_600;
    let mut headers = http::HeaderMap::new();
    headers.insert("retry-after", "30".parse().unwrap());
    let error = ProviderError::ProviderResponse(
        rig::ProviderResponseError::new(
            http::StatusCode::TOO_MANY_REQUESTS,
            format!(r#"{{"error":{{"type":"usage_limit_reached","plan_type":"plus","resets_at":{resets}}}}}"#),
        )
        .with_headers(Some(headers)),
    );
    let failure = Failure::provider(error, "request");
    assert_eq!(failure.kind, Kind::UsageLimit);
    assert_eq!(failure.title, "ChatGPT usage limit reached");
    assert_eq!(failure.resets_at, Some(resets * 1000));
    assert_eq!(failure.retry_after_seconds, Some(30));
    assert!(failure.retry_delay(0).is_none());
    assert!(
        failure
            .notice()
            .contains(&format!(r#""resetsAt":{}"#, resets * 1000))
    );
}

#[test]
fn a_chatgpt_usage_limit_gives_no_time_for_a_reset_gone_by() {
    let past = chrono::Utc::now().timestamp() - 60;
    for resets in [
        format!(r#","resets_at":{past}"#),
        r#","resets_at":"soon""#.to_string(),
        String::new(),
    ] {
        let body =
            format!(r#"{{"error":{{"type":"usage_limit_reached","plan_type":"plus"{resets}}}}}"#);
        let failure = Failure::classify(&body, Some(429), "stream");
        assert_eq!(failure.kind, Kind::UsageLimit);
        assert_eq!(
            failure.summary,
            "This ChatGPT sign-in reached its usage limit."
        );
        assert_eq!(failure.resets_at, None);
        assert!(!failure.notice().contains("resetsAt"));
    }
    let rate_limited = Failure::classify(
        r#"{"error":{"type":"rate_limit_exceeded"}}"#,
        Some(429),
        "stream",
    );
    assert_eq!(rate_limited.kind, Kind::RateLimit);
}

/// A notice with nothing to say about accounts is the bytes it was before
/// they could be named on it.
#[test]
fn only_a_card_about_an_account_carries_its_model_and_agent() {
    let plain = Failure::classify("busy", Some(503), "request").notice();
    for field in [r#""model""#, r#""backend""#, r#""resetsAt""#] {
        assert!(!plain.contains(field), "{plain}");
    }
    let gone = Failure::account_gone("gone".into(), Some("anthropic#c-2/claude-x"));
    assert_eq!(gone.model.as_deref(), Some("anthropic/claude-x"));
    assert_eq!(gone.kind, Kind::AccountGone);
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
