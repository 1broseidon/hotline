//! ChatGPT's model list, read from the same endpoint the Codex CLI reads.
//! Rig signs in and refreshes the token; this module only lists models.

use super::discovery::{self, DiscoveryHttp, ListedModel};
use bytes::Bytes;
use rig::http_client::HttpClientExt;
use serde::Deserialize;
use std::path::Path;

const MODELS_URL: &str = "https://chatgpt.com/backend-api/codex/models";

/// The backend leaves out any model newer than the client that asks, so this
/// tracks a recent Codex CLI release. Raise it when a new ChatGPT model is
/// missing from a refreshed list.
const CLIENT_VERSION: &str = "0.155.0";

#[derive(Deserialize)]
struct AuthRecord {
    access_token: Option<String>,
    account_id: Option<String>,
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
}

pub(crate) async fn list_models(token_dir: &Path) -> Result<Vec<ListedModel>, String> {
    list_models_at(token_dir, MODELS_URL).await
}

async fn list_models_at(token_dir: &Path, url: &str) -> Result<Vec<ListedModel>, String> {
    let auth_file = token_dir.join("auth.json");
    rig::providers::chatgpt::Client::builder()
        .oauth()
        .auth_file(&auth_file)
        .allow_device_flow(false)
        .build()
        .map_err(|_| "Could not prepare model discovery.".to_string())?
        .authorize()
        .await
        .map_err(|_| "The ChatGPT sign-in could not be refreshed. Sign in again.".to_string())?;
    let record: AuthRecord = crate::vault::read_model_file(&auth_file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or("The ChatGPT sign-in could not be read. Sign in again.")?;
    let token = record
        .access_token
        .filter(|token| !token.trim().is_empty())
        .ok_or("ChatGPT sign-in required. Sign in again under Settings → Providers.")?;

    let mut request = http::Request::get(format!("{url}?client_version={CLIENT_VERSION}"))
        .header(http::header::AUTHORIZATION, format!("Bearer {token}"))
        .header("originator", "codex_cli_rs");
    if let Some(account_id) = record.account_id.filter(|id| !id.is_empty()) {
        request = request.header("ChatGPT-Account-Id", account_id);
    }
    let request = request
        .body(Bytes::new())
        .map_err(|_| discovery::failed())?;
    let body = discovery::fetch(DiscoveryHttp::default().send::<Bytes, Vec<u8>>(request)).await?;
    listing(&body)
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
        .map(|entry| ListedModel {
            id: entry.slug,
            name: entry.display_name,
            context_limit: entry.context_window,
            output_limit: None,
        })
        .collect();
    discovery::finish(models)
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
             "supported_in_api": true, "context_window": 272000, "shell_type": "unified_exec"},
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
                },
                ListedModel {
                    id: "gpt-6-sol".into(),
                    name: Some("GPT-6-Sol".into()),
                    context_limit: Some(272000),
                    output_limit: None,
                },
            ]
        );
        assert!(listing(b"<html>").is_err());
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
                    assert_eq!(
                        query.0.as_deref(),
                        Some(format!("client_version={CLIENT_VERSION}").as_str())
                    );
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
        let models = list_models_at(&dir, &url).await;
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
