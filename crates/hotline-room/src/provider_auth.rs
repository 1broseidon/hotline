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
