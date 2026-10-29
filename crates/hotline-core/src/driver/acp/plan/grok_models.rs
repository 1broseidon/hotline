//! Grok Build's models with their levels, from the list the Grok CLI keeps
//! of them: `models_cache.json` in the Grok home, which the CLI fetches and
//! writes each time it starts signed in. A file read: no token, no request,
//! no process.
//!
//! A Grok session opened while the CLI is renewing its sign-in answers with
//! the CLI's built-in fallback list (two models, older ones) and learns the
//! real one only a moment later (`_x.ai/models/update`), so the list a
//! session reports can be short. This is the CLI's last real list.
//!
//! ```json
//! {"fetched_at":"…","models":{"grok-4.7":{"info":{"id":"grok-4.7","name":"Grok 4.7",
//!   "hidden":false,"supports_reasoning_effort":true,"reasoning_effort":"high",
//!   "reasoning_efforts":[{"value":"high","label":"High","description":"…","default":true}]}}}}
//! ```

use crate::contract::ConfigChoice;
use crate::driver::acp::catalog::Catalog;
use serde_json::Value;

/// Grok Build's catalogue, or `None` when the CLI has kept no list.
pub(crate) fn read(env: &(dyn Fn(&str) -> Option<String> + Sync)) -> Option<Catalog> {
    let home = super::grok_billing::grok_home(env)?;
    let text = std::fs::read_to_string(home.join("models_cache.json")).ok()?;
    catalog(&serde_json::from_str(&text).ok()?)
}

/// The catalogue in the CLI's list, in its order, hidden models left out;
/// each model's levels are what its session's effort picker offers.
pub(crate) fn catalog(cache: &Value) -> Option<Catalog> {
    let mut catalog = Catalog {
        effort_id: "reasoning_effort".to_string(),
        effort_name: "Reasoning Effort".to_string(),
        ..Catalog::default()
    };
    for (key, entry) in cache.get("models")?.as_object()? {
        let info = entry.get("info").unwrap_or(entry);
        let text = |field: &str| {
            info.get(field)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
        };
        let id = text("id").unwrap_or(key);
        if id.is_empty() || info.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let levels: &[Value] = if info
            .get("supports_reasoning_effort")
            .and_then(Value::as_bool)
            == Some(true)
        {
            info.get("reasoning_efforts")
                .and_then(Value::as_array)
                .map_or(&[], Vec::as_slice)
        } else {
            &[]
        };
        let mut default_effort = None;
        let efforts: Vec<ConfigChoice> = levels
            .iter()
            .filter_map(|level| {
                let value = level
                    .get("value")
                    .or_else(|| level.get("id"))?
                    .as_str()
                    .filter(|value| !value.is_empty())?;
                if level.get("default").and_then(Value::as_bool) == Some(true) {
                    default_effort = Some(value.to_string());
                }
                Some(ConfigChoice {
                    id: value.to_string(),
                    name: level
                        .get("label")
                        .and_then(Value::as_str)
                        .filter(|label| !label.is_empty())
                        .unwrap_or(value)
                        .to_string(),
                    description: level
                        .get("description")
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                        .map(str::to_string),
                    group: None,
                    account: None,
                    efforts: None,
                    default_effort: None,
                })
            })
            .collect();
        let default_effort = default_effort.or_else(|| {
            text("reasoning_effort")
                .filter(|level| efforts.iter().any(|one| one.id == *level))
                .map(str::to_string)
        });
        catalog.models.push(ConfigChoice {
            id: id.to_string(),
            name: text("name").unwrap_or(id).to_string(),
            description: text("description").map(str::to_string),
            group: None,
            account: None,
            efforts: Some(efforts),
            default_effort,
        });
    }
    (!catalog.models.is_empty()).then_some(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(value: &str, default: bool) -> Value {
        serde_json::json!({"id": value, "value": value, "label": value.to_uppercase(), "default": default})
    }

    #[test]
    fn the_clis_list_is_read_in_its_order_with_each_models_levels() {
        let cache = serde_json::json!({"fetched_at": "2026-10-29T20:08:34Z", "models": {
            "grok-4.7": {"info": {"id": "grok-4.7", "name": "Grok 4.7", "description": "Latest",
                "supports_reasoning_effort": true, "reasoning_effort": "high",
                "reasoning_efforts": [level("xhigh", false), level("high", true), level("low", false)]}},
            "grok-4.7-build-fast": {"info": {"id": "grok-4.7-build-fast", "name": "Grok 4.7 Fast",
                "supports_reasoning_effort": true, "reasoning_effort": "medium",
                "reasoning_efforts": [level("high", false), level("medium", false)]}},
            "grok-secret": {"info": {"id": "grok-secret", "name": "Hidden", "hidden": true}},
            "grok-4.5": {"info": {"id": "grok-4.5", "name": "Grok 4.5", "supports_reasoning_effort": false,
                "reasoning_efforts": [level("high", true)]}}
        }});
        let catalog = catalog(&cache).expect("a list");
        let ids: Vec<&str> = catalog
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect();
        assert_eq!(ids, ["grok-4.7", "grok-4.7-build-fast", "grok-4.5"]);
        let latest = &catalog.models[0];
        assert_eq!(latest.name, "Grok 4.7");
        assert_eq!(latest.description.as_deref(), Some("Latest"));
        let levels: Vec<&str> = latest
            .efforts
            .as_ref()
            .unwrap()
            .iter()
            .map(|one| one.id.as_str())
            .collect();
        assert_eq!(levels, ["xhigh", "high", "low"]);
        assert_eq!(latest.efforts.as_ref().unwrap()[1].name, "HIGH");
        assert_eq!(latest.default_effort.as_deref(), Some("high"));
        // No level marked default: the model's own setting, when it lists it.
        assert_eq!(catalog.models[1].default_effort.as_deref(), Some("medium"));
        // A model without levels is known to have none.
        assert_eq!(catalog.models[2].efforts, Some(vec![]));
        assert_eq!(catalog.models[2].default_effort, None);
        assert_eq!(catalog.effort_id, "reasoning_effort");
        assert_eq!(catalog.default_model, None);
    }

    #[test]
    fn no_list_is_no_catalogue() {
        assert_eq!(catalog(&serde_json::json!({})), None);
        assert_eq!(catalog(&serde_json::json!({"models": {}})), None);
    }

    #[test]
    fn the_list_is_read_from_the_grok_home() {
        let home = tempfile::tempdir().unwrap();
        let env = |name: &str| (name == "GROK_HOME").then(|| home.path().display().to_string());
        assert_eq!(read(&env), None);
        std::fs::write(
            home.path().join("models_cache.json"),
            r#"{"models":{"grok-4.7":{"info":{"id":"grok-4.7","name":"Grok 4.7"}}}}"#,
        )
        .unwrap();
        assert_eq!(read(&env).expect("a list").models[0].id, "grok-4.7");
    }
}
