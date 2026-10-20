//! OpenAI-compatible connections use Rig's native request and streaming code,
//! over a transport that repairs loose Responses frames ([`super::compat`]).

use crate::contract::CustomProviderDraft;
use rig::providers::openai::wire::{Auth, Dialect};
use rig::providers::openai::{OpenAI, OpenAIConfig, Route};
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
    let ids: Vec<_> = ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    super::discovery::validate_ids(&ids)
}

/// A server Hotline knows only by its address. It speaks OpenAI's wire, and
/// its own name keeps Rig's catalogue of OpenAI's models from describing a
/// model that only shares an id with one of them. A keyless server receives
/// no fabricated authorization header.
const COMPATIBLE: Dialect = {
    let mut dialect = Dialect::gateway("custom", "", "");
    dialect.quirks.auth = Auth::OptionalBearer;
    dialect.quirks.completion_route = Route::Responses;
    dialect
};

fn config(base_url: &str, key: Option<&str>) -> OpenAIConfig {
    OpenAIConfig::with_key(&COMPATIBLE, key.unwrap_or_default()).with_base_url(base_url)
}

/// A client on `route` for the server at `base_url`.
pub(crate) fn client(base_url: &str, key: Option<&str>, route: Route) -> Result<OpenAI, String> {
    let base_url = server_url(base_url)?;
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not prepare the custom connection.".to_string())?;
    Ok(config(&base_url, key)
        .with_route(route)
        .connect(super::compat::Http::new(http)))
}

pub(crate) async fn discover(base_url: &str, key: Option<&str>) -> Result<Vec<String>, String> {
    let base_url = server_url(base_url)?;
    let client = config(&base_url, key).connect(super::discovery::DiscoveryHttp::new()?);
    Ok(super::discovery::collect(client.list_models())
        .await?
        .into_iter()
        .map(|model| model.id)
        .collect())
}
