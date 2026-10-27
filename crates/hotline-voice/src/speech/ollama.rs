//! An Ollama server the owner connected, as a voice. Ollama hears nothing,
//! but a speech model pulled into it (`phonon`) speaks through the OpenAI
//! shape at `{server}/v1/audio/speech`, keyless, so the adapter is
//! [`super::OpenAiShape`] and this file only says where and what.
//!
//! What it can say is a fact of the moment, not of the connection: a model
//! is pulled or removed, the server is quit. So the speech models are asked
//! of the server itself, `GET /api/tags` for the models whose capabilities
//! say `speech` and `POST /api/show` for each one's voices, and nothing is
//! assumed when it cannot be asked.

use super::catalog::{Spoken, read_json};
use futures_util::future::join_all;
use hotline_support::model_kind::{ModelKind, Said};
use serde_json::{Value, json};

pub const PROVIDER_ID: &str = "ollama";
pub const NAME: &str = "Ollama";
/// The voice every Ollama speech model has, which it speaks in when asked
/// for none in particular.
pub const DEFAULT_VOICE: &str = "default";

/// Where the OpenAI-shaped `/audio` routes hang from on this server.
pub fn speech_base(server: &str) -> String {
    format!("{}/v1", server.trim_end_matches('/'))
}

/// The server's speech models and their voices, or `None` when the server
/// cannot be asked. A model whose voices cannot be read is left out rather
/// than offered to fail.
pub async fn speech_models(client: &reqwest::Client, server: &str) -> Option<Vec<Spoken>> {
    let server = server.trim_end_matches('/');
    let tags = read_json(client.get(format!("{server}/api/tags"))).await?;
    let names = speakers(&tags);
    let shown = join_all(names.iter().map(|name| {
        read_json(
            client
                .post(format!("{server}/api/show"))
                .json(&json!({ "model": name })),
        )
    }))
    .await;
    Some(
        names
            .into_iter()
            .zip(shown)
            .filter_map(|(name, show)| {
                let voices = voices(&show?);
                (!voices.is_empty()).then(|| Spoken {
                    label: Some(label(&name)),
                    id: name,
                    voices,
                })
            })
            .collect(),
    )
}

/// The installed models whose capabilities make them speech models, by the
/// same reading that keeps them out of the chat picker.
fn speakers(tags: &Value) -> Vec<String> {
    tags["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|model| {
            let capabilities = model["capabilities"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|one| one.as_str().map(str::to_string))
                .collect();
            Said::OllamaCapabilities(capabilities).kind() == Some(ModelKind::Speech)
        })
        .filter_map(|model| model["name"].as_str().map(str::to_string))
        .collect()
}

/// A model's voices as Ollama lists them, `default` moved first, because the
/// picker's first voice is the one a model speaks in when none is named.
fn voices(show: &Value) -> Vec<String> {
    let mut voices: Vec<String> = show["voices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|voice| voice.as_str().map(str::to_string))
        .collect();
    if let Some(at) = voices.iter().position(|voice| voice == DEFAULT_VOICE) {
        let default = voices.remove(at);
        voices.insert(0, default);
    }
    voices
}

/// `phonon:latest` reads as `phonon`; any other tag is part of the name.
fn label(name: &str) -> String {
    name.strip_suffix(":latest").unwrap_or(name).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_models_that_say_speech_speak_and_default_is_their_first_voice() {
        let tags = json!({ "models": [
            { "name": "phonon:latest", "capabilities": ["speech"] },
            { "name": "llama3.1:8b", "capabilities": ["completion", "tools"] },
            { "name": "old-model" },
        ]});
        assert_eq!(speakers(&tags), ["phonon:latest"]);
        let show = json!({ "capabilities": ["speech"], "voices": ["Alfie", "default", "Zoey"] });
        assert_eq!(voices(&show), ["default", "Alfie", "Zoey"]);
        assert_eq!(voices(&json!({})), [] as [String; 0]);
        assert_eq!(label("phonon:latest"), "phonon");
        assert_eq!(label("phonon:40m-fr"), "phonon:40m-fr");
        assert_eq!(
            speech_base("http://localhost:11434/"),
            "http://localhost:11434/v1"
        );
    }
}
