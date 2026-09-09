//! Connection work Rig does not own. Model requests and model discovery stay
//! on Rig's native provider clients.

pub(crate) mod xai;

use oauth2::PkceCodeChallenge;
use rig::client::ModelListingClient;
use rig::providers::ollama;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use url::Url;

pub(crate) const OLLAMA_CLOUD_URL: &str = "https://ollama.com";

pub(crate) fn ollama_url(input: &str) -> Result<String, String> {
    let url =
        Url::parse(input.trim()).map_err(|_| "Enter a full Ollama server URL.".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Use an HTTP or HTTPS server URL without credentials, a query, or a fragment.".into(),
        );
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub(crate) fn ollama_client(base_url: &str, key: &str) -> Result<ollama::Client, String> {
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        // Credentials must never follow a redirect to another server.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?;
    ollama::Client::builder()
        .api_key(key)
        .base_url(base_url)
        .http_client(http)
        .build()
        .map_err(|error| error.to_string())
}

pub(crate) async fn ollama_models(base_url: &str, key: &str) -> Result<Vec<String>, String> {
    let client = ollama_client(base_url, key)?;
    let models = tokio::time::timeout(Duration::from_secs(30), client.list_models())
        .await
        .map_err(|_| {
            "Ollama model discovery timed out. Check that the server is running.".to_string()
        })?
        .map_err(|error| format!("Could not read Ollama models: {error}"))?;
    let mut ids: Vec<String> = models
        .data
        .into_iter()
        .map(|model| model.id)
        .filter(|id| !id.trim().is_empty())
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[derive(Deserialize, Serialize)]
struct OpenRouterKey {
    key: String,
}

pub(crate) fn openrouter_key(token_dir: &Path) -> Result<String, String> {
    let bytes = std::fs::read(token_dir.join("auth.json"))
        .map_err(|_| "OpenRouter sign-in is missing. Sign in again.".to_string())?;
    let auth: OpenRouterKey = serde_json::from_slice(&bytes)
        .map_err(|_| "OpenRouter sign-in could not be read. Sign in again.".to_string())?;
    if auth.key.trim().is_empty() {
        return Err("OpenRouter sign-in has no key. Sign in again.".into());
    }
    Ok(auth.key)
}

pub(crate) async fn openrouter_login(
    token_dir: &Path,
    emit: impl FnOnce(String, String),
) -> Result<(), String> {
    openrouter_login_at(
        token_dir,
        emit,
        "https://openrouter.ai/auth",
        "https://openrouter.ai/api/v1/auth/keys",
    )
    .await
}

/// Dropping the login future (cancel, timeout, or shutdown) closes its listener.
struct CallbackServer(tokio::task::JoinHandle<()>);
impl Drop for CallbackServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn openrouter_login_at(
    token_dir: &Path,
    emit: impl FnOnce(String, String),
    authorize_url: &str,
    exchange_url: &str,
) -> Result<(), String> {
    use axum::{
        Router,
        http::{StatusCode, Uri},
        routing::get,
    };
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| format!("Could not open the sign-in callback: {error}"))?;
    // OpenRouter does not return OAuth state. An unguessable callback path
    // binds the browser response to this login; PKCE binds the exchanged code.
    let path = format!("/oauth/callback/{}", uuid::Uuid::new_v4());
    let callback = format!(
        "http://{}{}",
        listener.local_addr().map_err(|error| error.to_string())?,
        path
    );
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let app = Router::new().route(
        &path,
        get(move |uri: Uri| {
            let tx = tx.clone();
            async move {
                let query: Vec<_> =
                    url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes()).collect();
                let codes: Vec<_> = query.iter().filter(|(name, _)| name == "code").collect();
                let result = if query.iter().any(|(name, _)| name == "error") {
                    Err("OpenRouter sign-in was declined. Try signing in again.".to_string())
                } else if codes.len() == 1
                    && !codes[0].1.trim().is_empty()
                    && codes[0].1.len() <= 8192
                {
                    Ok(codes[0].1.to_string())
                } else {
                    return (
                        StatusCode::BAD_REQUEST,
                        [
                            ("Cache-Control", "no-store"),
                            ("Referrer-Policy", "no-referrer"),
                        ],
                        "Missing or invalid sign-in code.",
                    );
                };
                if let Some(tx) = tx
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                {
                    let _ = tx.send(result);
                }
                (
                    StatusCode::OK,
                    [
                        ("Cache-Control", "no-store"),
                        ("Referrer-Policy", "no-referrer"),
                    ],
                    "Return to Toad to finish signing in. You can close this page.",
                )
            }
        }),
    );
    let _server = CallbackServer(tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    }));
    let mut url = Url::parse(authorize_url).map_err(|error| error.to_string())?;
    url.query_pairs_mut()
        .append_pair("callback_url", &callback)
        .append_pair("code_challenge", challenge.as_str())
        .append_pair("code_challenge_method", "S256");
    emit(String::new(), url.to_string());
    let code = tokio::time::timeout(Duration::from_secs(5 * 60), rx)
        .await
        .map_err(|_| "OpenRouter sign-in timed out. Try again.".to_string())?
        .map_err(|_| "The sign-in callback closed.".to_string())??;
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?
        .post(exchange_url)
        .json(&serde_json::json!({
            "code": code, "code_verifier": verifier.secret(), "code_challenge_method": "S256"
        }))
        .send()
        .await
        .map_err(|_| "Could not exchange the OpenRouter sign-in code. Try again.".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "OpenRouter refused the sign-in code (HTTP {}). Try again.",
            response.status().as_u16()
        ));
    }
    let auth: OpenRouterKey = response
        .json()
        .await
        .map_err(|_| "OpenRouter returned an invalid sign-in response.".to_string())?;
    if auth.key.trim().is_empty() {
        return Err("OpenRouter returned an empty key.".into());
    }
    let bytes = serde_json::to_vec(&auth).map_err(|error| error.to_string())?;
    crate::vault::write_private(&token_dir.join("auth.json"), &bytes)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Bytes,
        http::HeaderMap,
        routing::{get, post},
    };
    use serde_json::{Value, json};

    async fn serve(app: Router) -> (String, CallbackServer) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = CallbackServer(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        (url, task)
    }

    #[test]
    fn ollama_urls_preserve_reverse_proxy_paths_and_refuse_embedded_secrets() {
        assert_eq!(
            ollama_url(" http://localhost:11434/ ").unwrap(),
            "http://localhost:11434"
        );
        assert_eq!(
            ollama_url("https://models.example/ollama/").unwrap(),
            "https://models.example/ollama"
        );
        for url in [
            "localhost:11434",
            "file:///tmp/socket",
            "http://user:secret@localhost",
            "https://host/?key=secret",
            "https://host/#secret",
        ] {
            assert!(ollama_url(url).is_err(), "{url}");
        }
    }

    #[tokio::test]
    async fn ollama_discovery_uses_rig_and_sends_only_the_selected_key() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let requests = seen.clone();
        let (url, _server) = serve(Router::new().route(
            "/api/tags",
            get(move |headers: HeaderMap| {
                let requests = requests.clone();
                async move {
                    requests.lock().unwrap().push(
                        headers
                            .get("authorization")
                            .map(|value| value.to_str().unwrap().to_string()),
                    );
                    json!({"models": [
                        {"name": "custom/coder:latest", "model": "custom/coder:latest"},
                        {"name": "cloud-model:cloud", "model": "cloud-model:cloud"},
                        {"name": "custom/coder:latest", "model": "custom/coder:latest"}
                    ]})
                    .to_string()
                }
            }),
        ))
        .await;
        assert_eq!(
            ollama_models(&url, "").await.unwrap(),
            ["cloud-model:cloud", "custom/coder:latest"]
        );
        assert_eq!(
            ollama_models(&url, "cloud-test-key").await.unwrap().len(),
            2
        );
        assert_eq!(
            *seen.lock().unwrap(),
            [None, Some("Bearer cloud-test-key".into())]
        );
    }

    #[tokio::test]
    async fn openrouter_pkce_exchanges_once_and_keeps_the_key_private() {
        let root = std::env::temp_dir().join(format!("toad-openrouter-{}", uuid::Uuid::new_v4()));
        let log = crate::log::Log::open(&root);
        let vault = crate::vault::Vault::open(&root, log.clone()).unwrap();
        let (id, dir) = vault.begin_login("openrouter").unwrap();
        let (exchange_tx, mut exchange_rx) = tokio::sync::mpsc::channel(1);
        let app = Router::new().route(
            "/keys",
            post(move |bytes: Bytes| {
                let tx = exchange_tx.clone();
                async move {
                    tx.send(serde_json::from_slice::<Value>(&bytes).unwrap())
                        .await
                        .unwrap();
                    json!({"key": "openrouter-test-secret"}).to_string()
                }
            }),
        );
        let (url, _server) = serve(app).await;
        let (prompt_tx, prompt_rx) = tokio::sync::oneshot::channel();
        let target = dir.clone();
        let login = tokio::spawn(async move {
            openrouter_login_at(
                &target,
                |code, url| {
                    assert!(code.is_empty());
                    prompt_tx.send(url).unwrap();
                },
                "https://openrouter.ai/auth",
                &format!("{url}/keys"),
            )
            .await
        });
        let prompt = Url::parse(&prompt_rx.await.unwrap()).unwrap();
        let params: std::collections::HashMap<_, _> = prompt.query_pairs().into_owned().collect();
        assert_eq!(params["code_challenge_method"], "S256");
        let callback = Url::parse(&params["callback_url"]).unwrap();
        assert_eq!(callback.host_str(), Some("127.0.0.1"));
        let http = reqwest::Client::new();
        assert_eq!(
            http.get(callback.join("/wrong").unwrap())
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            http.get(callback.clone()).send().await.unwrap().status(),
            400
        );
        let mut finished = callback.clone();
        finished
            .query_pairs_mut()
            .append_pair("code", "single-use-code");
        let response = http.get(finished).send().await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let exchange = exchange_rx.recv().await.unwrap();
        assert_eq!(exchange["code"], "single-use-code");
        assert_eq!(exchange["code_challenge_method"], "S256");
        let verifier =
            oauth2::PkceCodeVerifier::new(exchange["code_verifier"].as_str().unwrap().to_string());
        assert_eq!(
            PkceCodeChallenge::from_code_verifier_sha256(&verifier).as_str(),
            params["code_challenge"]
        );
        login.await.unwrap().unwrap();
        assert_eq!(openrouter_key(&dir).unwrap(), "openrouter-test-secret");
        vault.finish_login(&id, "openrouter", "OpenRouter").unwrap();
        assert!(
            !serde_json::to_string(&log.load(&crate::log::StreamId::Room))
                .unwrap()
                .contains("openrouter-test-secret")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.join("auth.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        vault.delete(&id).unwrap();
        assert!(openrouter_key(&dir).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn openrouter_rejects_empty_keys_and_redacts_failed_exchange_bodies() {
        for (status, body) in [
            (200, r#"{"key":""}"#),
            (403, "a response containing a secret"),
            (200, "not json"),
        ] {
            let app =
                Router::new().route(
                    "/keys",
                    post(move || async move {
                        (axum::http::StatusCode::from_u16(status).unwrap(), body)
                    }),
                );
            let (url, _server) = serve(app).await;
            let dir =
                std::env::temp_dir().join(format!("toad-openrouter-bad-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&dir).unwrap();
            let target = dir.clone();
            let (tx, rx) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                openrouter_login_at(
                    &target,
                    |_, url| {
                        let _ = tx.send(url);
                    },
                    "https://openrouter.ai/auth",
                    &format!("{url}/keys"),
                )
                .await
            });
            let prompt = Url::parse(&rx.await.unwrap()).unwrap();
            let callback = prompt
                .query_pairs()
                .find(|(key, _)| key == "callback_url")
                .unwrap()
                .1
                .into_owned();
            reqwest::get(format!("{callback}?code=test")).await.unwrap();
            let error = task.await.unwrap().unwrap_err();
            assert!(!error.contains("a response containing a secret"));
            assert!(!dir.join("auth.json").exists());
            std::fs::remove_dir(dir).unwrap();
        }
    }
}
