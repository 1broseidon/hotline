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

mod chatgpt;
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
pub use providers::{describe, model, options, resolve};

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
    SignInRequired,
    RefreshTimedOut,
    Revoked,
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
            ImageError::SignInRequired => write!(
                f,
                "The ChatGPT sign-in could not be refreshed. Sign in again under Settings → Providers."
            ),
            ImageError::RefreshTimedOut => {
                write!(f, "The ChatGPT sign-in refresh timed out. Try again.")
            }
            ImageError::Revoked => write!(f, "This teammate's capabilities have been revoked."),
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
            } if provider_id == chatgpt::PROVIDER_ID && *status == 401 => {
                write!(
                    f,
                    "ChatGPT refused the sign-in (HTTP 401). Reconnect it under Settings → Providers."
                )
            }
            ImageError::Refused {
                provider_id,
                status,
            } if provider_id == chatgpt::PROVIDER_ID && *status == 403 => {
                write!(
                    f,
                    "ChatGPT refused image access (HTTP 403). This account may not have access to Codex images."
                )
            }
            ImageError::Refused {
                provider_id,
                status,
            } if provider_id == chatgpt::PROVIDER_ID && *status == 429 => {
                write!(
                    f,
                    "ChatGPT's image usage or rate limit was reached (HTTP 429). Try again later."
                )
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
    fn subscription(&self) -> bool {
        false
    }
    /// Whether this model can leave the background transparent.
    fn transparent(&self) -> bool;
    /// How many reference images it takes; zero for none.
    fn max_references(&self) -> usize;
    /// What one picture costs at most, rounded up: reserved against the
    /// spend ledger before the request, and charged when the provider does
    /// not report a cost.
    fn estimate_usd(&self, request: &ImageRequest) -> f64;
    async fn generate(&self, request: &ImageRequest) -> Result<Image, ImageError>;

    /// Recheck the caller's authority immediately before dispatch. Adapters
    /// that await authentication override this and check after that wait.
    async fn generate_checked(
        &self,
        request: &ImageRequest,
        before_send: &(dyn Fn() -> Result<(), ImageError> + Send + Sync),
    ) -> Result<Image, ImageError> {
        before_send()?;
        self.generate(request).await
    }
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

/// The crew every avatar is drawn from: one plain character in the house
/// finish, with a head shaped like the Hotline mark, sent as the first
/// reference so the cast stays one set however many teammates there are.
const CREW: &[u8] = include_bytes!("house-avatar.jpg");

const AVATAR: &str = "Profile avatar for an AI teammate in a chat app, drawn as one of the Hotline crew. \
The first reference image is the crew's base character. Keep EXACTLY its build and finish, but not its \
pose: the head \
shaped like the Hotline toad mark, with two big rounded eye domes side by side and short, dark, \
horizontal slot pupils; the soft felt-clay texture; the stubby arms and round feet; and the dark \
charcoal crew jacket with one small signal-green #6bcb62 pin shaped like the toad mark on the chest. \
Not a new character, never a human. Nothing covers the eye domes: a hat sits behind or between them. \
The pin is the only signal green in the picture.";

const STICKER: &str = "Premium die-cut sticker: a thick clean warm-white outline follows the whole \
silhouette, character and props, with a subtle small drop shadow. Centred, generous scale, the whole \
character visible and the head large in the frame. Must read clearly as a small circle 32 pixels wide. \
No text, no letters.";

/// How a teammate stands. The base faces front and stands still, so every
/// avatar is told to do otherwise, and a second hash of its id picks which
/// way, so a roster reads as a cast rather than a row of the same figure.
const POSES: [&str; 8] = [
    "mid-stride in three-quarter view, one arm up in a wave",
    "leaning on something to one side, arms folded, head tilted",
    "sitting cross-legged, busy with its prop in its lap",
    "caught mid-hop, both feet off the ground, arms flung out",
    "turned three-quarters away, glancing back over its shoulder",
    "leaning in close to the viewer, head tipped, one hand raised",
    "carrying its prop over one shoulder, mid-step, body twisted",
    "crouched low and absorbed in its prop, seen from slightly above",
];

pub fn pose(persona_id: &str) -> &'static str {
    POSES[((hash(persona_id) / 7) % POSES.len() as u64) as usize]
}

/// A teammate's colour, the one its initial sits on (ui/src/ui/Avatar.tsx):
/// the same hash of its id picks one of seven hues, here as felt for the
/// body and a deep shade of it for the background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face {
    pub name: &'static str,
    pub body: &'static str,
    pub background: &'static str,
}

/// The hues 70 + 43n of `oklch(72% 0.10 h)` and `oklch(32% 0.045 h)`.
const FACES: [Face; 7] = [
    Face {
        name: "warm caramel",
        body: "#cd995c",
        background: "#422f18",
    },
    Face {
        name: "olive",
        body: "#a5ab5f",
        background: "#333519",
    },
    Face {
        name: "muted sage green",
        body: "#6db78a",
        background: "#1e3a29",
    },
    Face {
        name: "teal",
        body: "#48b8bc",
        background: "#103a3b",
    },
    Face {
        name: "sky blue",
        body: "#69acde",
        background: "#1d3648",
    },
    Face {
        name: "periwinkle",
        body: "#9e9ce1",
        background: "#303049",
    },
    Face {
        name: "orchid pink",
        body: "#c78ec4",
        background: "#3f2a3e",
    },
];

pub fn face(persona_id: &str) -> Face {
    FACES[(hash(persona_id) % 7) as usize]
}

fn hash(persona_id: &str) -> u64 {
    persona_id
        .encode_utf16()
        .fold(0u64, |hash, unit| (hash * 31 + u64::from(unit)) % 1_000_003)
}

/// A request with a named style laid over it.
#[derive(Debug)]
pub struct Styled {
    pub prompt: String,
    pub aspect: Aspect,
    /// What the style shows the model, before any references of the caller's.
    pub references: Vec<Reference>,
}

/// The teammate's words with a named style laid over them. The subject is
/// always theirs; a style only says how it's drawn. An avatar is square, in
/// the teammate's own colour, and shows the model the crew first.
pub fn styled(
    style: Option<&str>,
    prompt: &str,
    aspect: Aspect,
    persona_id: &str,
) -> Result<Styled, String> {
    let prompt = prompt.trim();
    match style {
        None => Ok(Styled {
            prompt: prompt.to_string(),
            aspect,
            references: Vec::new(),
        }),
        Some("avatar") => {
            let Face {
                name,
                body,
                background,
            } = face(persona_id);
            let pose = pose(persona_id);
            Ok(Styled {
                prompt: format!(
                    "{AVATAR}\n\nColour: this teammate's body is {name} felt ({body}) instead of the base's \
teal, on a flat solid {background} background. But if its name is a thing with a colour of its own (a fruit, \
a flower, a stone, a colour word), the body is that colour instead, its accents follow, and the background \
is a deep dark shade of it.\n\nPose: the base stands still facing front; this one must not. Draw it \
{pose}, expressive through the body and the tilt of the head, the pupils still slots. Give it one or two \
props that suit who it is. Any further reference images are the teammate's own: follow them for its \
colour, look and props.\n\n{STICKER}\n\nWho it is: {prompt}"
                ),
                aspect: Aspect::Square,
                references: vec![Reference {
                    mime: "image/jpeg".into(),
                    bytes: CREW.to_vec(),
                }],
            })
        }
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
        let styled = styled(
            Some("avatar"),
            "  an otter in a hard hat ",
            Aspect::Wide,
            "p1",
        )
        .unwrap();
        assert!(styled.prompt.ends_with("Who it is: an otter in a hard hat"));
        assert_eq!(styled.aspect, Aspect::Square);
        let plain = super::styled(None, "a lighthouse", Aspect::Wide, "p1").unwrap();
        assert_eq!(
            (plain.prompt.as_str(), plain.aspect, plain.references.len()),
            ("a lighthouse", Aspect::Wide, 0)
        );
        assert!(
            super::styled(Some("noir"), "x", Aspect::Square, "p1")
                .unwrap_err()
                .contains("avatar")
        );
    }

    #[test]
    fn an_avatar_shows_the_crew_first_in_the_teammates_own_colour() {
        let styled = styled(
            Some("avatar"),
            "a teammate called Mack",
            Aspect::Square,
            "mack",
        )
        .unwrap();
        assert_eq!(styled.references.len(), 1);
        assert_eq!(styled.references[0].mime, "image/jpeg");
        assert!(image::load_from_memory(&styled.references[0].bytes).is_ok());
        let face = face("mack");
        assert!(styled.prompt.contains(face.body) && styled.prompt.contains(face.background));
        assert!(styled.prompt.contains(pose("mack")));
    }

    #[test]
    fn poses_spread_across_a_roster() {
        let poses: std::collections::HashSet<_> =
            ["mack", "poe", "toad", "frankie", "clementine", "p_01J9ZK"]
                .into_iter()
                .map(pose)
                .collect();
        assert!(poses.len() >= 4, "{poses:?}");
    }

    #[test]
    fn a_face_is_the_hue_the_initial_sits_on() {
        // What Avatar.tsx's faceOf picks for these ids, worked out in Node.
        for (id, index) in [
            ("mack", 1),
            ("p_01J9ZK", 2),
            ("persona-ünïcode", 6),
            ("", 0),
        ] {
            assert_eq!(face(id), FACES[index], "{id}");
        }
        assert_ne!(face("a"), face("b"));
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
