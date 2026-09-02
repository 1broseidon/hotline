//! The models Toad Agent can offer, and the providers that serve them.
//!
//! Nobody hand-writes a model list here. `models.json` beside this crate is a
//! filtered snapshot of models.dev — the catalogue opencode and pi draw
//! theirs from — and `toad-models-sync` rewrites it: fetch, keep the
//! providers in [`WIRING`] and the models a coding agent can use, write. A
//! refresh is one command and one diff to read. The one thing a person edits
//! is [`WIRING`]: which providers Toad reaches, and which Rig client speaks
//! to each.
//!
//! The snapshot is checked in rather than fetched at run time because a
//! catalogue is behaviour: it says what a model is called, costs and can do,
//! and a release should mean the same thing on every machine that runs it.

use crate::contract::{ConfigChoice, Provider};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

/// Where the snapshot comes from.
pub const SOURCE: &str = "https://models.dev/api.json";

/// The Rig client a provider is spoken to with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    Anthropic,
    OpenAi,
    OpenRouter,
    Gemini,
    XAi,
    Groq,
    DeepSeek,
    Mistral,
}

/// One provider Toad reaches: its models.dev id, and how it is spoken to.
pub struct Wiring {
    pub id: &'static str,
    pub client: Client,
}

/// The providers Toad reaches, in the order the key form and the picker
/// offer them. The sync keeps exactly these out of the catalogue, so adding
/// a provider is one line here, a `Client` arm in the driver, and a sync.
pub const WIRING: &[Wiring] = &[
    Wiring {
        id: "anthropic",
        client: Client::Anthropic,
    },
    Wiring {
        id: "openai",
        client: Client::OpenAi,
    },
    Wiring {
        id: "openrouter",
        client: Client::OpenRouter,
    },
    Wiring {
        id: "google",
        client: Client::Gemini,
    },
    Wiring {
        id: "xai",
        client: Client::XAi,
    },
    Wiring {
        id: "groq",
        client: Client::Groq,
    },
    Wiring {
        id: "deepseek",
        client: Client::DeepSeek,
    },
    Wiring {
        id: "mistral",
        client: Client::Mistral,
    },
];

pub fn wiring(provider_id: &str) -> Option<&'static Wiring> {
    WIRING.iter().find(|wiring| wiring.id == provider_id)
}

/// The snapshot: `models.json` as written by the sync and read at start.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    pub source: String,
    /// The day the snapshot was taken, `YYYY-MM-DD`.
    pub synced: String,
    pub providers: BTreeMap<String, ProviderEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// Keyed by the id the provider's API takes, which for a router such as
    /// OpenRouter has a slash of its own (`anthropic/claude-opus-5`).
    pub models: BTreeMap<String, Model>,
}

/// What Toad keeps of a models.dev model. The fields are the catalogue's own
/// names, so a field it has and this struct lacks is one line to add and a
/// sync to fill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    pub release_date: String,
    /// `beta` when the catalogue says so. A deprecated model is not kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default)]
    pub reasoning: bool,
    /// Takes files (images, PDFs) alongside text.
    #[serde(default)]
    pub attachment: bool,
    /// Dollars per million tokens. Absent when the catalogue has no price,
    /// as for an open-weights model on a free tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    pub limit: Limit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    pub input: f64,
    pub output: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Limit {
    pub context: u64,
    pub output: u64,
}

/// The snapshot this build carries.
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../models.json"))
            .expect("models.json is written by toad-models-sync; a test checks it parses")
    })
}

/// Whether a coding agent can use a model: it calls tools, answers in text
/// and nothing else, and its provider has not retired it.
fn usable(model: &Value) -> bool {
    let calls_tools = model["tool_call"].as_bool() == Some(true);
    let text_out = model["modalities"]["output"]
        .as_array()
        .is_some_and(|output| output.len() == 1 && output[0] == "text");
    let retired = model["status"].as_str() == Some("deprecated");
    calls_tools && text_out && !retired
}

/// The catalogue cut down to what Toad ships: each wired provider, and the
/// models of it that [`usable`] keeps. A wired provider the catalogue lacks,
/// or one left with no usable model, is an error rather than an absence — a
/// sync that quietly drops a provider is a picker that quietly empties.
pub fn snapshot(api: &Value, synced: &str) -> Result<Catalog, String> {
    let mut providers = BTreeMap::new();
    for wiring in WIRING {
        let provider = api
            .get(wiring.id)
            .ok_or_else(|| format!("models.dev has no provider `{}`", wiring.id))?;
        let name = provider["name"]
            .as_str()
            .ok_or_else(|| format!("provider `{}` has no name", wiring.id))?
            .to_string();
        let doc = provider["doc"].as_str().map(str::to_string);
        let mut models = BTreeMap::new();
        for (id, model) in provider["models"].as_object().into_iter().flatten() {
            if !usable(model) {
                continue;
            }
            let model: Model = serde_json::from_value(model.clone())
                .map_err(|error| format!("{}/{id}: {error}", wiring.id))?;
            models.insert(id.clone(), model);
        }
        if models.is_empty() {
            return Err(format!("provider `{}` has no usable model", wiring.id));
        }
        providers.insert(wiring.id.to_string(), ProviderEntry { name, doc, models });
    }
    Ok(Catalog {
        source: SOURCE.to_string(),
        synced: synced.to_string(),
        providers,
    })
}

/// Every provider Toad Agent can hold a key for, wired order.
pub fn providers() -> Vec<Provider> {
    WIRING
        .iter()
        .filter_map(|wiring| {
            let entry = catalog().providers.get(wiring.id)?;
            Some(Provider {
                id: wiring.id.to_string(),
                name: entry.name.clone(),
                doc: entry.doc.clone(),
            })
        })
        .collect()
}

/// The models the given provider keys unlock, as the picker lists them:
/// providers in wired order, and within one the newest model first. An id
/// on the wire is `provider/model`, the shape the room has always stored, so
/// a teammate's saved choice keeps meaning the same thing.
pub fn choices(keys: &HashMap<String, String>) -> Vec<ConfigChoice> {
    WIRING
        .iter()
        .filter(|wiring| keys.contains_key(wiring.id))
        .filter_map(|wiring| Some((wiring, catalog().providers.get(wiring.id)?)))
        .flat_map(|(wiring, entry)| {
            let mut models: Vec<(&String, &Model)> = entry.models.iter().collect();
            models.sort_by(|a, b| b.1.release_date.cmp(&a.1.release_date).then(a.0.cmp(b.0)));
            models.into_iter().map(move |(id, model)| ConfigChoice {
                id: format!("{}/{id}", wiring.id),
                name: model.name.clone(),
                description: Some(wiring.id.to_string()),
                group: Some(format!("{} — API key", entry.name)),
            })
        })
        .collect()
}

/// The picker's name for a `provider/model` id, when the catalogue has it.
pub fn label_of(model_id: &str) -> Option<String> {
    let (provider, model) = model_id.split_once('/')?;
    catalog()
        .providers
        .get(provider)?
        .models
        .get(model)
        .map(|model| model.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_shipped_snapshot_is_exactly_the_wired_providers() {
        let catalog = catalog();
        assert_eq!(catalog.source, SOURCE);
        for wiring in WIRING {
            let entry = catalog
                .providers
                .get(wiring.id)
                .unwrap_or_else(|| panic!("{} is wired but not in models.json", wiring.id));
            assert!(!entry.models.is_empty(), "{} has no models", wiring.id);
        }
        for id in catalog.providers.keys() {
            assert!(wiring(id).is_some(), "{id} is in models.json but not wired");
        }
    }

    fn model(tool_call: bool, output: &[&str], status: Option<&str>) -> Value {
        json!({
            "id": "m", "name": "M", "release_date": "2026-01-01", "status": status,
            "tool_call": tool_call, "reasoning": true, "attachment": false,
            "modalities": {"input": ["text"], "output": output},
            "cost": {"input": 1.0, "output": 2.0}, "limit": {"context": 1000, "output": 100}
        })
    }

    /// Every wired provider, each with one plain usable model, so a test can
    /// vary one provider without the others failing the snapshot.
    fn api() -> Value {
        let mut api = serde_json::Map::new();
        for wiring in WIRING {
            api.insert(
                wiring.id.to_string(),
                json!({"name": wiring.id, "models": {"plain": model(true, &["text"], None)}}),
            );
        }
        Value::Object(api)
    }

    #[test]
    fn a_snapshot_keeps_tool_calling_text_models_of_wired_providers_only() {
        let mut api = api();
        api["anthropic"]["models"]["no-tools"] = model(false, &["text"], None);
        api["anthropic"]["models"]["draws"] = model(true, &["text", "image"], None);
        api["anthropic"]["models"]["retired"] = model(true, &["text"], Some("deprecated"));
        api["anthropic"]["models"]["beta"] = model(true, &["text"], Some("beta"));
        api["anthropic"]["doc"] = json!("https://example.test/models");
        api["unwired"] = json!({"name": "Nope", "models": {"x": model(true, &["text"], None)}});

        let catalog = snapshot(&api, "2026-09-02").unwrap();
        assert_eq!(catalog.synced, "2026-09-02");
        assert!(!catalog.providers.contains_key("unwired"));
        let anthropic = &catalog.providers["anthropic"];
        assert_eq!(
            anthropic.doc.as_deref(),
            Some("https://example.test/models")
        );
        let kept: Vec<&String> = anthropic.models.keys().collect();
        assert_eq!(kept, ["beta", "plain"]);
        assert_eq!(anthropic.models["beta"].status.as_deref(), Some("beta"));
        assert_eq!(anthropic.models["plain"].cost.as_ref().unwrap().input, 1.0);
    }

    #[test]
    fn a_wired_provider_the_catalogue_lacks_or_empties_fails_the_sync() {
        let mut api = api();
        api["anthropic"]["models"] = json!({"only": model(false, &["text"], None)});
        assert_eq!(
            snapshot(&api, "d").unwrap_err(),
            "provider `anthropic` has no usable model"
        );
        api.as_object_mut().unwrap().remove("anthropic");
        assert_eq!(
            snapshot(&api, "d").unwrap_err(),
            "models.dev has no provider `anthropic`"
        );
    }

    #[test]
    fn a_snapshot_reads_back_as_itself() {
        let catalog = snapshot(&api(), "2026-09-02").unwrap();
        let text = serde_json::to_string_pretty(&catalog).unwrap();
        assert_eq!(serde_json::from_str::<Catalog>(&text).unwrap(), catalog);
    }

    #[test]
    fn choices_follow_the_keys_the_desk_holds_newest_first() {
        let mut keys = HashMap::new();
        assert!(choices(&keys).is_empty());
        keys.insert("anthropic".to_string(), "k".to_string());
        let listed = choices(&keys);
        assert!(
            listed
                .iter()
                .all(|model| model.id.starts_with("anthropic/"))
        );
        assert_eq!(listed[0].group.as_deref(), Some("Anthropic — API key"));
        let entry = &catalog().providers["anthropic"];
        let dates: Vec<&str> = listed
            .iter()
            .map(|choice| {
                entry.models[choice.id.trim_start_matches("anthropic/")]
                    .release_date
                    .as_str()
            })
            .collect();
        let mut sorted = dates.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(dates, sorted);
        assert_eq!(
            label_of(&listed[0].id).as_deref(),
            Some(listed[0].name.as_str())
        );
        assert_eq!(label_of("anthropic/nope"), None);
        assert_eq!(label_of("bare"), None);
    }

    #[test]
    fn providers_are_offered_in_wired_order() {
        let ids: Vec<String> = providers().into_iter().map(|one| one.id).collect();
        let wired: Vec<&str> = WIRING.iter().map(|wiring| wiring.id).collect();
        assert_eq!(ids, wired);
        assert_eq!(providers()[0].name, "Anthropic");
    }
}
