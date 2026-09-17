//! Native Rig listing with bounded connection metadata and fixed provider wiring.

use crate::models::Client;
use bytes::Bytes;
use rig::client::ModelListingClient;
use rig::http_client::{self, HttpClientExt, LazyBody, MultipartForm, StreamingResponse};
use rig::providers::{
    anthropic, copilot, deepseek, gemini, groq, mistral, ollama, openai, openrouter,
};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_MODELS: usize = 10_000;
const MAX_CONTEXT: u64 = 100_000_000;
const MAX_OUTPUT: u64 = 1_000_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListedModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_limit: Option<u64>,
}

pub(crate) fn validate_ids(ids: &[String]) -> Result<Vec<String>, String> {
    if ids.len() > MAX_MODELS {
        return Err("Too many model IDs (maximum 10000).".into());
    }
    for id in ids {
        if id.is_empty()
            || id.len() > 256
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-._:/@+".contains(&byte))
            || id
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | ".."))
        {
            return Err("Model IDs must be 1–256 ASCII letters, digits, or - . _ : / @ +, without empty or relative path segments.".into());
        }
    }
    let mut ids = ids.to_vec();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub(crate) fn validate(models: &[ListedModel]) -> Result<(), String> {
    if models.len() > MAX_MODELS {
        return Err("The provider returned too many models.".into());
    }
    validate_ids(
        &models
            .iter()
            .map(|model| model.id.clone())
            .collect::<Vec<_>>(),
    )?;
    for model in models {
        if model.name.as_ref().is_some_and(|name| {
            name.trim().is_empty() || name.len() > 256 || name.chars().any(|c| {
                c.is_control() || matches!(c, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
            })
        }) || model.context_limit.is_some_and(|limit| !(1..=MAX_CONTEXT).contains(&limit))
            || model.output_limit.is_some_and(|limit| !(1..=MAX_OUTPUT).contains(&limit)) {
            return Err("The provider returned invalid model metadata.".into());
        }
    }
    Ok(())
}

/// This transport is used only for listing. Rig still owns URLs, auth, paging,
/// and parsing; the shared budget bounds all pages before they reach its parser.
#[derive(Clone, Debug)]
pub(crate) struct DiscoveryHttp {
    http: reqwest::Client,
    bytes: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
}

impl Default for DiscoveryHttp {
    fn default() -> Self {
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("the discovery HTTP configuration is valid"),
            bytes: Arc::new(AtomicUsize::new(0)),
            requests: Arc::new(AtomicUsize::new(0)),
        }
    }
}

fn refused(message: &'static str) -> http_client::Error {
    http_client::Error::Instance(Box::new(std::io::Error::other(message)))
}

impl HttpClientExt for DiscoveryHttp {
    fn send<T, U>(
        &self,
        request: http::Request<T>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        let this = self.clone();
        let request = request.map(Into::into);
        async move {
            if this.requests.fetch_add(1, Ordering::Relaxed) >= 64 {
                return Err(refused("Model discovery exceeded 64 requests."));
            }
            let (parts, body) = request.into_parts();
            let mut response = this
                .http
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .body(body)
                .send()
                .await
                .map_err(|_| refused("Model discovery request failed."))?;
            if !response.status().is_success() {
                return Err(http_client::Error::InvalidStatusCode(response.status()));
            }
            if response
                .content_length()
                .is_some_and(|size| size > MAX_BYTES as u64)
            {
                return Err(refused("Model discovery response is too large."));
            }
            let mut result = http::Response::builder().status(response.status());
            *result.headers_mut().expect("valid response") = response.headers().clone();
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| refused("Model discovery response failed."))?
            {
                if this
                    .bytes
                    .fetch_add(chunk.len(), Ordering::Relaxed)
                    .saturating_add(chunk.len())
                    > MAX_BYTES
                {
                    return Err(refused("Model discovery response is too large."));
                }
                bytes.extend_from_slice(&chunk);
            }
            let body: LazyBody<U> = Box::pin(async move { Ok(U::from(Bytes::from(bytes))) });
            result.body(body).map_err(Into::into)
        }
    }

    fn send_multipart<U>(
        &self,
        _: http::Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        std::future::ready(Err(refused(
            "Model discovery does not send multipart requests.",
        )))
    }

    async fn send_streaming<T>(&self, _: http::Request<T>) -> http_client::Result<StreamingResponse>
    where
        T: Into<Bytes> + Send,
    {
        Err(refused("Model discovery does not open streaming requests."))
    }
}

pub(crate) async fn collect(client: &impl ModelListingClient) -> Result<Vec<ListedModel>, String> {
    let listed = tokio::time::timeout(Duration::from_secs(30), client.list_models()).await
        .map_err(|_| "Model discovery timed out; the previous list is unchanged.".to_string())?
        .map_err(|_| "Could not discover models. Check this connection and try again; the previous list is unchanged.".to_string())?;
    if listed.len() > MAX_MODELS {
        return Err("The provider returned too many models.".into());
    }
    let mut models: Vec<_> = listed
        .data
        .into_iter()
        .filter(|model| !matches!(model.r#type.as_deref(), Some("embedding" | "embeddings")))
        .map(|model| ListedModel {
            id: model.id,
            name: model.name,
            context_limit: model.context_length.map(u64::from),
            output_limit: model.max_output_tokens.map(u64::from),
        })
        .collect();
    validate(&models)?;
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    Ok(models)
}

pub(crate) async fn api_models(provider: Client, key: &str) -> Result<Vec<ListedModel>, String> {
    api_models_at(provider, key, None).await
}

async fn api_models_at(
    provider: Client,
    key: &str,
    base_url: Option<&str>,
) -> Result<Vec<ListedModel>, String> {
    macro_rules! list {
        ($provider:ident) => {{
            let builder = $provider::Client::builder()
                .api_key(key)
                .http_client(DiscoveryHttp::default());
            let builder = if let Some(base_url) = base_url {
                builder.base_url(base_url)
            } else {
                builder
            };
            collect(
                &builder
                    .build()
                    .map_err(|_| "Could not prepare model discovery.".to_string())?,
            )
            .await
        }};
    }
    match provider {
        Client::Anthropic => list!(anthropic),
        Client::OpenAi => list!(openai),
        Client::OpenRouter => list!(openrouter),
        Client::Gemini => list!(gemini),
        Client::Groq => list!(groq),
        Client::DeepSeek => list!(deepseek),
        Client::Mistral => list!(mistral),
        _ => Err("This provider does not support native model discovery.".into()),
    }
}

pub(crate) async fn ollama_models(base_url: &str, key: &str) -> Result<Vec<ListedModel>, String> {
    let client = ollama::Client::builder()
        .api_key(key)
        .base_url(base_url)
        .http_client(DiscoveryHttp::default())
        .build()
        .map_err(|_| "Could not prepare Ollama model discovery.".to_string())?;
    collect(&client).await
}

pub(crate) async fn copilot_models(
    token_dir: &std::path::Path,
) -> Result<Vec<ListedModel>, String> {
    let client = copilot::Client::builder()
        .oauth()
        .token_dir(token_dir)
        .allow_device_flow(false)
        .http_client(DiscoveryHttp::default())
        .build()
        .map_err(|_| "Could not prepare Copilot model discovery. Sign in again.".to_string())?;
    collect(&client).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        http::{HeaderMap, StatusCode, Uri},
        routing::get,
    };
    use serde_json::json;

    struct Server(tokio::task::JoinHandle<()>);
    impl Drop for Server {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    async fn serve(app: Router) -> (String, Server) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        (
            url,
            Server(tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            })),
        )
    }

    #[tokio::test]
    async fn native_clients_keep_provider_paths_auth_and_listing_metadata() {
        for (provider, path, body, name, context, output) in [
            (
                Client::Anthropic,
                "/v1/models",
                json!({"data":[{"id":"new-coder", "display_name":"New coder"}], "has_more":false}),
                Some("New coder"),
                None,
                None,
            ),
            (
                Client::OpenAi,
                "/models",
                json!({"data":[{"id":"new-coder", "owned_by":"untrusted-vendor"}]}),
                None,
                None,
                None,
            ),
            (
                Client::OpenRouter,
                "/models",
                json!({"data":[{"id":"new-coder", "name":"New coder", "created":1, "context_length":100000, "top_provider":{"max_completion_tokens":12000}}]}),
                Some("New coder"),
                Some(100000),
                Some(12000),
            ),
            (
                Client::Gemini,
                "/v1beta/models",
                json!({"models":[{"name":"models/new-coder", "displayName":"New coder", "inputTokenLimit":100000, "outputTokenLimit":12000}]}),
                Some("New coder"),
                Some(100000),
                Some(12000),
            ),
            (
                Client::Groq,
                "/models",
                json!({"data":[{"id":"new-coder", "context_window":100000, "max_completion_tokens":12000}]}),
                None,
                Some(100000),
                Some(12000),
            ),
            (
                Client::DeepSeek,
                "/models",
                json!({"data":[{"id":"new-coder"}]}),
                None,
                None,
                None,
            ),
            (
                Client::Mistral,
                "/v1/models",
                json!({"data":[{"id":"new-coder", "max_context_length":100000}]}),
                None,
                Some(100000),
                None,
            ),
        ] {
            let (url, _server) = serve(Router::new().route(
                path,
                get(move |headers: HeaderMap, uri: Uri| {
                    let body = body.clone();
                    async move {
                        match provider {
                            Client::Anthropic => assert_eq!(headers["x-api-key"], "test-only-key"),
                            Client::Gemini => assert!(
                                uri.query()
                                    .is_some_and(|query| query.contains("key=test-only-key"))
                            ),
                            _ => assert_eq!(headers["authorization"], "Bearer test-only-key"),
                        }
                        if provider != Client::Gemini {
                            assert!(!uri.to_string().contains("test-only-key"));
                        }
                        body.to_string()
                    }
                }),
            ))
            .await;
            let models = api_models_at(provider, "test-only-key", Some(&url))
                .await
                .unwrap_or_else(|error| panic!("{provider:?}: {error}"));
            assert_eq!(
                models,
                [ListedModel {
                    id: "new-coder".into(),
                    name: name.map(str::to_string),
                    context_limit: context,
                    output_limit: output
                }],
                "{provider:?}"
            );
        }
    }

    #[tokio::test]
    async fn native_pagination_is_bounded_and_keeps_both_pages() {
        let (url, _server) = serve(Router::new().route("/v1/models", get(|uri: Uri| async move {
            if uri.query().is_some() {
                json!({"data":[{"id":"second", "display_name":"Second"}], "has_more":false}).to_string()
            } else {
                json!({"data":[{"id":"first", "display_name":"First"}], "has_more":true, "last_id":"first"}).to_string()
            }
        }))).await;
        let models = api_models_at(Client::Anthropic, "test", Some(&url))
            .await
            .unwrap();
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["first", "second"]
        );
        let page = Arc::new(AtomicUsize::new(0));
        let counter = page.clone();
        let (url, _server) = serve(Router::new().route("/v1/models", get(move || {
            let counter = counter.clone();
            async move {
                let id = counter.fetch_add(1, Ordering::Relaxed).to_string();
                json!({"data":[{"id":id, "display_name":"Page"}], "has_more":true, "last_id":id}).to_string()
            }
        }))).await;
        assert!(
            api_models_at(Client::Anthropic, "test", Some(&url))
                .await
                .is_err()
        );
        assert_eq!(page.load(Ordering::Relaxed), 64);
    }

    #[tokio::test]
    async fn oversized_responses_and_redirects_fail_without_exposing_response_text() {
        for (status, body) in [
            (StatusCode::OK, "x".repeat(MAX_BYTES + 1)),
            (StatusCode::UNAUTHORIZED, "private upstream secret".into()),
        ] {
            let (url, _server) = serve(Router::new().route(
                "/models",
                get(move || {
                    let body = body.clone();
                    async move { (status, body) }
                }),
            ))
            .await;
            let error = api_models_at(Client::OpenAi, "test-only-key", Some(&url))
                .await
                .unwrap_err();
            assert!(!error.contains("private upstream secret"));
            assert!(!error.contains("test-only-key"));
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let (target, _server) = serve(Router::new().route(
            "/models",
            get(move || {
                counter.fetch_add(1, Ordering::Relaxed);
                async { "{}" }
            }),
        ))
        .await;
        let (url, _redirect) = serve(Router::new().route(
            "/models",
            get(move || {
                let target = format!("{target}/models");
                async move { (StatusCode::FOUND, [("location", target)]) }
            }),
        ))
        .await;
        assert!(
            api_models_at(Client::OpenAi, "test", Some(&url))
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn invalid_ids_labels_and_limits_are_rejected_without_inventing_capabilities() {
        for id in [
            "",
            "../model",
            "model\nrun",
            "model name",
            "evil\u{202e}id",
            "https://other/model",
            "/absolute",
        ] {
            assert!(validate_ids(&[id.into()]).is_err(), "{id:?}");
        }
        assert!(validate_ids(&["x".repeat(257)]).is_err());
        assert!(validate_ids(&vec!["model".into(); MAX_MODELS + 1]).is_err());
        for model in [
            ListedModel {
                id: "coder".into(),
                name: Some("Click\nRun".into()),
                context_limit: None,
                output_limit: None,
            },
            ListedModel {
                id: "coder".into(),
                name: Some("Fake\u{202e} provider".into()),
                context_limit: None,
                output_limit: None,
            },
            ListedModel {
                id: "coder".into(),
                name: None,
                context_limit: Some(0),
                output_limit: None,
            },
            ListedModel {
                id: "coder".into(),
                name: None,
                context_limit: None,
                output_limit: Some(MAX_OUTPUT + 1),
            },
        ] {
            assert!(validate(&[model]).is_err());
        }
        assert!(
            serde_json::from_value::<ListedModel>(
                json!({"id":"coder","endpoint":"https://evil.example"})
            )
            .is_err()
        );
    }
}
