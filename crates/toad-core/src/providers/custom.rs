//! OpenAI-compatible connections use Rig's native request and streaming code.

use crate::contract::CustomProviderDraft;
use rig::client::{ApiKey, ClientBuilder, ModelListingClient, ProviderBuilder};
use rig::http_client::{self, HttpClientExt};
use rig::providers::openai;
use std::time::Duration;

pub(crate) fn server_url(input: &str) -> Result<String, String> {
    let url = url::Url::parse(input.trim()).map_err(|_| "Enter a full server URL.".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Use an HTTP or HTTPS base URL without credentials, a query, or a fragment.".into(),
        );
    }
    Ok(url.as_str().trim_end_matches('/').into())
}

pub(crate) fn validate(mut draft: CustomProviderDraft) -> Result<CustomProviderDraft, String> {
    draft.name = draft.name.trim().into();
    if draft.name.is_empty() {
        return Err("Give this connection a name.".into());
    }
    draft.base_url = server_url(&draft.base_url)?;
    draft.models = model_ids(draft.models)?;
    if draft.models.is_empty() {
        return Err("Discover models or enter at least one model ID.".into());
    }
    Ok(draft)
}

fn model_ids(ids: Vec<String>) -> Result<Vec<String>, String> {
    let mut ids: Vec<_> = ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if ids.iter().any(|id| id.chars().any(char::is_control)) {
        return Err("Model IDs cannot contain control characters.".into());
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

struct OptionalKey(Option<String>);
impl ApiKey for OptionalKey {
    fn into_header(
        self,
    ) -> Option<http_client::Result<(http::header::HeaderName, http::HeaderValue)>> {
        self.0.map(http_client::make_auth_header)
    }
}

// Rig's OpenAI builder requires a bearer. This builder changes only that
// requirement, so a keyless server receives no fabricated authorization header.
#[derive(Clone, Default)]
struct CompatibleBuilder;
impl ProviderBuilder for CompatibleBuilder {
    type ApiKey = OptionalKey;
    type Extension<H>
        = openai::OpenAIResponsesExt
    where
        H: HttpClientExt;
    const BASE_URL: &'static str = "";

    fn build<H: HttpClientExt>(
        _: &ClientBuilder<Self, Self::ApiKey, H>,
    ) -> http_client::Result<Self::Extension<H>> {
        Ok(openai::OpenAIResponsesExt::default())
    }
}

pub(crate) fn client(base_url: &str, key: Option<&str>) -> Result<openai::Client, String> {
    let base_url = server_url(base_url)?;
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not prepare the custom connection.".to_string())?;
    ClientBuilder::<CompatibleBuilder>::default()
        .api_key(OptionalKey(key.map(str::to_string)))
        .base_url(&base_url)
        .http_client(http)
        .build()
        .map_err(|_| "Could not prepare the custom connection. Check the API key.".into())
}

pub(crate) async fn discover(base_url: &str, key: Option<&str>) -> Result<Vec<String>, String> {
    let client = client(base_url, key)?;
    let models = tokio::time::timeout(Duration::from_secs(30), client.list_models())
        .await
        .map_err(|_| "Model discovery timed out. You can enter model IDs manually.".to_string())?
        .map_err(|_| {
            "Could not discover models. Check the URL and key, or enter model IDs manually."
                .to_string()
        })?;
    model_ids(models.data.into_iter().map(|model| model.id).collect())
}
