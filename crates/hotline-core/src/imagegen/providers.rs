//! Which of the owner's connected providers can draw, and with what by
//! default. Filled in with the adapters; the signature is the contract.

use super::{ImageSet, ImageSettings};
use crate::vault::Vault;

/// The first connected provider that can draw, or the owner's choice. A
/// choice naming a provider that isn't connected, or can't draw, is an
/// error rather than a quiet switch: the words would go somewhere the owner
/// didn't pick. When nothing can draw, the error is a sentence for a person.
pub fn resolve(_vault: &Vault, _settings: &ImageSettings) -> Result<ImageSet, String> {
    Err("Connect OpenRouter, OpenAI or Google to make images.".into())
}
