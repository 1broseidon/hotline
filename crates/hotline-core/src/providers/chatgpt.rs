//! ChatGPT's model list, read from the same endpoint the Codex CLI reads, and
//! the login's live token for the requests Rig does not build. Rig signs in
//! and refreshes the token, through the login's shared authenticator
//! ([`super::rig_login`]).

use super::discovery::{self, DiscoveryHttp, ListedModel};
use crate::models::Said;
use bytes::Bytes;
use rig::http_client::HttpClientExt;
use serde::Deserialize;
use std::path::Path;

const MODELS_URL: &str = "https://chatgpt.com/backend-api/codex/models";

/// The backend leaves out any model newer than the client that asks, so a
/// listing asks as the Codex CLI installed on the desk, which the person
/// keeps current, and as this release when there is none or it is older.
const CLIENT_VERSION_FLOOR: &str = "0.159.0";

/// How long `codex --version` may take before the floor is used instead.
const VERSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// A live ChatGPT sign-in: the access token and the account it belongs to.
pub(crate) struct AuthRecord {
    pub(crate) access_token: Option<String>,
    pub(crate) account_id: Option<String>,
}

#[derive(Deserialize)]
struct Listing {
    models: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    slug: String,
    display_name: Option<String>,
    visibility: Option<String>,
    supported_in_api: Option<bool>,
    context_window: Option<u64>,
    /// The levels this model takes. The listing's `default_reasoning_level`
    /// is not read: a teammate with no effort stored starts at the desk's
    /// own default (`models::DEFAULT_EFFORT`).
    supported_reasoning_levels: Option<Vec<Level>>,
}

#[derive(Deserialize)]
struct Level {
    effort: Option<String>,
}

/// Levels ChatGPT lists that only its Codex harness can use, which Hotline
/// Agent neither offers nor sends. Empty: every listed level is offered as
/// ChatGPT lists it. A level found to fail when Hotline Agent sends it
/// (`ultra` delegates to subagents in Codex, and may be one) goes here. The
/// kept listing still records it, and the vault drops it as the listing is
/// read ([`offered_by_hotline`]), so it leaves every picker and request
/// without waiting for a refresh.
const HARNESS_ONLY_EFFORTS: &[&str] = &[];

/// Whether Hotline Agent offers and sends a level a listing names.
pub(crate) fn offered_by_hotline(level: &str) -> bool {
    !HARNESS_ONLY_EFFORTS.contains(&level)
}

/// A model's levels as the desk keeps them: least first, each once, and only
/// those [`discovery::effort_id_ok`] lets through, so one odd level never
/// costs the whole listing.
fn efforts(levels: Option<Vec<Level>>) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for effort in levels
        .into_iter()
        .flatten()
        .filter_map(|level| level.effort)
    {
        if discovery::effort_id_ok(&effort) && !kept.contains(&effort) {
            kept.push(effort);
        }
    }
    crate::models::least_first(kept)
}

pub(crate) async fn list_models(token_dir: &Path) -> Result<Vec<ListedModel>, String> {
    let version = client_version(installed_codex_version().await.as_deref());
    list_models_at(token_dir, MODELS_URL, &version).await
}

/// The version a listing asks as: the installed Codex when it is newer than
/// the floor, the floor otherwise.
fn client_version(installed: Option<&str>) -> String {
    installed
        .and_then(release)
        .filter(|found| Some(*found) > release(CLIENT_VERSION_FLOOR))
        .map_or_else(
            || CLIENT_VERSION_FLOOR.to_string(),
            |(major, minor, patch)| format!("{major}.{minor}.{patch}"),
        )
}

/// `major.minor.patch` out of `codex --version`'s line (`codex-cli 0.159.0`),
/// a pre-release suffix dropped.
fn release(line: &str) -> Option<(u64, u64, u64)> {
    let word = line.split_whitespace().last()?.trim_start_matches('v');
    let core = word.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let found = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(found)
}

/// What `codex --version` says, when Codex is on PATH and answers in time.
async fn installed_codex_version() -> Option<String> {
    let codex = crate::driver::acp::registry::which("codex")?;
    let mut command = tokio::process::Command::new(codex);
    command
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    crate::process_windows::quiet(&mut command);
    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(crate) async fn list_models_at(
    token_dir: &Path,
    url: &str,
    version: &str,
) -> Result<Vec<ListedModel>, String> {
    let record = refreshed_auth(token_dir).await?;
    let token = record.access_token.ok_or("ChatGPT sign-in required.")?;

    let mut request = http::Request::get(format!("{url}?client_version={version}"))
        .header(http::header::AUTHORIZATION, format!("Bearer {token}"))
        .header("originator", "codex_cli_rs");
    if let Some(account_id) = record.account_id.filter(|id| !id.is_empty()) {
        request = request.header("ChatGPT-Account-Id", account_id);
    }
    let request = request
        .body(Bytes::new())
        .map_err(|_| discovery::failed())?;
    let body = discovery::fetch(DiscoveryHttp::new()?.send::<Bytes, Vec<u8>>(request)).await?;
    listing(&body)
}

/// `model` on the ChatGPT login in `token_dir`. Rig's ChatGPT dialect writes
/// the request; the transport signs it with the login's live token and
/// account as it goes.
pub(crate) fn model(
    token_dir: &Path,
    model: &str,
) -> Result<rig::DynModel<rig::operation::Completion>, String> {
    let transport = super::outgoing::Outgoing::signed(
        super::rig_login::chatgpt(token_dir),
        std::convert::identity,
    )?;
    let config =
        rig::providers::openai::OpenAIConfig::with_key(&rig::providers::chatgpt::DIALECT, "");
    #[cfg(feature = "test-support")]
    let config = match crate::provider_fixture::origin(&token_dir.to_string_lossy()) {
        Some(origin) => config.with_base_url(origin),
        None => config,
    };
    Ok(config
        .with_auth(rig::providers::openai::wire::Auth::OptionalBearer)
        .connect(transport)
        .completion(model)
        .erase())
}

/// The login's live token, refreshed first when it has expired. With no
/// sign-in in the record this fails, naming the sign-in, and never starts
/// one. A keychain that would not give the record up says so instead:
/// signing in again would meet the same keychain.
pub(crate) async fn refreshed_auth(token_dir: &Path) -> Result<AuthRecord, String> {
    let context = super::rig_login::chatgpt(token_dir)
        .auth_context(&super::rig_login::http())
        .await
        .map_err(|error| match error {
            rig::providers::chatgpt::auth::AuthError::Store(_) => error.to_string(),
            _ => "The ChatGPT sign-in could not be refreshed. Sign in again.".to_string(),
        })?;
    let token = context.api_key.expose();
    if token.trim().is_empty() {
        return Err("ChatGPT sign-in required. Sign in again under Settings → Providers.".into());
    }
    Ok(AuthRecord {
        access_token: Some(token.to_string()),
        account_id: context.account_id,
    })
}

/// The models a ChatGPT sign-in offers: listed ones only, since the backend
/// also returns hidden models the Codex picker never shows.
fn listing(body: &[u8]) -> Result<Vec<ListedModel>, String> {
    let listed: Listing = serde_json::from_slice(body).map_err(|_| discovery::failed())?;
    let models = listed
        .models
        .into_iter()
        .filter(|entry| {
            entry
                .visibility
                .as_deref()
                .is_none_or(|seen| seen == "list")
        })
        .filter(|entry| entry.supported_in_api != Some(false))
        .map(|entry| {
            let model = ListedModel {
                id: entry.slug,
                name: entry.display_name,
                context_limit: entry.context_window,
                output_limit: None,
                efforts: efforts(entry.supported_reasoning_levels),
            };
            (model, Said::Nothing)
        })
        .collect();
    discovery::finish("openai-codex", models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get};
    use serde_json::json;

    #[test]
    fn only_listed_api_models_are_offered_with_their_names_and_windows() {
        let body = json!({"models": [
            {"slug": "gpt-6-sol", "display_name": "GPT-6-Sol", "visibility": "list",
             "supported_in_api": true, "context_window": 272_000, "shell_type": "unified_exec"},
            {"slug": "gpt-reserve", "visibility": "hide", "supported_in_api": true},
            {"slug": "app-only", "visibility": "list", "supported_in_api": false},
            {"slug": "gpt-5.5", "visibility": "list"},
        ]})
        .to_string();
        let models = listing(body.as_bytes()).unwrap();
        assert_eq!(
            models,
            [
                ListedModel {
                    id: "gpt-5.5".into(),
                    name: None,
                    context_limit: None,
                    output_limit: None,
                    efforts: Vec::new(),
                },
                ListedModel {
                    id: "gpt-6-sol".into(),
                    name: Some("GPT-6-Sol".into()),
                    context_limit: Some(272_000),
                    output_limit: None,
                    efforts: Vec::new(),
                },
            ]
        );
        assert!(listing(b"<html>").is_err());
    }

    /// Each model keeps the levels ChatGPT lists for it, least first, the
    /// listing's own default not taken. An entry the shape of the live
    /// listing (the Codex CLI's cached copy of the same endpoint).
    #[test]
    fn each_listed_model_keeps_its_own_effort_levels_least_first() {
        let level = |effort: &str| json!({"effort": effort, "description": "…"});
        let body = json!({"models": [
            {"slug": "gpt-6.2-sol", "display_name": "GPT-6.2-Sol", "visibility": "list",
             "supported_in_api": true, "context_window": 500_000,
             "default_reasoning_level": "low",
             "supported_reasoning_levels": [level("low"), level("medium"), level("high"),
                 level("xhigh"), level("max"), level("ultra")],
             "supports_reasoning_effort_updates": true},
            // Most first, a level twice, and one no picker could show.
            {"slug": "gpt-shuffled", "visibility": "list",
             "supported_reasoning_levels": [level("high"), level("low"), level("high"),
                 level("Bad Level\n"), {"description": "no effort"}]},
            {"slug": "gpt-unsaid", "visibility": "list"},
            {"slug": "gpt-null", "visibility": "list", "supported_reasoning_levels": null},
        ]})
        .to_string();
        let levels: Vec<(String, Vec<String>)> = listing(body.as_bytes())
            .unwrap()
            .into_iter()
            .map(|model| (model.id, model.efforts))
            .collect();
        let owned = |ids: &[&str]| ids.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(
            levels,
            [
                (
                    "gpt-6.2-sol".into(),
                    owned(&["low", "medium", "high", "xhigh", "max", "ultra"])
                ),
                ("gpt-null".into(), Vec::new()),
                ("gpt-shuffled".into(), owned(&["low", "high"])),
                ("gpt-unsaid".into(), Vec::new()),
            ]
        );
    }

    #[test]
    fn a_listing_asks_as_the_installed_codex_when_it_is_newer_than_the_floor() {
        assert_eq!(client_version(Some("codex-cli 9.1.0")), "9.1.0");
        assert_eq!(client_version(Some("codex-cli 9.2.0-alpha.3")), "9.2.0");
        assert_eq!(
            client_version(Some("codex-cli 0.1.0")),
            CLIENT_VERSION_FLOOR
        );
        assert_eq!(client_version(Some("not a version")), CLIENT_VERSION_FLOOR);
        assert_eq!(client_version(Some("codex-cli 1.2")), CLIENT_VERSION_FLOOR);
        assert_eq!(client_version(None), CLIENT_VERSION_FLOOR);
    }

    #[tokio::test]
    async fn a_listing_sends_the_sign_in_account_and_client_version() {
        let dir = std::env::temp_dir().join(format!("hotline-chatgpt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let far = chrono::Utc::now().timestamp() + 3600;
        std::fs::write(
            dir.join("auth.json"),
            json!({"access_token": "chatgpt-access", "refresh_token": "r",
                   "expires_at": far, "account_id": "acct-1"})
            .to_string(),
        )
        .unwrap();
        let app = Router::new().route(
            "/models",
            get(
                |headers: http::HeaderMap, query: axum::extract::RawQuery| async move {
                    assert_eq!(headers["authorization"], "Bearer chatgpt-access");
                    assert_eq!(headers["chatgpt-account-id"], "acct-1");
                    assert_eq!(query.0.as_deref(), Some("client_version=0.161.2"));
                    (
                        [("content-type", "application/json")],
                        json!({"models": [{"slug": "gpt-6-luna", "visibility": "list"}]})
                            .to_string(),
                    )
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/models", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let models = list_models_at(&dir, &url, "0.161.2").await;
        server.abort();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            models
                .unwrap()
                .into_iter()
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            ["gpt-6-luna"]
        );
    }
}
