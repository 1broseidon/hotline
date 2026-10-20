//! Building the Rig model for one turn, and the agent for one answer: the
//! model on its provider's client, the reasoning effort each client takes,
//! and the request settings a provider needs.

use crate::contract::SessionConfig;
use crate::models::{self, Client};
use crate::session::ProviderAuth;
use bytes::Bytes;
use rig::agent::AgentBuilder;
use rig::completion::{CacheRetention, CompletionRequest, GenerationOptions};
use rig::operation::Completion;
use rig::providers::openai::{OpenAI, OpenAIConfig, Route, wire};
use rig::providers::{anthropic, deepseek, gemini, groq, mistral, xai};
use rig::{DynModel, Model};
use serde_json::Value;
use std::collections::HashMap;

/// The JSON merged into every request for this effort, when this client
/// carries `additional_params` that far. Copilot's chat-completions route
/// is applied in [`request_settings`]: this function returns the Responses
/// body Copilot shares with OpenAI and ChatGPT.
pub(super) fn effort_params(client: Client, effort: &str) -> Option<serde_json::Value> {
    match client {
        Client::CustomOpenAi => None,
        Client::Anthropic => Some(serde_json::json!({
            "thinking": {"type": "adaptive"},
            "output_config": {"effort": effort}
        })),
        Client::OpenAi | Client::ChatGpt | Client::Copilot | Client::OpenRouter | Client::XAi => {
            Some(serde_json::json!({"reasoning": {"effort": effort}}))
        }
        Client::Gemini => match effort {
            // Merged into the request body as written, so the catalogue id
            // is the value Google reads.
            "minimal" | "low" | "medium" | "high" => Some(serde_json::json!({
                "generationConfig": {
                    "thinkingConfig": {"thinkingLevel": effort}
                }
            })),
            _ => None,
        },
        Client::Ollama | Client::OllamaCloud => match effort {
            "low" | "medium" | "high" | "max" => Some(serde_json::json!({"think": effort})),
            _ => None,
        },
        Client::Groq | Client::DeepSeek | Client::Mistral | Client::Zai | Client::ZaiCoding => {
            Some(serde_json::json!({"reasoning_effort": effort}))
        }
    }
}

/// One Effort picker when the model lists any, otherwise none.
pub(super) fn effort_config(model_id: &str, current: Option<&str>) -> Vec<SessionConfig> {
    let options = models::effort_choices(model_id);
    if options.is_empty() {
        return Vec::new();
    }
    vec![SessionConfig {
        id: "effort".to_string(),
        name: "Effort".to_string(),
        category: Some(crate::contract::SessionConfigCategory::Effort),
        current_id: current.map(str::to_string),
        options,
    }]
}

/// How often the requests a builder makes go out. A turn resends the tools,
/// the preamble and the conversation so far on every round, so a provider
/// that caches only when asked is asked. A single answer is asked once, and a
/// cache write costs more than a plain read that nothing would follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reuse {
    Rounds,
    Once,
}

/// Claude over the API caches nothing unless the request asks. The tools and
/// the system prompt are marked here, and the request's cache
/// ([`RequestSettings`]) moves the top-level breakpoint forward with the
/// conversation, so each round reads what the round before it wrote.
pub(super) fn anthropic_model(
    client: &anthropic::Anthropic,
    model: &str,
    reuse: Reuse,
) -> Model<anthropic::Messages> {
    let mut model = client.completion(model);
    if reuse == Reuse::Rounds {
        model.wire = model.wire.with_prompt_caching();
    }
    model
}

/// Claude on OpenRouter. OpenRouter hands a cache marker on the system prompt
/// through to Claude, which caches nothing without one; the request's cache
/// ([`RequestSettings`]) is a marker on the request as a whole, which is all
/// Rig sends, so a turn's prompt is marked on its way out as well. The other
/// models OpenRouter routes cache on their own or not at all, and are left
/// as they were.
pub(super) fn openrouter_model(
    config: OpenAIConfig,
    model: &str,
    reuse: Reuse,
) -> Result<DynModel<Completion>, String> {
    if reuse == Reuse::Rounds && caches_when_asked(Client::OpenRouter, model) {
        let transport = crate::providers::outgoing::Outgoing::mended(mark_system_prompt)?;
        Ok(config.connect(transport).completion(model).erase())
    } else {
        Ok(config.client().completion(model).erase())
    }
}

/// A chat request's leading system prompt with an ephemeral cache marker on
/// its last text part. Anything that is not a chat request with a system
/// prompt first passes as it came.
fn mark_system_prompt(body: Bytes) -> Bytes {
    let marker = || serde_json::json!({"type": "ephemeral"});
    let Ok(mut request) = serde_json::from_slice::<Value>(&body) else {
        return body;
    };
    let Some(system) = request
        .get_mut("messages")
        .and_then(Value::as_array_mut)
        .and_then(|messages| messages.first_mut())
        .filter(|message| message["role"] == "system")
    else {
        return body;
    };
    match &mut system["content"] {
        Value::String(text) => {
            let text = std::mem::take(text);
            system["content"] =
                serde_json::json!([{"type": "text", "text": text, "cache_control": marker()}]);
        }
        Value::Array(parts) => match parts.last_mut().and_then(Value::as_object_mut) {
            Some(part) if part.get("type").and_then(Value::as_str) == Some("text") => {
                part.insert("cache_control".into(), marker());
            }
            _ => return body,
        },
        _ => return body,
    }
    serde_json::to_vec(&request).map_or(body, Bytes::from)
}

/// Whether requests for `model` on `client` ask the provider to cache:
/// Claude, over the API or through OpenRouter, caches nothing without a
/// marker. Every other model caches on its own or not at all, and is sent
/// none.
fn caches_when_asked(client: Client, model: &str) -> bool {
    match client {
        Client::Anthropic => true,
        Client::OpenRouter => model.starts_with("anthropic/"),
        _ => false,
    }
}

/// What every request to one model carries besides its conversation and its
/// tools: the output ceiling, the effort, and the cache it asks for. A turn
/// writes its own requests and an agent writes them for an answer; both take
/// these from here, so neither can send what the other does not.
///
/// Every request also accepts a finish reason Rig does not know as an
/// ordinary stop. A gateway that passes its upstream's own reason through
/// would otherwise fail a spoken answer that arrived whole, and leave a
/// turn's reply out of every later request (docs/design.md).
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct RequestSettings {
    pub(super) max_tokens: Option<u64>,
    pub(super) additional_params: Option<Value>,
    pub(super) cache: Option<CacheRetention>,
}

impl RequestSettings {
    pub(super) fn on_request(&self, request: CompletionRequest) -> CompletionRequest {
        let request = request
            .max_tokens(self.max_tokens)
            .additional_params(self.additional_params.clone())
            .accept_unknown_finish_reasons(true);
        match self.cache {
            Some(cache) => request.cache(cache),
            None => request,
        }
    }

    pub(super) fn on_agent(&self, builder: AgentBuilder) -> AgentBuilder {
        let builder = builder.accept_unknown_finish_reasons(true);
        let builder = match self.max_tokens {
            Some(ceiling) => builder.max_tokens(ceiling),
            None => builder,
        };
        let builder = match &self.additional_params {
            Some(params) => builder.additional_params(params.clone()),
            None => builder,
        };
        match self.cache {
            Some(cache) => builder.options(GenerationOptions::new().cache(cache)),
            None => builder,
        }
    }
}

/// An agent for one model on the provider whose credential the desk holds,
/// with the output ceiling and effort its requests carry.
pub(super) async fn agent_builder(
    keys: &HashMap<String, ProviderAuth>,
    model_id: &str,
    effort: Option<&str>,
    output_limit: Option<u64>,
    reuse: Reuse,
) -> Result<AgentBuilder, String> {
    let builder = AgentBuilder::new(completion_model(keys, model_id, reuse).await?);
    let settings = request_settings(keys, model_id, effort, output_limit, reuse)?;
    Ok(settings.on_agent(builder))
}

/// An OpenAI-shaped vendor's client for `key`.
fn vendor(dialect: &wire::Dialect, key: &str) -> OpenAI {
    OpenAIConfig::with_key(dialect, key).client()
}

/// One model on the provider whose credential the desk holds. A turn sends
/// its own requests to it; an agent wraps it for a single answer.
pub(super) async fn completion_model(
    keys: &HashMap<String, ProviderAuth>,
    model_id: &str,
    reuse: Reuse,
) -> Result<DynModel<Completion>, String> {
    let (provider, model) = model_id
        .split_once('/')
        .ok_or_else(|| format!("{model_id} is not a provider/model id"))?;
    let held = keys
        .get(provider)
        .ok_or_else(|| format!("No connection for {provider} is available on this desk."))?;
    let wiring = models::wiring(provider)
        .ok_or_else(|| format!("{provider} is not a provider Hotline Agent can use"))?;
    let model = match (wiring.client, held) {
        (_, ProviderAuth::Unavailable(error)) => return Err(error.clone()),
        (
            Client::CustomOpenAi,
            ProviderAuth::Custom {
                base_url,
                api_key,
                config,
                ..
            },
        ) => {
            let route = match config.api {
                crate::contract::OpenAiApi::Responses => Route::Responses,
                crate::contract::OpenAiApi::ChatCompletions => Route::Chat,
            };
            crate::providers::custom::client(base_url, api_key.as_deref(), route)?
                .completion(model)
                .erase()
        }
        (Client::CustomOpenAi, _) | (_, ProviderAuth::Custom { .. }) => {
            return Err("This model needs its saved custom connection.".into());
        }
        (Client::ChatGpt, ProviderAuth::Login { token_dir }) => {
            crate::providers::chatgpt::model(token_dir, model)?
        }
        (Client::Copilot, ProviderAuth::Login { token_dir }) => {
            crate::providers::copilot::ensure_endpoints(token_dir).await;
            let responses = crate::providers::copilot::wants_responses(token_dir, model);
            crate::providers::copilot::model(token_dir, model, responses).await?
        }
        (Client::OpenRouter, ProviderAuth::StoredLogin { tokens }) => openrouter_model(
            OpenAIConfig::with_key(&wire::OPENROUTER, crate::providers::openrouter_key(tokens)?),
            model,
            reuse,
        )?,
        (Client::XAi, ProviderAuth::StoredLogin { tokens }) => {
            crate::providers::xai::client(tokens)?
                .completion(model)
                .erase()
        }
        (Client::Ollama, ProviderAuth::Local { base_url }) => {
            crate::providers::ollama_client(base_url, "")?
                .native_completion(model)
                .erase()
        }
        (Client::OllamaCloud, ProviderAuth::ApiKey(key)) => {
            crate::providers::ollama_client(crate::providers::OLLAMA_CLOUD_URL, key)?
                .native_completion(model)
                .erase()
        }
        (_, ProviderAuth::Local { .. }) | (Client::Ollama, ProviderAuth::ApiKey(_)) => {
            return Err(format!("{provider} has an incompatible connection method."));
        }
        (Client::ChatGpt | Client::Copilot, ProviderAuth::ApiKey(_)) => {
            return Err(format!("{provider} needs a sign-in, not a key."));
        }
        (_, ProviderAuth::Login { .. } | ProviderAuth::StoredLogin { .. }) => {
            return Err(format!("{provider} needs a key, not a sign-in."));
        }
        (Client::Anthropic, ProviderAuth::ApiKey(key)) => {
            let config = anthropic::AnthropicConfig::new(key.as_str());
            #[cfg(feature = "test-support")]
            let config = match crate::provider_fixture::origin(key) {
                Some(origin) => config.with_base_url(origin),
                None => config,
            };
            anthropic_model(&config.client(), model, reuse).erase()
        }
        (Client::OpenAi, ProviderAuth::ApiKey(key)) => {
            OpenAI::new(key.as_str()).completion(model).erase()
        }
        (Client::OpenRouter, ProviderAuth::ApiKey(key)) => {
            openrouter_model(OpenAIConfig::with_key(&wire::OPENROUTER, key), model, reuse)?
        }
        (Client::Gemini, ProviderAuth::ApiKey(key)) => {
            gemini::Gemini::new(key.as_str()).completion(model).erase()
        }
        (Client::XAi, ProviderAuth::ApiKey(key)) => {
            xai::new(key.as_str()).completion(model).erase()
        }
        (Client::Groq, ProviderAuth::ApiKey(key)) => {
            groq::new(key.as_str()).completion(model).erase()
        }
        (Client::DeepSeek, ProviderAuth::ApiKey(key)) => {
            deepseek::new(key.as_str()).completion(model).erase()
        }
        (Client::Mistral, ProviderAuth::ApiKey(key)) => {
            mistral::new(key.as_str()).completion(model).erase()
        }
        (Client::Zai, ProviderAuth::ApiKey(key)) => {
            vendor(&wire::ZAI, key).completion(model).erase()
        }
        (Client::ZaiCoding, ProviderAuth::ApiKey(key)) => {
            vendor(&wire::ZAI_CODING, key).completion(model).erase()
        }
    };
    Ok(model)
}

/// Whether a Copilot model goes over `/responses` from here: the account's
/// model list offered it nowhere else. Rig's own route takes the rest.
pub(super) fn copilot_responses(
    keys: &HashMap<String, ProviderAuth>,
    provider: &str,
    model: &str,
) -> bool {
    models::wiring(provider).is_some_and(|wiring| wiring.client == Client::Copilot)
        && matches!(
            keys.get(provider),
            Some(ProviderAuth::Login { token_dir })
                if crate::providers::copilot::wants_responses(token_dir, model)
        )
}

pub(super) fn request_settings(
    keys: &HashMap<String, ProviderAuth>,
    model_id: &str,
    effort: Option<&str>,
    output_limit: Option<u64>,
    reuse: Reuse,
) -> Result<RequestSettings, String> {
    let (provider, model) = model_id.split_once('/').ok_or("Model has no provider")?;
    let wiring = models::wiring(provider).ok_or("Unknown provider")?;
    // Anthropic requires a ceiling even for a model absent from discovery.
    let ceiling = output_limit
        .or_else(|| models::output_limit(model_id))
        .or_else(|| (wiring.client == Client::Anthropic).then_some(4096));
    let params = effort.and_then(|effort| {
        let chat_route = wiring.client == Client::Copilot
            && !model.to_ascii_lowercase().contains("codex")
            && !copilot_responses(keys, provider, model);
        if chat_route {
            Some(serde_json::json!({"reasoning_effort": effort}))
        } else {
            effort_params(wiring.client, effort)
        }
    });
    let cache = (reuse == Reuse::Rounds && caches_when_asked(wiring.client, model))
        .then_some(CacheRetention::Short);
    Ok(RequestSettings {
        max_tokens: ceiling,
        additional_params: params,
        cache,
    })
}
