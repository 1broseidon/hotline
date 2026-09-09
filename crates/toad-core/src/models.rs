//! The models Toad Agent can offer, and the providers that serve them.
//!
//! Nobody hand-writes a model list here, with one exception. `models.json`
//! beside this crate is a filtered snapshot of models.dev — the catalogue
//! opencode and pi draw theirs from — and `toad-models-sync` rewrites it:
//! fetch, keep the providers in [`WIRING`] and the models a coding agent can
//! use, write. A refresh is one command and one diff to read. The one thing
//! a person edits is [`WIRING`]: which providers Toad reaches, which Rig
//! client speaks to each, and whether a credential is a key or a login.
//!
//! `openai-codex` is the exception: models.dev has no ChatGPT subscription
//! provider, so the snapshot copies the listed models off `openai` and
//! drops their per-token price. A subscription has no per-token price.
//!
//! The snapshot is checked in rather than fetched at run time because a
//! catalogue is behaviour: it says what a model is called, costs and can do,
//! and a release should mean the same thing on every machine that runs it.

use crate::contract::{CatalogModel, ConfigChoice, CredentialKind, Provider};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
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
    ChatGpt,
    Copilot,
}

/// One provider Toad reaches: its models.dev id, how it is spoken to, and
/// what a credential for it is.
pub struct Wiring {
    pub id: &'static str,
    pub client: Client,
    pub credential_kind: CredentialKind,
}

/// Model ids ChatGPT's subscription serves. Copied from openai's catalogue
/// with `cost` cleared, because a subscription has no per-token price.
/// An id openai lacks is an error from the sync, so this list cannot drift
/// silently.
pub const CHATGPT_MODELS: &[&str] = &[
    "gpt-6-astra",
    "gpt-5.6",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
    "gpt-5.4",
];

/// The providers Toad reaches, in wired order — which is how the model
/// picker groups them. The key form sorts by name, because mixing two acts
/// in wired order is how a login hid among keys. The sync keeps exactly
/// these out of the catalogue, so adding a provider is one line here, a
/// `Client` arm in the driver, and a sync.
pub const WIRING: &[Wiring] = &[
    Wiring {
        id: "anthropic",
        client: Client::Anthropic,
        credential_kind: CredentialKind::ApiKey,
    },
    Wiring {
        id: "openai",
        client: Client::OpenAi,
        credential_kind: CredentialKind::ApiKey,
    },
    Wiring {
        id: "openrouter",
        client: Client::OpenRouter,
        credential_kind: CredentialKind::ApiKey,
    },
    Wiring {
        id: "google",
        client: Client::Gemini,
        credential_kind: CredentialKind::ApiKey,
    },
    Wiring {
        id: "xai",
        client: Client::XAi,
        credential_kind: CredentialKind::ApiKey,
    },
    Wiring {
        id: "groq",
        client: Client::Groq,
        credential_kind: CredentialKind::ApiKey,
    },
    Wiring {
        id: "deepseek",
        client: Client::DeepSeek,
        credential_kind: CredentialKind::ApiKey,
    },
    Wiring {
        id: "mistral",
        client: Client::Mistral,
        credential_kind: CredentialKind::ApiKey,
    },
    Wiring {
        id: "github-copilot",
        client: Client::Copilot,
        credential_kind: CredentialKind::Oauth,
    },
    // models.dev has no ChatGPT subscription provider. This row is
    // hand-written so the picker can offer the same models billed as a login.
    Wiring {
        id: "openai-codex",
        client: Client::ChatGpt,
        credential_kind: CredentialKind::Oauth,
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
    /// Effort levels this model offers, from models.dev's `reasoning_options`
    /// entry of type `effort`, in the order the catalogue lists them. Empty
    /// when the model has no such option — only that type is read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
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

/// One catalogue model, with `efforts` taken from the raw `reasoning_options`
/// rather than from a field models.dev already spells the way we store it.
fn take_model(provider: &str, id: &str, raw: &Value) -> Result<Model, String> {
    let mut model: Model =
        serde_json::from_value(raw.clone()).map_err(|error| format!("{provider}/{id}: {error}"))?;
    model.efforts = efforts_of(raw);
    Ok(model)
}

/// The `effort` entry of `reasoning_options`, in catalogue order. A `toggle`
/// or `budget_tokens` option is not an effort list and is ignored.
fn efforts_of(model: &Value) -> Vec<String> {
    model
        .get("reasoning_options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|option| option.get("type").and_then(Value::as_str) == Some("effort"))
        .and_then(|option| option.get("values").and_then(Value::as_array))
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
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
/// `openai-codex` is synthesised from openai rather than looked up, because
/// models.dev has no ChatGPT subscription provider.
pub fn snapshot(api: &Value, synced: &str) -> Result<Catalog, String> {
    let mut providers = BTreeMap::new();
    for wiring in WIRING {
        if wiring.id == "openai-codex" {
            providers.insert(wiring.id.to_string(), chatgpt_from_openai(api)?);
            continue;
        }
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
            let model = take_model(wiring.id, id, model)?;
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

/// ChatGPT's catalogue row: openai's listed models with the price taken off.
fn chatgpt_from_openai(api: &Value) -> Result<ProviderEntry, String> {
    let openai = api
        .get("openai")
        .ok_or_else(|| "models.dev has no provider `openai`".to_string())?;
    let mut models = BTreeMap::new();
    for id in CHATGPT_MODELS {
        let model = openai["models"].get(*id).ok_or_else(|| {
            format!("openai-codex needs openai model `{id}`, and models.dev does not have it")
        })?;
        let mut model = take_model("openai-codex", id, model)?;
        model.cost = None;
        models.insert((*id).to_string(), model);
    }
    Ok(ProviderEntry {
        name: "ChatGPT".to_string(),
        doc: Some("https://chatgpt.com".to_string()),
        models,
    })
}

/// Every provider Toad Agent can hold a credential for, wired order.
pub fn providers() -> Vec<Provider> {
    WIRING
        .iter()
        .filter_map(|wiring| {
            let entry = catalog().providers.get(wiring.id)?;
            Some(Provider {
                id: wiring.id.to_string(),
                name: entry.name.clone(),
                doc: entry.doc.clone(),
                credential_kind: wiring.credential_kind,
            })
        })
        .collect()
}

/// Why a device-code login cannot start for this provider, when it cannot.
pub fn login_refusal(provider_id: &str) -> Option<String> {
    match wiring(provider_id) {
        None => Some(format!(
            "{provider_id} is not a provider Toad Agent can use."
        )),
        Some(wiring) if wiring.credential_kind == CredentialKind::Oauth => None,
        Some(wiring) => {
            let name = catalog()
                .providers
                .get(wiring.id)
                .map(|entry| entry.name.as_str())
                .unwrap_or(wiring.id);
            Some(format!("{name} takes an API key, not a sign-in."))
        }
    }
}

/// The room's standing model preference: `defaultModelId` when it is a
/// string, else `lastModelId`. A non-string value reads as absent, the same
/// as a key nobody has set — a bad setting costs its own preference, never
/// the picker.
pub fn preferred_model(settings: &Map<String, Value>) -> Option<String> {
    setting_string(settings, "defaultModelId").or_else(|| setting_string(settings, "lastModelId"))
}

fn setting_string(settings: &Map<String, Value>, key: &str) -> Option<String> {
    settings
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// The room's `enabledModels` setting, as the picker applies it.
///
/// A provider absent from the object shows every model. A present one shows
/// only the listed ids. A value that is not an object, or an entry that is
/// not an array of strings, reads as absent — a bad setting costs its own
/// filter, never the picker.
pub fn enabled_models(settings: &Map<String, Value>) -> HashMap<String, Vec<String>> {
    let Some(Value::Object(map)) = settings.get("enabledModels") else {
        return HashMap::new();
    };
    map.iter()
        .filter_map(|(provider, value)| {
            let ids: Vec<String> = value
                .as_array()?
                .iter()
                .map(|item| item.as_str().map(str::to_string))
                .collect::<Option<_>>()?;
            Some((provider.clone(), ids))
        })
        .collect()
}

/// Whether this provider's filter lets this catalogue id through. Absent
/// from the map means every model; present means only the listed ids.
fn model_offered(
    enabled: &HashMap<String, Vec<String>>,
    provider_id: &str,
    model_id: &str,
) -> bool {
    enabled
        .get(provider_id)
        .is_none_or(|list| list.iter().any(|wanted| wanted == model_id))
}

/// Whether a subscription's account list lets this catalogue id through.
/// A key provider is never narrowed this way. `None` is the whole catalogue;
/// a present list is only those bare ids — the same "narrows what is offered,
/// never what is in use" rule as [`model_offered`].
fn account_offered(kind: CredentialKind, account: Option<&[String]>, model_id: &str) -> bool {
    if kind != CredentialKind::Oauth {
        return true;
    }
    account.is_none_or(|list| list.iter().any(|id| id == model_id))
}

/// The models of one catalogue provider, newest first.
fn newest_first(entry: &ProviderEntry) -> Vec<(&String, &Model)> {
    let mut models: Vec<(&String, &Model)> = entry.models.iter().collect();
    models.sort_by(|a, b| b.1.release_date.cmp(&a.1.release_date).then(a.0.cmp(b.0)));
    models
}

/// The models the given provider credentials unlock, as the picker lists
/// them: providers in wired order, and within one the newest model first. An
/// id on the wire is `provider/model`, the shape the room has always stored,
/// so a teammate's saved choice keeps meaning the same thing. The keys map's
/// values are unused; presence is what unlocks a group. `enabled` is the
/// saved filter: a provider absent from it shows every model, a present one
/// only the listed ids. `account` is the same shape for a subscription's
/// held list: present means only those bare ids, absent the whole catalogue.
/// Both narrow what is offered, never a model already in use.
pub fn choices<T>(
    keys: &HashMap<String, T>,
    enabled: &HashMap<String, Vec<String>>,
    account: &HashMap<String, Vec<String>>,
) -> Vec<ConfigChoice> {
    WIRING
        .iter()
        .filter(|wiring| keys.contains_key(wiring.id))
        .filter_map(|wiring| Some((wiring, catalog().providers.get(wiring.id)?)))
        .flat_map(|(wiring, entry)| {
            let flavor = match wiring.credential_kind {
                CredentialKind::ApiKey => "API key",
                CredentialKind::Oauth => "subscription",
            };
            newest_first(entry)
                .into_iter()
                .filter(|(id, _)| {
                    model_offered(enabled, wiring.id, id)
                        && account_offered(
                            wiring.credential_kind,
                            account.get(wiring.id).map(Vec::as_slice),
                            id,
                        )
                })
                .map(move |(id, model)| ConfigChoice {
                    id: format!("{}/{id}", wiring.id),
                    name: model.name.clone(),
                    description: Some(wiring.id.to_string()),
                    group: Some(format!("{} — {flavor}", entry.name)),
                })
        })
        .collect()
}

/// Every model of this provider in the catalogue, newest first, each flagged
/// by the saved filter. All `enabled` when the provider is absent from it.
/// A subscription with an `account` list omits models the account cannot
/// run, so the Models shown panel has no checkbox for them. An unwired
/// provider, or one the snapshot has not got, is an empty list; the wire
/// turns that into an error before it gets here.
pub fn catalog_models(
    provider_id: &str,
    enabled: &HashMap<String, Vec<String>>,
    account: Option<&[String]>,
) -> Vec<CatalogModel> {
    let Some(entry) = catalog().providers.get(provider_id) else {
        return Vec::new();
    };
    let kind = wiring(provider_id)
        .map(|wiring| wiring.credential_kind)
        .unwrap_or(CredentialKind::ApiKey);
    newest_first(entry)
        .into_iter()
        .filter(|(id, _)| account_offered(kind, account, id))
        .map(|(id, model)| CatalogModel {
            id: id.clone(),
            name: model.name.clone(),
            release_date: model.release_date.clone(),
            enabled: model_offered(enabled, provider_id, id),
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

/// The effort levels a `provider/model` offers, empty when the id is unknown
/// or the model has no `effort` option.
pub fn efforts(model_id: &str) -> Vec<String> {
    let Some((provider, model)) = model_id.split_once('/') else {
        return Vec::new();
    };
    catalog()
        .providers
        .get(provider)
        .and_then(|entry| entry.models.get(model))
        .map(|model| model.efforts.clone())
        .unwrap_or_default()
}

/// The picker's label for an effort id. One function, so the driver and the
/// idle picker cannot drift.
pub fn effort_label(id: &str) -> String {
    match id {
        "none" => "None",
        "minimal" => "Minimal",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra high",
        "max" => "Max",
        "default" => "Default",
        other => return other.to_string(),
    }
    .to_string()
}

/// The idle picker's choices for a model's efforts.
pub fn effort_choices(model_id: &str) -> Vec<ConfigChoice> {
    efforts(model_id)
        .into_iter()
        .map(|id| ConfigChoice {
            name: effort_label(&id),
            id,
            description: None,
            group: None,
        })
        .collect()
}

/// The most tokens one answer from a `provider/model` may hold, when the
/// catalogue has it.
pub fn output_limit(model_id: &str) -> Option<u64> {
    let (provider, model) = model_id.split_once('/')?;
    catalog()
        .providers
        .get(provider)?
        .models
        .get(model)
        .map(|model| model.limit.output)
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
    /// `openai-codex` is not a models.dev provider: it is synthesised from
    /// openai, so openai also carries the ChatGPT ids.
    fn api() -> Value {
        let mut api = serde_json::Map::new();
        for wiring in WIRING {
            if wiring.id == "openai-codex" {
                continue;
            }
            let mut models = serde_json::Map::new();
            models.insert("plain".into(), model(true, &["text"], None));
            if wiring.id == "openai" {
                for id in CHATGPT_MODELS {
                    models.insert((*id).into(), model(true, &["text"], None));
                }
            }
            api.insert(
                wiring.id.to_string(),
                json!({"name": wiring.id, "models": models}),
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
        let none = HashMap::new();
        assert!(choices(&keys, &none, &none).is_empty());
        keys.insert("anthropic".to_string(), "k".to_string());
        let listed = choices(&keys, &none, &none);
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
    fn preferred_model_reads_default_then_last_and_ignores_a_non_string() {
        let mut settings = Map::new();
        assert_eq!(preferred_model(&settings), None);

        settings.insert("lastModelId".into(), json!("openai/gpt"));
        assert_eq!(preferred_model(&settings).as_deref(), Some("openai/gpt"));

        settings.insert("defaultModelId".into(), json!("anthropic/claude"));
        assert_eq!(
            preferred_model(&settings).as_deref(),
            Some("anthropic/claude")
        );

        settings.insert("defaultModelId".into(), json!(1));
        assert_eq!(preferred_model(&settings).as_deref(), Some("openai/gpt"));

        settings.insert("lastModelId".into(), json!(true));
        assert_eq!(preferred_model(&settings), None);
    }

    #[test]
    fn enabled_models_reads_an_object_of_string_arrays_and_ignores_the_rest() {
        let mut settings = Map::new();
        assert!(enabled_models(&settings).is_empty());

        settings.insert("enabledModels".into(), json!("nope"));
        assert!(enabled_models(&settings).is_empty());

        settings.insert(
            "enabledModels".into(),
            json!({
                "openrouter": ["anthropic/claude-opus-5", "openai/gpt-5"],
                "anthropic": [1, "claude"],
                "openai": "not-an-array",
                "xai": ["grok-4"],
            }),
        );
        let got = enabled_models(&settings);
        assert_eq!(
            got.get("openrouter"),
            Some(&vec![
                "anthropic/claude-opus-5".to_string(),
                "openai/gpt-5".to_string()
            ])
        );
        assert!(!got.contains_key("anthropic"));
        assert!(!got.contains_key("openai"));
        assert_eq!(got.get("xai"), Some(&vec!["grok-4".to_string()]));
    }

    #[test]
    fn choices_omit_a_model_the_filter_did_not_list() {
        let mut keys = HashMap::new();
        keys.insert("anthropic".to_string(), "k");
        let none = HashMap::new();
        let all = choices(&keys, &none, &none);
        assert!(all.len() > 1, "anthropic must list more than one model");

        let kept = all[0].id.trim_start_matches("anthropic/").to_string();
        let mut filter = HashMap::new();
        filter.insert("anthropic".to_string(), vec![kept.clone()]);
        let listed = choices(&keys, &filter, &none);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, format!("anthropic/{kept}"));

        filter.insert("openai".to_string(), vec!["nope".to_string()]);
        assert_eq!(choices(&keys, &filter, &none).len(), 1);
    }

    #[test]
    fn catalog_models_are_newest_first_and_a_filter_flags_them() {
        let none = HashMap::new();
        let all = catalog_models("anthropic", &none, None);
        assert!(!all.is_empty());
        assert!(all.iter().all(|model| model.enabled));
        let dates: Vec<&str> = all
            .iter()
            .map(|model| model.release_date.as_str())
            .collect();
        let mut sorted = dates.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(dates, sorted);

        let mut filter = HashMap::new();
        filter.insert("anthropic".to_string(), vec![all[0].id.clone()]);
        let flagged = catalog_models("anthropic", &filter, None);
        assert_eq!(flagged.len(), all.len());
        assert!(flagged[0].enabled);
        assert!(flagged.iter().skip(1).all(|model| !model.enabled));
        assert!(catalog_models("nope", &none, None).is_empty());
    }

    #[test]
    fn providers_are_offered_in_wired_order() {
        let ids: Vec<String> = providers().into_iter().map(|one| one.id).collect();
        let wired: Vec<&str> = WIRING.iter().map(|wiring| wiring.id).collect();
        assert_eq!(ids, wired);
        assert_eq!(providers()[0].name, "Anthropic");
        assert_eq!(providers()[0].credential_kind, CredentialKind::ApiKey);
    }

    #[test]
    fn openai_codex_is_openai_models_billed_as_a_login() {
        let mut api = api();
        let catalog = snapshot(&api, "d").unwrap();
        let entry = &catalog.providers["openai-codex"];
        assert_eq!(entry.name, "ChatGPT");
        assert_eq!(entry.doc.as_deref(), Some("https://chatgpt.com"));
        for id in CHATGPT_MODELS {
            assert!(entry.models.contains_key(*id), "{id}");
            assert!(
                entry.models[*id].cost.is_none(),
                "{id} kept a per-token price"
            );
        }
        api["openai"]["models"]
            .as_object_mut()
            .unwrap()
            .remove("gpt-5.6");
        let err = snapshot(&api, "d").unwrap_err();
        assert!(
            err.contains("gpt-5.6"),
            "a missing openai id must fail the sync: {err}"
        );
    }

    #[test]
    fn a_snapshot_reads_an_effort_option_and_ignores_toggle_and_budget() {
        let mut api = api();
        api["anthropic"]["models"]["plain"]["reasoning_options"] = json!([
            {"type": "toggle"},
            {"type": "budget_tokens", "min": 1024, "max": 32000},
            {"type": "effort", "values": ["low", "medium", "high"]},
        ]);
        let catalog = snapshot(&api, "d").unwrap();
        assert_eq!(
            catalog.providers["anthropic"].models["plain"].efforts,
            ["low", "medium", "high"]
        );
        assert!(
            catalog.providers["openai"].models["plain"]
                .efforts
                .is_empty()
        );
    }

    #[test]
    fn openai_codex_inherits_openai_efforts() {
        let mut api = api();
        api["openai"]["models"]["gpt-5.6"]["reasoning_options"] = json!([
            {"type": "effort", "values": ["none", "low", "medium", "high"]}
        ]);
        let catalog = snapshot(&api, "d").unwrap();
        assert_eq!(
            catalog.providers["openai-codex"].models["gpt-5.6"].efforts,
            ["none", "low", "medium", "high"]
        );
    }

    #[test]
    fn efforts_lists_a_known_model_and_is_empty_for_an_unknown() {
        let known = catalog()
            .providers
            .iter()
            .find_map(|(provider, entry)| {
                entry.models.iter().find_map(|(id, model)| {
                    (!model.efforts.is_empty())
                        .then(|| (format!("{provider}/{id}"), model.efforts.clone()))
                })
            })
            .expect("the snapshot has at least one model with an effort list");
        assert_eq!(efforts(&known.0), known.1);
        assert!(efforts("nope/nope").is_empty());
        assert!(efforts("bare").is_empty());
    }

    #[test]
    fn an_oauth_providers_models_are_labelled_as_a_subscription() {
        let mut keys = HashMap::new();
        keys.insert("openai-codex".to_string(), "ignored");
        let listed = choices(&keys, &HashMap::new(), &HashMap::new());
        assert!(
            listed
                .iter()
                .all(|model| model.id.starts_with("openai-codex/"))
        );
        assert_eq!(listed[0].group.as_deref(), Some("ChatGPT — subscription"));
    }

    #[test]
    fn choices_narrow_an_oauth_account_and_leave_key_providers_alone() {
        let mut keys = HashMap::new();
        keys.insert("github-copilot".to_string(), "ignored");
        keys.insert("anthropic".to_string(), "k");
        let none = HashMap::new();
        let all = choices(&keys, &none, &none);
        let copilot: Vec<&str> = all
            .iter()
            .filter(|choice| choice.id.starts_with("github-copilot/"))
            .map(|choice| choice.id.as_str())
            .collect();
        assert!(
            copilot.len() > 2,
            "github-copilot must list more than two models"
        );
        let first = copilot[0].trim_start_matches("github-copilot/").to_string();
        let later = copilot[2].trim_start_matches("github-copilot/").to_string();

        let mut account = HashMap::new();
        account.insert(
            "github-copilot".to_string(),
            vec![later.clone(), first.clone(), "copilot-search-a".to_string()],
        );
        account.insert("anthropic".to_string(), vec!["nope".to_string()]);
        let listed = choices(&keys, &none, &account);
        let copilot: Vec<String> = listed
            .iter()
            .filter(|choice| choice.id.starts_with("github-copilot/"))
            .map(|choice| choice.id.clone())
            .collect();
        assert_eq!(
            copilot,
            vec![
                format!("github-copilot/{first}"),
                format!("github-copilot/{later}"),
            ]
        );

        let anthropic: Vec<&str> = listed
            .iter()
            .filter(|choice| choice.id.starts_with("anthropic/"))
            .map(|choice| choice.id.as_str())
            .collect();
        let all_anthropic: Vec<&str> = all
            .iter()
            .filter(|choice| choice.id.starts_with("anthropic/"))
            .map(|choice| choice.id.as_str())
            .collect();
        assert_eq!(anthropic, all_anthropic);
    }

    #[test]
    fn catalog_models_intersect_an_oauth_account_in_catalogue_order() {
        let none = HashMap::new();
        let all = catalog_models("github-copilot", &none, None);
        assert!(all.len() > 2);
        let first = all[0].id.clone();
        let later = all[2].id.clone();
        let account = vec![later.clone(), first.clone(), "copilot-search-a".to_string()];
        let listed = catalog_models("github-copilot", &none, Some(&account));
        let ids: Vec<&str> = listed.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, [first.as_str(), later.as_str()]);
        assert!(listed.iter().all(|model| model.enabled));

        let anthropic = catalog_models("anthropic", &none, Some(&["nope".to_string()]));
        assert_eq!(
            anthropic.len(),
            catalog_models("anthropic", &none, None).len()
        );
    }
}
