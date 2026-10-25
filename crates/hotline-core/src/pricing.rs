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
    let model = &*crate::models::base_id(model);
    metadata
        .get(model)
        .and_then(|entry| entry.cost.clone())
        .or_else(|| {
            let (provider, id) = model.split_once('/')?;
            let entry = crate::models::model_of(provider, id)?;
            let cost = entry.cost.as_ref()?;
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

/// One response's tokens as Hotline counts them, whatever route reported
/// them: `input` holds every token the request carried, what was read from
/// the provider's cache and written to it included. The budgets, the context
/// threshold and a turn's record all read these, and [`Tokens::of`] is the
/// one place Rig's usage is read. Rig reports every provider that way, so
/// nothing is added here: counting Anthropic's cache beside its input again
/// would price each cached token twice and fill the context early.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tokens {
    pub(crate) input: u64,
    pub(crate) output: u64,
    pub(crate) total: u64,
    pub(crate) cache_read: u64,
    pub(crate) cache_write: u64,
    /// Whether the provider reported usage at all. Unreported usage is not a
    /// free response: the round keeps whatever it reserved.
    pub(crate) reported: bool,
}

impl Tokens {
    /// What `usage` counts. A counter the provider did not report is none,
    /// and a response it counted at nothing at all reported nothing: no
    /// reply is free, so it keeps what was reserved for it.
    pub(crate) fn of(usage: &Usage) -> Self {
        let mut tokens = Self {
            input: usage.input_tokens.unwrap_or_default(),
            output: usage.output_tokens.unwrap_or_default(),
            total: usage.total_tokens.unwrap_or_default(),
            cache_read: usage.cached_input_tokens.unwrap_or_default(),
            cache_write: usage.cache_creation_input_tokens.unwrap_or_default(),
            reported: false,
        };
        tokens.reported = tokens != Self::default();
        tokens
    }

    /// The tokens the conversation filled: what went in and what came out.
    pub(crate) fn context(&self) -> u64 {
        self.input.saturating_add(self.output)
    }

    /// A turn's record of these tokens, with the cache counts left out when
    /// nothing was cached.
    pub(crate) fn token_usage(&self) -> crate::contract::TokenUsage {
        crate::contract::TokenUsage {
            input_tokens: Some(self.input as i64),
            output_tokens: Some(self.output as i64),
            total_tokens: Some(self.total as i64),
            cache_read_tokens: Some(self.cache_read as i64).filter(|n| *n > 0),
            cache_write_tokens: Some(self.cache_write as i64).filter(|n| *n > 0),
        }
    }
}

impl std::ops::AddAssign for Tokens {
    fn add_assign(&mut self, other: Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.total = self.total.saturating_add(other.total);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
        self.reported = self.reported || other.reported;
    }
}

/// What a response cost at `price`, in dollars. Cache reads and writes are
/// inside `input`, and priced once, at their own rates. Tokens the total
/// holds beyond input and output (a Gemini model's thinking) are priced as
/// output. A cache price the catalogue lacks is the input price for a read
/// and Anthropic's 1.25 times it for a write.
pub(crate) fn cost(price: &ModelCost, tokens: &Tokens) -> f64 {
    let read = tokens.cache_read;
    let written = tokens.cache_write;
    let cached = read.saturating_add(written);
    let fresh = tokens.input.saturating_sub(cached);
    let input = tokens.input.max(cached);
    let unreported = tokens
        .total
        .saturating_sub(input.saturating_add(tokens.output));
    let output = tokens.output.saturating_add(unreported);
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
