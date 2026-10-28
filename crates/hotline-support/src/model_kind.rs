//! What a listed model is for, in the words a provider's own listing uses.
//!
//! Every model a provider lists gets exactly one kind, and each picker offers
//! only its own: the chat picker chats, the speaking picker speaks. The kind
//! is decided by `hotline_core::models::kind`, which asks [`Said::kind`]
//! first; this half lives here because the voice crate reads an Ollama
//! server's speech models from the same capabilities, and the two must not
//! disagree about which models those are.

use serde::{Deserialize, Serialize};

/// The one thing a model is for. The names are models.dev's `type` values,
/// so the snapshot stores them as the catalogue spells them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Chat,
    Image,
    Speech,
    Transcription,
    Video,
    Embedding,
    Realtime,
    Reranking,
}

impl ModelKind {
    /// A `type` field: models.dev's, or an OpenAI-shaped listing's. A word
    /// that names no kind (Mistral's `base`, Copilot's `completion`) decides
    /// nothing.
    pub fn from_type(word: &str) -> Option<Self> {
        Some(match word {
            "chat" => Self::Chat,
            "image" => Self::Image,
            "speech" | "tts" => Self::Speech,
            "transcription" => Self::Transcription,
            "video" => Self::Video,
            "embedding" | "embeddings" => Self::Embedding,
            "realtime" => Self::Realtime,
            "rerank" | "reranking" => Self::Reranking,
            _ => return None,
        })
    }

    /// What a model's outputs say it is. Text alone is chat. Without text,
    /// the first output that names a kind decides. Text beside a picture or
    /// sound decides nothing: OpenRouter's own router answers in text and
    /// pictures, as an image model does, so the snapshot has to tell them
    /// apart.
    pub fn from_outputs(outputs: &[String]) -> Option<Self> {
        if outputs.iter().any(|output| output == "text") {
            return (outputs.len() == 1).then_some(Self::Chat);
        }
        outputs.iter().find_map(|output| match output.as_str() {
            "audio" => Some(Self::Speech),
            other => Self::from_type(other),
        })
    }
}

/// What a provider's listing said about one model, as it said it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Said {
    /// The listing says nothing about what the model does.
    Nothing,
    /// Ollama's `/api/tags` `capabilities`: `completion`, `tools`,
    /// `vision`, `thinking`, `speech`, `embedding`, `image`.
    OllamaCapabilities(Vec<String>),
    /// OpenRouter's `architecture.output_modalities`, or xAI's
    /// `output_modalities`.
    OutputModalities(Vec<String>),
    /// An OpenAI-shaped listing's `type`.
    Type(String),
}

impl Said {
    /// The kind the listing itself decides, if it decides one. An Ollama
    /// model that speaks is a speech model even if it also completes, so
    /// the speaking picker and the chat picker never share one.
    pub fn kind(&self) -> Option<ModelKind> {
        match self {
            Self::Nothing => None,
            Self::OllamaCapabilities(capabilities) => {
                let has = |word: &str| capabilities.iter().any(|one| one == word);
                if has("speech") {
                    Some(ModelKind::Speech)
                } else if has("embedding") {
                    Some(ModelKind::Embedding)
                } else if has("image") {
                    Some(ModelKind::Image)
                } else if has("completion") {
                    Some(ModelKind::Chat)
                } else {
                    None
                }
            }
            Self::OutputModalities(outputs) => ModelKind::from_outputs(outputs),
            Self::Type(word) => ModelKind::from_type(word),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn outputs_decide_only_when_they_are_unambiguous() {
        assert_eq!(
            ModelKind::from_outputs(&words(&["text"])),
            Some(ModelKind::Chat)
        );
        assert_eq!(
            ModelKind::from_outputs(&words(&["image"])),
            Some(ModelKind::Image)
        );
        assert_eq!(
            ModelKind::from_outputs(&words(&["audio"])),
            Some(ModelKind::Speech)
        );
        assert_eq!(
            ModelKind::from_outputs(&words(&["embeddings"])),
            Some(ModelKind::Embedding)
        );
        assert_eq!(ModelKind::from_outputs(&words(&["image", "text"])), None);
        assert_eq!(ModelKind::from_outputs(&words(&["decisions"])), None);
        assert_eq!(ModelKind::from_outputs(&[]), None);
    }

    #[test]
    fn a_kind_is_stored_as_models_dev_spells_it() {
        for kind in [
            ModelKind::Chat,
            ModelKind::Image,
            ModelKind::Speech,
            ModelKind::Transcription,
            ModelKind::Video,
            ModelKind::Embedding,
            ModelKind::Realtime,
            ModelKind::Reranking,
        ] {
            let word = serde_json::to_value(kind).unwrap();
            assert_eq!(ModelKind::from_type(word.as_str().unwrap()), Some(kind));
        }
        assert_eq!(ModelKind::from_type("fine-tuned"), None);
    }
}
