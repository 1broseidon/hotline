//! Making images (BRO-174): `generate(request)` over whichever connected
//! provider the owner has, the way `voice::speech` hears and speaks.
//!
//! There is never a new key. A provider counts only if the owner already
//! connected it, and [`resolve`] picks the first that can draw unless
//! `settings.images` names one. OpenRouter's unified Image API covers most
//! models behind one request shape; OpenAI and Google have their own.
//!
//! This module only makes pictures. Where they go (the teammate's
//! workspace, the conversation), what they may cost (the spend ledger) and
//! who may ask (the `generate_image` tool) live with their callers.

mod google;
mod http;
mod openai;
mod openrouter;
mod providers;
#[cfg(test)]
mod tests;

pub use google::Google;
pub use openai::OpenAi;
pub use openrouter::OpenRouter;
pub use providers::{describe, model, resolve};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

/// The shapes a picture can be asked for in. Each adapter maps these onto
/// what its model accepts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aspect {
    #[default]
    #[serde(rename = "1:1")]
    Square,
    #[serde(rename = "16:9")]
    Wide,
    #[serde(rename = "9:16")]
    Tall,
    #[serde(rename = "4:3")]
    Landscape,
    #[serde(rename = "3:4")]
    Portrait,
}

impl Aspect {
    pub fn ratio(self) -> &'static str {
        match self {
            Aspect::Square => "1:1",
            Aspect::Wide => "16:9",
            Aspect::Tall => "9:16",
            Aspect::Landscape => "4:3",
            Aspect::Portrait => "3:4",
        }
    }
}

/// An image the model is shown alongside the words: the thing to edit, or
/// what to match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// What to draw. `prompt` is final: a style preset has already been laid
/// over it by [`styled`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageRequest {
    pub prompt: String,
    pub aspect: Aspect,
    /// Asked for, not promised: [`Image::transparent`] says what came back.
    pub transparent: bool,
    pub references: Vec<Reference>,
}

/// Which provider and model drew a picture.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageId {
    pub provider_id: String,
    pub model_id: String,
}

impl fmt::Display for ImageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.provider_id, self.model_id)
    }
}

/// One finished picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// `image/png`, `image/webp`, `image/jpeg` or `image/svg+xml`, as the
    /// provider made it: a transparent background survives only if nobody
    /// re-encodes it as a JPEG.
    pub mime: String,
    pub bytes: Vec<u8>,
    pub id: ImageId,
    /// Whether a transparent background was asked for and the model can give one.
    pub transparent: bool,
    /// What the provider says it cost, when it says. Otherwise the caller
    /// charges [`ImageGen::estimate_usd`].
    pub cost_usd: Option<f64>,
    pub millis: u64,
}

/// Why no picture came back. None of these carry a response body: a
/// provider's error text can echo the prompt or a reference image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Nothing was asked for.
    EmptyPrompt,
    /// A reference in a format the adapter cannot send.
    UnsupportedReference(String),
    /// The model takes no references, or not this many.
    TooManyReferences {
        max: usize,
    },
    Unreachable {
        provider_id: String,
    },
    Refused {
        provider_id: String,
        status: u16,
    },
    Malformed {
        provider_id: String,
    },
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::EmptyPrompt => write!(f, "There was nothing to draw."),
            ImageError::UnsupportedReference(mime) => {
                write!(f, "An image of type {mime} can't be used as a reference.")
            }
            ImageError::TooManyReferences { max: 0 } => {
                write!(f, "This image model doesn't take reference images.")
            }
            ImageError::TooManyReferences { max } => {
                write!(f, "This image model takes at most {max} reference images.")
            }
            ImageError::Unreachable { provider_id } => {
                write!(f, "{provider_id} could not be reached.")
            }
            ImageError::Refused {
                provider_id,
                status,
            } => write!(f, "{provider_id} refused the request (HTTP {status})."),
            ImageError::Malformed { provider_id } => {
                write!(
                    f,
                    "{provider_id} answered with something that isn't an image."
                )
            }
        }
    }
}

impl std::error::Error for ImageError {}

#[async_trait]
pub trait ImageGen: Send + Sync {
    fn id(&self) -> ImageId;
    /// Whether this model can leave the background transparent.
    fn transparent(&self) -> bool;
    /// How many reference images it takes; zero for none.
    fn max_references(&self) -> usize;
    /// What one picture costs at most, rounded up: reserved against the
    /// spend ledger before the request, and charged when the provider does
    /// not report a cost.
    fn estimate_usd(&self, request: &ImageRequest) -> f64;
    async fn generate(&self, request: &ImageRequest) -> Result<Image, ImageError>;
}

/// What the desk knows of one image model: whether it can leave the
/// background transparent, how many reference images it takes, the quality
/// to ask for, and what one picture costs at most. A model it doesn't know
/// gets the careful answer to each.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub id: String,
    pub transparent: bool,
    pub max_references: usize,
    pub quality: Option<&'static str>,
    pub price_usd: f64,
}

/// What a reference image adds to a picture's price, at most.
const REFERENCE_USD: f64 = 0.01;

impl Model {
    /// Rounded up: this is reserved before the request, so it's a guard,
    /// not an invoice.
    pub fn estimate_usd(&self, request: &ImageRequest) -> f64 {
        self.price_usd + REFERENCE_USD * request.references.len() as f64
    }

    /// Whether a request is one this model can be sent at all.
    pub fn admit(&self, request: &ImageRequest) -> Result<(), ImageError> {
        if request.prompt.trim().is_empty() {
            return Err(ImageError::EmptyPrompt);
        }
        if request.references.len() > self.max_references {
            return Err(ImageError::TooManyReferences {
                max: self.max_references,
            });
        }
        Ok(())
    }
}

/// The owner's choice, from `settings.images`. Both keys are optional: none
/// means the first connected provider that can draw, on its default model.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ImageSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// What the owner's connected providers give the desk: a model to draw
/// with, and a second to try when it fails.
#[derive(Clone)]
pub struct ImageSet {
    pub primary: Arc<dyn ImageGen>,
    pub fallback: Option<Arc<dyn ImageGen>>,
}

/// The house styles a teammate can ask for by name, so a room's pictures of
/// one kind look like one set without every agent inventing the words.
pub const STYLES: &[&str] = &["avatar"];

const AVATAR: &str = "Profile avatar for an AI teammate in a chat app. One subject, centred, \
filling most of a square frame. Bold simple shapes, flat colour with soft shading, friendly. \
Plain solid background in one soft colour, or transparent. No text, no letters, no border, \
no frame. Must read clearly when shown as a small circle 32 pixels wide.";

/// The teammate's words with a named style laid over them. The subject is
/// always theirs; a style only says how it's drawn. An avatar is square.
pub fn styled(
    style: Option<&str>,
    prompt: &str,
    aspect: Aspect,
) -> Result<(String, Aspect), String> {
    let prompt = prompt.trim();
    match style {
        None => Ok((prompt.to_string(), aspect)),
        Some("avatar") => Ok((format!("{AVATAR}\n\nThe subject: {prompt}"), Aspect::Square)),
        Some(other) => Err(format!(
            "There's no image style called \"{other}\". The styles are: {}.",
            STYLES.join(", ")
        )),
    }
}

#[cfg(test)]
mod style_tests {
    use super::*;

    #[test]
    fn a_style_keeps_the_subject_and_squares_an_avatar() {
        let (prompt, aspect) =
            styled(Some("avatar"), "  an otter in a hard hat ", Aspect::Wide).unwrap();
        assert!(prompt.ends_with("The subject: an otter in a hard hat"));
        assert_eq!(aspect, Aspect::Square);
        assert_eq!(
            styled(None, "a lighthouse", Aspect::Wide).unwrap(),
            ("a lighthouse".into(), Aspect::Wide)
        );
        assert!(
            styled(Some("noir"), "x", Aspect::Square)
                .unwrap_err()
                .contains("avatar")
        );
    }

    #[test]
    fn aspects_read_and_write_as_ratios() {
        assert_eq!(serde_json::to_string(&Aspect::Wide).unwrap(), "\"16:9\"");
        assert_eq!(
            serde_json::from_str::<Aspect>("\"3:4\"").unwrap(),
            Aspect::Portrait
        );
    }
}
