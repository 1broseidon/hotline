//! A teammate runs on the account its model names, and on no other.

use super::*;

/// Two Anthropic accounts: "work" is the provider's default, "personal" the
/// second. A third id names an account that is gone.
struct TwoAccounts;

impl ProviderKeys for TwoAccounts {
    fn provider_auth(&self) -> HashMap<String, ProviderAuth> {
        HashMap::from([
            (
                "anthropic".to_string(),
                ProviderAuth::ApiKey("sk-work".to_string()),
            ),
            (
                "openai".to_string(),
                ProviderAuth::ApiKey("sk-openai".to_string()),
            ),
        ])
    }

    fn account_auth(&self, account: &str) -> Option<(String, ProviderAuth)> {
        match account {
            "work" => Some(("anthropic".into(), ProviderAuth::ApiKey("sk-work".into()))),
            "personal" => Some((
                "anthropic".into(),
                ProviderAuth::ApiKey("sk-personal".into()),
            )),
            "openai-key" => Some(("openai".into(), ProviderAuth::ApiKey("sk-openai".into()))),
            _ => None,
        }
    }
}

#[test]
fn a_plain_model_runs_on_the_providers_default_account() {
    let (keys, model) = crate::session::account_keys(&TwoAccounts, "anthropic/claude-x").unwrap();
    assert_eq!(model, "anthropic/claude-x");
    assert_eq!(keys["anthropic"], ProviderAuth::ApiKey("sk-work".into()));
}

#[test]
fn a_model_naming_an_account_runs_on_that_account_and_asks_for_the_plain_model() {
    let (keys, model) =
        crate::session::account_keys(&TwoAccounts, "anthropic#personal/claude-x").unwrap();
    assert_eq!(
        model, "anthropic/claude-x",
        "providers are asked for the model they know"
    );
    assert_eq!(
        keys["anthropic"],
        ProviderAuth::ApiKey("sk-personal".into()),
        "the provider's key is the account's for the whole turn"
    );
    assert_eq!(
        keys["openai"],
        ProviderAuth::ApiKey("sk-openai".into()),
        "other providers are left as they are"
    );
}

#[test]
fn an_account_that_is_gone_is_refused_and_never_stood_in_for() {
    let error = crate::session::account_keys(&TwoAccounts, "anthropic#gone/claude-x").unwrap_err();
    assert!(error.contains("no longer connected"), "{error}");
    assert!(error.contains("won't move"), "{error}");
}

#[test]
fn an_account_of_another_provider_is_refused() {
    // An id whose account belongs to a different provider is not a way to
    // send one provider's model on another's key.
    let error =
        crate::session::account_keys(&TwoAccounts, "anthropic#openai-key/claude-x").unwrap_err();
    assert!(error.contains("no longer connected"), "{error}");
}
