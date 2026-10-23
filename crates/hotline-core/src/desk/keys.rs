use crate::contract::CatalogModel;
use crate::log::Log;
use crate::session::{ProviderAuth, ProviderKeys};
use crate::vault::Vault;
use hotline_room::provider_auth::ChosenAccount;
use std::collections::HashMap;
use std::sync::Arc;

/// The vault's keys and the room's model settings, as one seam.
///
/// The driver builds `SessionInfo.models` from [`ProviderKeys`] alone and
/// has no log. Reading the filter and the preferred model here, each time,
/// is how a saved choice is in force on the next turn without a restart.
pub(super) struct DeskCredentials {
    pub(super) vault: Arc<Vault>,
    pub(super) log: Log,
    pub(super) budget: Arc<crate::voice::metering::Budget>,
}

impl ProviderKeys for DeskCredentials {
    fn provider_auth(&self) -> HashMap<String, ProviderAuth> {
        Vault::provider_auth(&self.vault)
    }

    fn provider_auth_for(&self, account: Option<&ChosenAccount>) -> HashMap<String, ProviderAuth> {
        self.vault.provider_auth_for(account)
    }

    fn default_account(&self, provider_id: &str) -> Option<(String, bool)> {
        self.vault.default_account(provider_id)
    }

    fn enabled_models(&self) -> HashMap<String, Vec<String>> {
        crate::models::enabled_models(&crate::room::settings(&self.log))
    }

    fn preferred_model(&self) -> Option<String> {
        crate::models::preferred_model(&crate::room::settings(&self.log))
    }

    fn model_metadata(&self) -> HashMap<String, CatalogModel> {
        self.vault.model_metadata()
    }

    fn account_models(&self) -> HashMap<String, Vec<String>> {
        self.vault.account_models()
    }

    fn web_search_keys(&self) -> crate::websearch::Keys {
        self.vault.web_search_keys()
    }

    fn budget(&self) -> Option<Arc<crate::voice::metering::Budget>> {
        Some(self.budget.clone())
    }
}
