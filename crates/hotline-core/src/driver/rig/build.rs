//! Building the Rig model for one turn, and the agent for one answer: the
//! model on its provider's client, the reasoning effort each client takes,
//! and the request settings a provider needs.

use super::text;
use crate::contract::SessionConfig;
use crate::models::{self, Client};
use crate::session::ProviderAuth;
use rig::agent::{AgentBuilder, ModelHandle};
use rig::prelude::*;
use rig::providers::{
    anthropic, chatgpt, deepseek, gemini, groq, mistral, openai, openrouter, xai, zai,
};
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
            // Rig's AdditionalParameters is camelCase; ThinkingLevel is
            // snake_case, so the catalogue id is the value. Uppercasing it
            // would fail the deserialize that puts additional_params on the
            // request.
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
/// the preamble are marked, and the top-level breakpoint moves forward with
/// the conversation, so each round reads what the round before it wrote.
pub(super) fn anthropic_model(
    client: &anthropic::Client,
    model: &str,
    reuse: Reuse,
) -> anthropic::completion::CompletionModel {
    let model = client.completion_model(model);
    match reuse {
        Reuse::Rounds => model.with_prompt_caching().with_automatic_caching(),
        Reuse::Once => model,
    }
}

/// OpenRouter hands a cache marker on the preamble through to Claude, which
/// caches nothing without one. The other models it routes cache on their own
/// or not at all, so they are left as they were.
pub(super) fn openrouter_model(
    client: &openrouter::Client,
    model: &str,
    reuse: Reuse,
) -> openrouter::CompletionModel {
    let completion = client.completion_model(model);
    if reuse == Reuse::Rounds && model.starts_with("anthropic/") {
        completion.with_prompt_caching()
    } else {
        completion
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
    let builder = AgentBuilder::from_model_handle(completion_model(keys, model_id, reuse).await?);
    let (ceiling, params) = request_settings(keys, model_id, effort, output_limit)?;
    let builder = match ceiling {
        Some(ceiling) => builder.max_tokens(ceiling),
        None => builder,
    };
    Ok(match params {
        Some(params) => builder.additional_params(params),
        None => builder,
    })
}

/// One model on the provider whose credential the desk holds. A turn sends
/// its own requests to it; an agent wraps it for a single answer.
pub(super) async fn completion_model(
    keys: &HashMap<String, ProviderAuth>,
    model_id: &str,
    reuse: Reuse,
) -> Result<ModelHandle, String> {
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
            let client = crate::providers::custom::client(base_url, api_key.as_deref())?;
            match config.api {
                crate::contract::OpenAiApi::Responses => {
                    ModelHandle::new(client.completion_model(model))
                }
                crate::contract::OpenAiApi::ChatCompletions => {
                    ModelHandle::new(client.completions_api().completion_model(model))
                }
            }
        }
        (Client::CustomOpenAi, _) | (_, ProviderAuth::Custom { .. }) => {
            return Err("This model needs its saved custom connection.".into());
        }
        (Client::ChatGpt, ProviderAuth::Login { token_dir }) => ModelHandle::new(
            chatgpt::Client::builder()
                .oauth()
                .auth_file(token_dir.join("auth.json"))
                .allow_device_flow(false)
                .build()
                .map_err(text)?
                .completion_model(model),
        ),
        (Client::Copilot, ProviderAuth::Login { token_dir }) => {
            crate::providers::copilot::ensure_endpoints(token_dir).await;
            if crate::providers::copilot::wants_responses(token_dir, model) {
                crate::providers::copilot::responses_model(token_dir, model).await?
            } else {
                crate::providers::copilot::chat_model(token_dir, model)?
            }
        }
        (Client::OpenRouter, ProviderAuth::StoredLogin { tokens }) => {
            ModelHandle::new(openrouter_model(
                &openrouter::Client::new(&crate::providers::openrouter_key(tokens)?)
                    .map_err(text)?,
                model,
                reuse,
            ))
        }
        (Client::XAi, ProviderAuth::StoredLogin { tokens }) => {
            ModelHandle::new(crate::providers::xai::client(tokens)?.completion_model(model))
        }
        (Client::Ollama, ProviderAuth::Local { base_url }) => {
            ModelHandle::new(crate::providers::ollama_client(base_url, "")?.completion_model(model))
        }
        (Client::OllamaCloud, ProviderAuth::ApiKey(key)) => ModelHandle::new(
            crate::providers::ollama_client(crate::providers::OLLAMA_CLOUD_URL, key)?
                .completion_model(model),
        ),
        (_, ProviderAuth::Local { .. }) | (Client::Ollama, ProviderAuth::ApiKey(_)) => {
            return Err(format!("{provider} has an incompatible connection method."));
        }
        (Client::ChatGpt | Client::Copilot, ProviderAuth::ApiKey(_)) => {
            return Err(format!("{provider} needs a sign-in, not a key."));
        }
        (_, ProviderAuth::Login { .. } | ProviderAuth::StoredLogin { .. }) => {
            return Err(format!("{provider} needs a key, not a sign-in."));
        }
        (Client::Anthropic, ProviderAuth::ApiKey(key)) => ModelHandle::new(anthropic_model(
            &anthropic::Client::new(key).map_err(text)?,
            model,
            reuse,
        )),
        (Client::OpenAi, ProviderAuth::ApiKey(key)) => ModelHandle::new(
            openai::Client::new(key)
                .map_err(text)?
                .completion_model(model),
        ),
        (Client::OpenRouter, ProviderAuth::ApiKey(key)) => ModelHandle::new(openrouter_model(
            &openrouter::Client::new(key).map_err(text)?,
            model,
            reuse,
        )),
        (Client::Gemini, ProviderAuth::ApiKey(key)) => ModelHandle::new(
            gemini::Client::new(key)
                .map_err(text)?
                .completion_model(model),
        ),
        (Client::XAi, ProviderAuth::ApiKey(key)) => {
            ModelHandle::new(xai::Client::new(key).map_err(text)?.completion_model(model))
        }
        (Client::Groq, ProviderAuth::ApiKey(key)) => ModelHandle::new(
            groq::Client::new(key)
                .map_err(text)?
                .completion_model(model),
        ),
        (Client::DeepSeek, ProviderAuth::ApiKey(key)) => ModelHandle::new(
            deepseek::Client::new(key)
                .map_err(text)?
                .completion_model(model),
        ),
        (Client::Mistral, ProviderAuth::ApiKey(key)) => ModelHandle::new(
            mistral::Client::new(key)
                .map_err(text)?
                .completion_model(model),
        ),
        (Client::Zai, ProviderAuth::ApiKey(key)) => ModelHandle::new(
            zai::Client::builder()
                .api_key(key)
                .general()
                .build()
                .map_err(text)?
                .completion_model(model),
        ),
        (Client::ZaiCoding, ProviderAuth::ApiKey(key)) => ModelHandle::new(
            zai::Client::builder()
                .api_key(key)
                .coding()
                .build()
                .map_err(text)?
                .completion_model(model),
        ),
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
) -> Result<(Option<u64>, Option<Value>), String> {
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
    Ok((ceiling, params))
}
