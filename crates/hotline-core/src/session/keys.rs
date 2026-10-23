use super::ProviderAuth;
use hotline_room::provider_auth::ChosenAccount;
use std::collections::HashMap;
use std::sync::Arc;

/// Where the provider credentials come from.
///
/// The room never holds a secret: it asks at the start of every turn, so a
/// key or login added on the desk is in force on the next one and nothing
/// keeps a stale copy. The desk implements this from the vault and the room
/// log; a test hands over a map.
pub trait ProviderKeys: Send + Sync {
    fn provider_auth(&self) -> HashMap<String, ProviderAuth>;

    /// The same, with `account`'s own auth in place of its provider's
    /// default. Test doubles have one credential per provider, so they
    /// ignore it.
    fn provider_auth_for(&self, _account: Option<&ChosenAccount>) -> HashMap<String, ProviderAuth> {
        self.provider_auth()
    }

    /// The credential a turn on `provider_id` is signed with when its
    /// teammate chose none, and whether the provider has another live one.
    /// Test doubles have none.
    fn default_account(&self, _provider_id: &str) -> Option<(String, bool)> {
        None
    }

    /// The models each provider may offer in the picker. An empty map shows
    /// every model: a missing filter is not an empty one. The desk reads it
    /// from the room log each time, so a saved filter is in force on the
    /// next turn without a restart. Test doubles leave it empty.
    fn enabled_models(&self) -> HashMap<String, Vec<String>> {
        HashMap::new()
    }

    /// The model a Hotline Agent teammate starts on when it has no choice of
    /// its own: the room's `defaultModelId`, else `lastModelId`. The desk
    /// reads the log each time, like [`Self::enabled_models`]. Test doubles
    /// leave it absent.
    fn preferred_model(&self) -> Option<String> {
        None
    }

    /// Validated live metadata, keyed by exact provider/model identity. Missing
    /// fields fall back to that exact bundled entry; no labels choose routes.
    fn model_metadata(&self) -> HashMap<String, crate::contract::CatalogModel> {
        HashMap::new()
    }

    /// Discovered and permitted manual ids for each active connection. An
    /// absent provider uses bundled fallback choices; a present empty list
    /// remains empty. Test doubles leave the map empty.
    fn account_models(&self) -> HashMap<String, Vec<String>> {
        HashMap::new()
    }

    /// The API key saved for each web search provider that has one. The desk
    /// reads them from the vault each time, so a key saved in Settings is in
    /// force on the next search. Test doubles have none: every provider keyless.
    fn web_search_keys(&self) -> crate::websearch::Keys {
        HashMap::new()
    }

    /// The desk's spending budgets, which a teammate's turn on a per-token
    /// key is metered against (the Chat budget). Test doubles have none, so
    /// their turns are neither metered nor refused.
    fn budget(&self) -> Option<Arc<crate::voice::metering::Budget>> {
        None
    }
}
