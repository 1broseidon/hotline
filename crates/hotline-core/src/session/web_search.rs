//! The `web_search` tool's room side: who may search with what, and the call.
//!
//! The providers and the chain are `websearch`; this is only the glue that
//! reads the desk's switches, the teammate's policy and the vault's keys, and
//! hands them over.

use super::Room;
use crate::websearch::{self, Search, query::Query};
use serde_json::Value;

impl Room {
    pub(crate) async fn web_search(
        &self,
        persona_id: &str,
        arguments: &Value,
    ) -> Result<String, String> {
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|query| query.chars().count() >= 2)
            .ok_or_else(|| "web_search needs a `query` of at least two characters.".to_string())?;
        if query.chars().count() > websearch::MAX_QUERY_CHARS {
            return Err(format!(
                "web_search takes a query of at most {} characters.",
                websearch::MAX_QUERY_CHARS
            ));
        }
        let limit = match arguments.get("limit") {
            None | Some(Value::Null) => websearch::DEFAULT_LIMIT,
            Some(value) => value
                .as_u64()
                .ok_or_else(|| "web_search's `limit` is a whole number.".to_string())?
                .clamp(1, websearch::MAX_LIMIT as u64) as usize,
        };

        let settings = crate::room::try_settings(self.log())?;
        let persona = self.persona(persona_id)?;
        let keys = self.keys.web_search_keys();
        let providers = websearch::effective_chain(
            &websearch::disabled_on_desk(&settings),
            persona.web_search_policy.as_ref(),
        );
        if providers.is_empty() {
            return Err("Web search is switched off for you. The person can turn it on in Settings, Tools, or in your own tools.".to_string());
        }

        let client = websearch::client();
        #[cfg(test)]
        let endpoints = super::lock(&self.web_search_endpoints)
            .clone()
            .unwrap_or_default();
        #[cfg(not(test))]
        let endpoints = websearch::Endpoints::default();
        let attempts = providers
            .into_iter()
            .map(|provider| {
                websearch::attempt(provider, &client, keys.get(&provider).cloned(), &endpoints)
            })
            .collect();
        let answered = Search::new(attempts)
            .run(&Query::parse(query), limit)
            .await
            .map_err(|error| format!("Web search failed: {error}."))?;
        Ok(websearch::render(&answered))
    }

    #[cfg(test)]
    pub(crate) fn set_web_search_endpoints(&self, endpoints: websearch::Endpoints) {
        *super::lock(&self.web_search_endpoints) = Some(endpoints);
    }
}
