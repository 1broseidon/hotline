//! Which of the owner's connected providers can draw, and with what by
//! default. There is never a new key: a provider counts only if the owner
//! has already connected it.

use super::chatgpt::{self, ChatGpt};
use super::xai::{self, Grok, GrokAuth};
use super::{
    Google, ImageGen, ImageSet, ImageSettings, Model, OpenAi, OpenRouter, google, openai,
    openrouter,
};
use crate::contract::{CapabilityModel, CapabilityProvider};
use crate::credentials::CredentialFile;
use crate::session::ProviderAuth;
use crate::vault::Vault;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

/// Price for a model the desk doesn't know: high, because the ledger is a
/// guard and an unknown model could be an expensive one.
const UNKNOWN_USD: f64 = 0.10;

/// The models the desk knows, from the 30 Sep bake-off (BRO-173) and each
/// provider's published limits. Prices are per picture at the quality asked
/// for, rounded up.
fn known(id: &str) -> Option<Model> {
    let (transparent, max_references, quality, price_usd) = match id {
        "openai/gpt-image-2.5-flare" | "gpt-image-2.5-flare" => (true, 16, Some("low"), 0.02),
        "openai/gpt-image-2.5-sunburst" | "gpt-image-2.5-sunburst" => (true, 16, Some("low"), 0.05),
        "openai/gpt-image-1-mini" | "gpt-image-1-mini" => (true, 16, Some("medium"), 0.02),
        "openai/gpt-image-2" | "gpt-image-2" => (false, 16, Some("low"), 0.05),
        "google/gemini-3.1-flash-image" | "gemini-3.1-flash-image" => (false, 8, None, 0.08),
        "google/gemini-3.1-flash-lite-image" | "gemini-3.1-flash-lite-image" => {
            (false, 8, None, 0.05)
        }
        "google/gemini-3-pro-image" | "gemini-3-pro-image" => (false, 8, None, 0.20),
        "sourceful/riverflow-v2.5-fast" => (true, 4, None, 0.03),
        "x-ai/grok-imagine-image-2.0" => (false, 4, Some("low"), 0.05),
        "grok-imagine-image-2.0" => (false, 5, None, 0.04),
        "grok-imagine-image-quality" => (false, 5, None, 0.05),
        "grok-imagine-image" => (false, 1, None, 0.02),
        _ => return None,
    };
    Some(Model {
        id: id.to_string(),
        transparent,
        max_references,
        quality,
        price_usd,
    })
}

/// A model by id, known or not. One the desk doesn't know is sent no
/// quality and no transparency, may take a few references (the provider
/// refuses if it can't), and is priced high.
pub fn model(id: &str) -> Model {
    known(id).unwrap_or_else(|| Model {
        id: id.to_string(),
        transparent: false,
        max_references: 4,
        quality: None,
        price_usd: UNKNOWN_USD,
    })
}

struct Row {
    provider_id: &'static str,
    name: &'static str,
    base_url: &'static str,
    /// What it draws with unless the owner names another model.
    draws: &'static str,
    /// A second model on the same provider, for when it's the only one
    /// connected that can draw.
    also: Option<&'static str>,
    /// Every model it is known to draw with, the default first, for the
    /// owner to choose from.
    models: &'static [&'static str],
}

const ROWS: &[Row] = &[
    Row {
        provider_id: openrouter::PROVIDER_ID,
        name: "OpenRouter",
        base_url: openrouter::BASE_URL,
        draws: "openai/gpt-image-2.5-flare",
        also: Some("google/gemini-3.1-flash-image"),
        models: &[
            "openai/gpt-image-2.5-flare",
            "openai/gpt-image-2.5-sunburst",
            "openai/gpt-image-1-mini",
            "openai/gpt-image-2",
            "google/gemini-3.1-flash-image",
            "google/gemini-3.1-flash-lite-image",
            "google/gemini-3-pro-image",
            "sourceful/riverflow-v2.5-fast",
            "x-ai/grok-imagine-image-2.0",
        ],
    },
    Row {
        provider_id: openai::PROVIDER_ID,
        name: "OpenAI",
        base_url: openai::BASE_URL,
        draws: "gpt-image-2.5-flare",
        also: Some("gpt-image-1-mini"),
        models: &[
            "gpt-image-2.5-flare",
            "gpt-image-2.5-sunburst",
            "gpt-image-1-mini",
            "gpt-image-2",
        ],
    },
    Row {
        provider_id: google::PROVIDER_ID,
        name: "Google",
        base_url: google::BASE_URL,
        draws: "gemini-3.1-flash-image",
        also: Some("gemini-3.1-flash-lite-image"),
        models: &[
            "gemini-3.1-flash-image",
            "gemini-3.1-flash-lite-image",
            "gemini-3-pro-image",
        ],
    },
    Row {
        provider_id: xai::PROVIDER_ID,
        name: "Grok",
        base_url: xai::BASE_URL,
        draws: "grok-imagine-image-2.0",
        also: Some("grok-imagine-image"),
        models: &[
            "grok-imagine-image-2.0",
            "grok-imagine-image-quality",
            "grok-imagine-image",
        ],
    },
];

fn row(provider_id: &str) -> Option<&'static Row> {
    ROWS.iter().find(|row| row.provider_id == provider_id)
}

/// A provider the owner has connected, with what it takes to call it.
#[derive(Clone)]
enum ConnectionAuth {
    Key(Option<String>),
    ChatGpt(PathBuf),
    Grok(CredentialFile),
}

#[derive(Clone)]
struct Connection {
    provider_id: String,
    name: String,
    base_url: String,
    auth: ConnectionAuth,
    /// A custom connection's model ids, which are all we know of what it serves.
    models: Vec<String>,
}

impl Connection {
    /// Draws on a subscription's limits rather than the spend ledger.
    fn subscription(&self) -> bool {
        matches!(
            self.auth,
            ConnectionAuth::ChatGpt(_) | ConnectionAuth::Grok(_)
        )
    }

    /// What this connection draws with: the named model, else its default.
    /// A custom connection draws only if it lists or is given an image model.
    fn draws(&self, named: Option<&str>) -> Option<Model> {
        if matches!(self.auth, ConnectionAuth::ChatGpt(_)) {
            return named.is_none_or(|id| id == chatgpt::MODEL).then(|| Model {
                id: chatgpt::MODEL.into(),
                transparent: true,
                max_references: 5,
                quality: Some("auto"),
                price_usd: 0.0,
            });
        }
        if let Some(named) = named {
            return Some(model(named));
        }
        match row(&self.provider_id) {
            Some(row) => Some(model(row.draws)),
            None => self
                .models
                .iter()
                .find(|id| id.to_ascii_lowercase().contains("image"))
                .map(|id| model(id)),
        }
    }

    fn also(&self, primary: &Model) -> Option<Model> {
        let row = row(&self.provider_id)?;
        [row.draws, row.also?]
            .into_iter()
            .find(|id| *id != primary.id)
            .map(model)
    }

    fn adapter(&self, model: Model) -> Result<Arc<dyn ImageGen>, String> {
        let key = match &self.auth {
            ConnectionAuth::ChatGpt(token_dir) => {
                return Ok(Arc::new(ChatGpt::new(token_dir.clone())?));
            }
            ConnectionAuth::Grok(tokens) => {
                return Ok(Arc::new(Grok::new(
                    &self.base_url,
                    GrokAuth::Subscription(tokens.clone()),
                    model,
                )?));
            }
            ConnectionAuth::Key(key) => key.as_deref(),
        };
        Ok(match self.provider_id.as_str() {
            openrouter::PROVIDER_ID => Arc::new(OpenRouter::new(
                &self.base_url,
                key.unwrap_or_default(),
                model,
            )?),
            google::PROVIDER_ID => {
                Arc::new(Google::new(&self.base_url, key.unwrap_or_default(), model)?)
            }
            xai::PROVIDER_ID => Arc::new(Grok::new(
                &self.base_url,
                GrokAuth::Key(key.unwrap_or_default().to_string()),
                model,
            )?),
            // OpenAI and any custom connection that serves the OpenAI shape.
            _ => Arc::new(OpenAi::new(&self.provider_id, &self.base_url, key, model)?),
        })
    }
}

/// The connected providers in the order the owner connected them, one each.
fn connections(vault: &Vault) -> Vec<Connection> {
    let auth = vault.provider_auth();
    let mut seen = HashSet::new();
    vault
        .list()
        .into_iter()
        .filter(|credential| !credential.revoked)
        .filter_map(|credential| {
            let auth = auth.get(&credential.provider_id)?;
            seen.insert(credential.provider_id.clone())
                .then(|| connection(&credential.provider_id, auth))
                .flatten()
        })
        .collect()
}

fn connection(provider_id: &str, auth: &ProviderAuth) -> Option<Connection> {
    if provider_id == chatgpt::PROVIDER_ID {
        let ProviderAuth::Login { token_dir } = auth else {
            return None;
        };
        return Some(Connection {
            provider_id: provider_id.into(),
            name: "Codex (ChatGPT subscription)".into(),
            base_url: String::new(),
            auth: ConnectionAuth::ChatGpt(token_dir.clone()),
            models: Vec::new(),
        });
    }
    if let Some(row) = row(provider_id) {
        // Grok's sign-in is a subscription: its bearer draws on the plan.
        if let ProviderAuth::StoredLogin { tokens } = auth
            && provider_id == xai::PROVIDER_ID
        {
            return Some(Connection {
                provider_id: provider_id.into(),
                name: "Grok (subscription)".into(),
                base_url: row.base_url.into(),
                auth: ConnectionAuth::Grok(tokens.clone()),
                models: Vec::new(),
            });
        }
        let key = match auth {
            ProviderAuth::ApiKey(key) => key.clone(),
            // OpenRouter's sign-in is a PKCE exchange that leaves a plain key.
            ProviderAuth::StoredLogin { tokens } if provider_id == openrouter::PROVIDER_ID => {
                crate::providers::openrouter_key(tokens).ok()?
            }
            _ => return None,
        };
        return Some(Connection {
            provider_id: provider_id.to_string(),
            name: row.name.to_string(),
            base_url: row.base_url.to_string(),
            auth: ConnectionAuth::Key(Some(key)),
            models: Vec::new(),
        });
    }
    match auth {
        ProviderAuth::Custom {
            name,
            base_url,
            api_key,
            config,
        } => Some(Connection {
            provider_id: provider_id.to_string(),
            name: name.clone(),
            base_url: base_url.clone(),
            auth: ConnectionAuth::Key(api_key.clone()),
            models: config.models.clone(),
        }),
        _ => None,
    }
}

/// The models each connected provider can draw with, for the owner to pick
/// from: the ones the desk knows for a provider it knows, a custom
/// connection's own image models. A provider with none is left out.
pub fn options(vault: &Vault) -> Vec<CapabilityProvider> {
    options_from(&connections(vault))
}

fn options_from(connections: &[Connection]) -> Vec<CapabilityProvider> {
    connections
        .iter()
        .filter_map(|connection| {
            let ids: Vec<String> = if matches!(connection.auth, ConnectionAuth::ChatGpt(_)) {
                vec![chatgpt::MODEL.into()]
            } else {
                match row(&connection.provider_id) {
                    Some(row) => row.models.iter().map(|id| id.to_string()).collect(),
                    None => connection
                        .models
                        .iter()
                        .filter(|id| id.to_ascii_lowercase().contains("image"))
                        .cloned()
                        .collect(),
                }
            };
            (!ids.is_empty()).then(|| CapabilityProvider {
                provider_id: connection.provider_id.clone(),
                provider_name: connection.name.clone(),
                models: ids
                    .into_iter()
                    .map(|id| CapabilityModel {
                        id,
                        label: None,
                        voices: None,
                        efforts: None,
                    })
                    .collect(),
            })
        })
        .collect()
}

/// The owner's choice, or else the first connected provider that can draw,
/// a subscription before a paid key: a picture the plan already covers
/// costs nothing more. A choice naming a provider that isn't connected, or
/// can't draw, is an error rather than a quiet switch: the words would go
/// somewhere the owner didn't pick. When nothing can draw, the error is a
/// sentence for a person.
///
/// The fallback is the next connected provider that can draw, or, when only
/// one can, its second model: so one model failing isn't the end of it. A
/// subscription falls back only to a subscription, never to a paid API.
pub fn resolve(vault: &Vault, settings: &ImageSettings) -> Result<ImageSet, String> {
    resolve_from(&connections(vault), settings)
}

/// Which provider and model would draw, for a status line: `None` when
/// nothing can, with the reason.
pub fn describe(vault: &Vault, settings: &ImageSettings) -> Result<super::ImageId, String> {
    resolve(vault, settings).map(|set| set.primary.id())
}

pub(crate) const NOTHING_DRAWS: &str =
    "Connect OpenRouter, OpenAI, Google, Grok or a ChatGPT subscription to make images.";

/// The connections in the order they were connected, subscriptions first.
fn subscriptions_first(connections: &[Connection]) -> impl Iterator<Item = &Connection> {
    let (plans, paid): (Vec<_>, Vec<_>) = connections
        .iter()
        .partition(|connection| connection.subscription());
    plans.into_iter().chain(paid)
}

fn resolve_from(connections: &[Connection], settings: &ImageSettings) -> Result<ImageSet, String> {
    let (from, model) = match &settings.provider {
        Some(provider_id) => {
            let connection = connections
                .iter()
                .find(|connection| &connection.provider_id == provider_id)
                .ok_or_else(|| {
                    format!("Images are set to use {provider_id}, which isn't connected.")
                })?;
            let model = connection.draws(settings.model.as_deref()).ok_or_else(|| {
                format!(
                    "{} can't make images. Choose another provider for images.",
                    connection.name
                )
            })?;
            (connection, model)
        }
        // Model ids are the provider's own, so a model without a provider
        // means nothing yet: the first provider draws with its default.
        None => subscriptions_first(connections)
            .find_map(|connection| Some((connection, connection.draws(None)?)))
            .ok_or_else(|| NOTHING_DRAWS.to_string())?,
    };
    let fallback = subscriptions_first(connections)
        .filter(|connection| connection.provider_id != from.provider_id)
        .filter(|connection| connection.subscription() || !from.subscription())
        .find_map(|connection| Some((connection, connection.draws(None)?)))
        .or_else(|| Some((from, from.also(&model)?)));
    Ok(ImageSet {
        primary: from.adapter(model)?,
        fallback: fallback
            .map(|(connection, model)| connection.adapter(model))
            .transpose()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed(provider_id: &str) -> Connection {
        let row = row(provider_id).unwrap();
        Connection {
            provider_id: provider_id.into(),
            name: row.name.into(),
            base_url: row.base_url.into(),
            auth: ConnectionAuth::Key(Some("k".into())),
            models: Vec::new(),
        }
    }

    fn custom(models: &[&str]) -> Connection {
        Connection {
            provider_id: "custom-abc".into(),
            name: "My gateway".into(),
            base_url: "http://127.0.0.1:1/v1".into(),
            auth: ConnectionAuth::Key(None),
            models: models.iter().map(|m| m.to_string()).collect(),
        }
    }

    fn ids(set: &ImageSet) -> (String, Option<String>) {
        (
            set.primary.id().to_string(),
            set.fallback.as_ref().map(|f| f.id().to_string()),
        )
    }

    fn chatgpt_login() -> Connection {
        connection(
            chatgpt::PROVIDER_ID,
            &ProviderAuth::Login {
                token_dir: PathBuf::from("unused-test-login"),
            },
        )
        .unwrap()
    }

    fn grok_login(dir: &std::path::Path) -> Connection {
        let tokens = crate::credentials::CredentialFiles::new(
            dir.into(),
            crate::credentials::default_store(),
        )
        .file(dir.join("auth.json"));
        connection(xai::PROVIDER_ID, &ProviderAuth::StoredLogin { tokens }).unwrap()
    }

    #[test]
    fn a_subscription_draws_first_and_never_falls_back_to_a_paid_api() {
        let connected = [keyed("openai"), chatgpt_login()];
        let automatic = resolve_from(&connected, &ImageSettings::default()).unwrap();
        assert_eq!(
            automatic.primary.id().to_string(),
            "openai-codex/gpt-image-2"
        );
        assert!(automatic.primary.subscription());
        assert!(automatic.fallback.is_none());
        assert_eq!(
            automatic
                .primary
                .estimate_usd(&super::super::ImageRequest::default()),
            0.0
        );

        // A paid choice may still fall back to the subscription.
        let paid = ImageSettings {
            provider: Some("openai".into()),
            model: None,
        };
        let set = resolve_from(&connected, &paid).unwrap();
        assert_eq!(set.primary.id().provider_id, "openai");
        assert_eq!(set.fallback.unwrap().id().provider_id, chatgpt::PROVIDER_ID);

        assert!(
            resolve_from(
                &connected,
                &ImageSettings {
                    provider: Some(chatgpt::PROVIDER_ID.into()),
                    model: Some("unsupported-image".into()),
                }
            )
            .is_err()
        );
        assert!(
            connection(
                chatgpt::PROVIDER_ID,
                &ProviderAuth::ApiKey("not-a-login".into())
            )
            .is_none()
        );
    }

    #[test]
    fn one_subscription_falls_back_to_another() {
        let dir = tempfile::tempdir().unwrap();
        let connected = [keyed("openrouter"), chatgpt_login(), grok_login(dir.path())];
        let set = resolve_from(&connected, &ImageSettings::default()).unwrap();
        assert_eq!(
            ids(&set),
            (
                "openai-codex/gpt-image-2".into(),
                Some("xai/grok-imagine-image-2.0".into())
            )
        );
        assert!(set.fallback.unwrap().subscription());
    }

    #[test]
    fn grok_draws_with_a_key_or_on_its_subscription() {
        let dir = tempfile::tempdir().unwrap();
        let keyed = keyed("xai");
        let set = resolve_from(std::slice::from_ref(&keyed), &ImageSettings::default()).unwrap();
        assert_eq!(
            ids(&set),
            (
                "xai/grok-imagine-image-2.0".into(),
                Some("xai/grok-imagine-image".into())
            )
        );
        assert!(!set.primary.subscription());
        assert!(
            set.primary
                .estimate_usd(&super::super::ImageRequest::default())
                > 0.0
        );

        let login = grok_login(dir.path());
        assert_eq!(login.name, "Grok (subscription)");
        let set = resolve_from(std::slice::from_ref(&login), &ImageSettings::default()).unwrap();
        assert!(set.primary.subscription());
        assert_eq!(set.primary.max_references(), 5);
        assert_eq!(
            set.fallback.unwrap().id().to_string(),
            "xai/grok-imagine-image"
        );
        let offered = options_from(&[login]);
        assert_eq!(offered[0].provider_id, "xai");
        assert_eq!(offered[0].models[0].id, "grok-imagine-image-2.0");
    }

    #[test]
    fn the_first_connected_provider_draws_and_the_next_is_the_fallback() {
        let set = resolve_from(
            &[keyed("openrouter"), keyed("google")],
            &ImageSettings::default(),
        )
        .unwrap();
        assert_eq!(
            ids(&set),
            (
                "openrouter/openai/gpt-image-2.5-flare".into(),
                Some("google/gemini-3.1-flash-image".into())
            )
        );
        assert!(set.primary.transparent());
    }

    #[test]
    fn a_lone_provider_falls_back_to_its_second_model() {
        let set = resolve_from(&[keyed("openrouter")], &ImageSettings::default()).unwrap();
        assert_eq!(
            ids(&set),
            (
                "openrouter/openai/gpt-image-2.5-flare".into(),
                Some("openrouter/google/gemini-3.1-flash-image".into())
            )
        );
    }

    #[test]
    fn a_custom_connection_draws_only_with_an_image_model() {
        let settings = ImageSettings::default();
        assert!(resolve_from(&[custom(&["llama-3", "whisper"])], &settings).is_err());
        let set = resolve_from(
            &[custom(&["llama-3", "flux-image-dev"]), keyed("openai")],
            &settings,
        )
        .unwrap();
        assert_eq!(set.primary.id().to_string(), "custom-abc/flux-image-dev");
        assert!(!set.primary.transparent());
        assert_eq!(set.primary.max_references(), 4);
    }

    #[test]
    fn the_owners_choice_wins_and_a_bad_one_is_an_error_not_a_switch() {
        let connected = [keyed("openrouter"), keyed("openai")];
        let pick = |provider: &str, model: Option<&str>| ImageSettings {
            provider: Some(provider.into()),
            model: model.map(str::to_string),
        };
        let set = resolve_from(&connected, &pick("openai", Some("gpt-image-1-mini"))).unwrap();
        assert_eq!(set.primary.id().to_string(), "openai/gpt-image-1-mini");
        assert_eq!(set.fallback.unwrap().id().provider_id, "openrouter");
        assert!(
            resolve_from(&connected, &pick("google", None))
                .err()
                .unwrap()
                .contains("isn't connected")
        );
        assert!(
            resolve_from(&[custom(&["llama"])], &pick("custom-abc", None))
                .err()
                .unwrap()
                .contains("can't make images")
        );
    }

    #[test]
    fn every_row_offers_its_default_first_and_its_fallback() {
        for row in ROWS {
            assert_eq!(row.models[0], row.draws, "{}", row.name);
            assert!(row.models.contains(&row.also.unwrap()), "{}", row.name);
            assert!(row.models.iter().all(|id| known(id).is_some()));
        }
    }

    #[test]
    fn the_options_are_the_connected_providers_that_draw() {
        let offered = options_from(&[
            keyed("openai"),
            custom(&["llama-3", "my-image-1"]),
            custom(&["llama-3"]),
        ]);
        assert_eq!(offered.len(), 2);
        assert_eq!(offered[0].provider_name, "OpenAI");
        assert_eq!(offered[0].models[0].id, "gpt-image-2.5-flare");
        assert_eq!(offered[1].provider_name, "My gateway");
        assert_eq!(offered[1].models.len(), 1);
        assert_eq!(offered[1].models[0].id, "my-image-1");
        assert!(options_from(&[custom(&["llama-3"])]).is_empty());
    }

    #[test]
    fn chatgpt_is_offered_with_its_one_model() {
        let login = connection(
            chatgpt::PROVIDER_ID,
            &ProviderAuth::Login {
                token_dir: PathBuf::from("unused-test-login"),
            },
        )
        .unwrap();
        let offered = options_from(std::slice::from_ref(&login));
        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0].provider_id, chatgpt::PROVIDER_ID);
        assert_eq!(offered[0].provider_name, "Codex (ChatGPT subscription)");
        assert_eq!(offered[0].models.len(), 1);
        assert_eq!(offered[0].models[0].id, chatgpt::MODEL);
        assert!(resolve_from(&[login], &ImageSettings::default()).is_ok());
    }

    #[test]
    fn nothing_connected_that_draws_says_what_to_connect() {
        assert_eq!(
            resolve_from(&[], &ImageSettings::default()).err().unwrap(),
            NOTHING_DRAWS
        );
    }

    #[test]
    fn an_unknown_model_is_priced_high_and_sent_nothing_it_might_refuse() {
        let unknown = model("someone/new-image-model");
        assert_eq!(unknown.price_usd, UNKNOWN_USD);
        assert!(!unknown.transparent && unknown.quality.is_none());
        assert!(known("openai/gpt-image-2.5-flare").unwrap().transparent);
        assert!(!known("gpt-image-2").unwrap().transparent);
    }
}
