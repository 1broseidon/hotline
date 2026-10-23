//! How Hotline Agent reaches a provider, as the vault resolves it at the start
//! of a turn and the voice engines and image makers read it.

use std::path::PathBuf;

/// How Hotline Agent reaches a provider: a key, a login directory, or a
/// configured endpoint with its own protocol and optional key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderAuth {
    ApiKey(String),
    Unavailable(String),
    StoredLogin {
        tokens: hotline_support::credentials::CredentialFile,
    },
    Login {
        token_dir: PathBuf,
    },
    Local {
        base_url: String,
    },
    Custom {
        name: String,
        base_url: String,
        api_key: Option<String>,
        config: hotline_contract::CustomProvider,
    },
}

/// The credential a teammate is set to. It stands in for its own provider
/// alone, so a turn on another provider's model never reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChosenAccount {
    pub id: String,
    pub provider_id: String,
}

/// Whether a provider id names a connection the owner configured themselves:
/// `custom-` and a uuid, never a provider Hotline knows.
pub fn is_custom(provider_id: &str) -> bool {
    provider_id
        .strip_prefix("custom-")
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
}
