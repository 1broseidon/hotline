//! What a model costs per token, and what a response cost: the price the
//! call assistant and teammates' turns are metered at.
//!
//! A price comes from the connection's discovered metadata, else the bundled
//! catalogue (`models.json`). A sign-in or a local server costs nothing. A
//! model on a key with no listed price is metered at [`UNPRICED`], which the
//! desk logs once per model.

use crate::contract::{CatalogModel, CredentialKind, ModelCost};
use rig::completion::Usage;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock, PoisonError};

/// What a model with no listed price is metered at, per million tokens: an
/// Opus-class rate, so the caps still bound the spend of a model nobody
/// priced. It is a guard and not a price, so a model metered at it runs its
/// budget down faster than the bill does, and the desk logs that
/// once per model. Adding the model to `models.json` with `hotline-models-sync`
/// is the fix.
pub(crate) const UNPRICED: ModelCost = ModelCost {
    input: 5.0,
    output: 25.0,
    cache_read: None,
    cache_write: None,
};

/// The model's price from what its connection discovered, else from the
/// bundled catalogue. `None` when neither lists one.
pub(crate) fn listed_price(
    model: &str,
    metadata: &HashMap<String, CatalogModel>,
) -> Option<ModelCost> {
    metadata
        .get(model)
        .and_then(|entry| entry.cost.clone())
        .or_else(|| {
            let (provider, id) = model.split_once('/')?;
            let cost = crate::models::catalog()
                .providers
                .get(provider)?
                .models
                .get(id)?
                .cost
                .as_ref()?;
            Some(ModelCost {
                input: cost.input,
                output: cost.output,
                cache_read: cost.cache_read,
                cache_write: cost.cache_write,
            })
        })
}

/// What a model pays per token: nothing on a sign-in or a local server, the listed price on an API key, and [`UNPRICED`] on a key whose
/// model has no listed price.
pub(crate) fn billed(
    kind: Option<CredentialKind>,
    model: &str,
    listed: Option<ModelCost>,
) -> ModelCost {
    match kind {
        Some(CredentialKind::Oauth | CredentialKind::Local) => ModelCost {
            input: 0.0,
            output: 0.0,
            cache_read: None,
            cache_write: None,
        },
        _ => listed.unwrap_or_else(|| {
            warn_unpriced(model);
            UNPRICED
        }),
    }
}

/// Says once per model, in the desk's log, that it is metered at [`UNPRICED`].
fn warn_unpriced(model: &str) {
    static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let first = WARNED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(model.to_string());
    if first {
        eprintln!(
            "[pricing] {model} has no listed price, so it is metered at ${} in and ${} out per million tokens; its budget runs down faster than the bill",
            UNPRICED.input, UNPRICED.output
        );
    }
}

/// What a response cost at `price`, in dollars. Anthropic reports cache reads
/// and writes beside `input_tokens`; the OpenAI-style APIs count them inside
/// it. Tokens the total holds beyond input and output (a Gemini model's
/// thinking) are priced as output. A cache price the catalogue lacks is the
/// input price for a read and Anthropic's 1.25 times it for a write.
pub(crate) fn cost(price: &ModelCost, usage: &Usage, cache_beside_input: bool) -> f64 {
    let read = usage.cached_input_tokens;
    let written = usage.cache_creation_input_tokens;
    let cached = read.saturating_add(written);
    let (fresh, input) = if cache_beside_input {
        (
            usage.input_tokens,
            usage.input_tokens.saturating_add(cached),
        )
    } else {
        (
            usage.input_tokens.saturating_sub(cached),
            usage.input_tokens.max(cached),
        )
    };
    let unreported = usage
        .total_tokens
        .saturating_sub(input.saturating_add(usage.output_tokens));
    let output = usage.output_tokens.saturating_add(unreported);
    (fresh as f64 * price.input
        + read as f64 * price.cache_read.unwrap_or(price.input)
        + written as f64 * price.cache_write.unwrap_or(price.input * 1.25)
        + output as f64 * price.output)
        / 1_000_000.0
}

/// The credential a connection amounts to, for [`billed`]: a key pays per
/// token; a sign-in, a local server or a custom server without a key does not.
pub(crate) fn credential_kind(auth: &crate::session::ProviderAuth) -> Option<CredentialKind> {
    use crate::session::ProviderAuth;
    match auth {
        ProviderAuth::ApiKey(_)
        | ProviderAuth::Custom {
            api_key: Some(_), ..
        } => Some(CredentialKind::ApiKey),
        ProviderAuth::StoredLogin { .. } | ProviderAuth::Login { .. } => {
            Some(CredentialKind::Oauth)
        }
        ProviderAuth::Local { .. } | ProviderAuth::Custom { api_key: None, .. } => {
            Some(CredentialKind::Local)
        }
        ProviderAuth::Unavailable(_) => None,
    }
}
