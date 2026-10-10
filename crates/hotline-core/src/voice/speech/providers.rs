//! Which of the owner's connected providers can hear and speak, and with what
//! by default. There is never a new key: a provider counts only if the owner
//! has already connected it. The desk's own models (`local.rs`) hear beside
//! them once the owner has downloaded one.

use super::catalog::{self, Found};
use super::google::{self, Google};
use super::local::{self, Installed, Local};
use super::xai::{self, Xai};
use super::{AudioFormat, Endpoint, OpenAiShape, Speech, SpeechOutput, SpeechSet, TurnClock};
use crate::contract::{CapabilityModel, CapabilityPick, CapabilityProvider};
use crate::credentials::CredentialFile;
use crate::session::ProviderAuth;
use crate::vault::Vault;
use crate::voice::settings::{Choice, VoiceSettings};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A voice and the model that speaks it.
#[derive(Clone)]
struct Speaking {
    model: String,
    voice: String,
    format: AudioFormat,
    /// Some voices take only so many characters a request.
    max_chars: usize,
}

struct Row {
    provider_id: &'static str,
    name: &'static str,
    /// The root the OpenAI-shaped `/audio` routes hang from; for Google, the API host.
    base_url: &'static str,
    /// The default model to listen with.
    listen: &'static str,
    /// The default model, voice and format to speak with, if the provider speaks
    /// in a shape we can ask for.
    speak: Option<(&'static str, &'static str, AudioFormat, usize)>,
    /// The voices the owner can pick to speak with, the default first.
    voices: &'static [&'static str],
}

/// The one table of what each provider hears and says by default, chosen for
/// a short first sound. xAI has its own `/v1/stt` and `/v1/tts` adapter.
/// Mistral's voice answers with base64 in JSON, so Mistral only listens.
const ROWS: &[Row] = &[
    Row {
        provider_id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        listen: "gpt-4o-mini-transcribe",
        speak: Some(("gpt-4o-mini-tts", "marin", AudioFormat::Wav, 4096)),
        voices: &["marin", "cedar", "alloy", "ash", "coral", "sage", "verse"],
    },
    Row {
        provider_id: google::PROVIDER_ID,
        name: "Google",
        base_url: google::BASE_URL,
        listen: "gemini-3.5-flash-lite",
        speak: Some(("gemini-3.8-flash-tts", "Sulafat", AudioFormat::Wav, 4096)),
        voices: &["Sulafat", "Kore", "Puck", "Charon", "Aoede"],
    },
    Row {
        provider_id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        listen: "openai/whisper-large-v3-turbo",
        speak: Some(("x-ai/grok-voice-tts-1.0", "eve", AudioFormat::Mp3, 4096)),
        voices: &["eve", "ara", "rex", "sal", "leo"],
    },
    Row {
        provider_id: "groq",
        name: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        listen: "whisper-large-v3-turbo",
        // Orpheus takes 200 characters a request.
        speak: Some((
            "canopylabs/orpheus-v1-english",
            "hannah",
            AudioFormat::Wav,
            200,
        )),
        voices: &["hannah", "autumn", "diana", "troy", "austin", "daniel"],
    },
    Row {
        provider_id: "mistral",
        name: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        listen: "voxtral-mini-latest",
        speak: None,
        voices: &[],
    },
    Row {
        provider_id: xai::PROVIDER_ID,
        name: "xAI",
        base_url: xai::BASE_URL,
        listen: xai::LISTEN_MODEL,
        speak: Some((xai::SPEAK_MODEL, "eve", AudioFormat::Wav, 15_000)),
        voices: &["eve", "ara", "rex", "sal", "leo"],
    },
    Row {
        provider_id: xai::SUBSCRIPTION_PROVIDER_ID,
        name: "Grok subscription",
        base_url: xai::BASE_URL,
        listen: xai::LISTEN_MODEL,
        speak: Some((xai::SPEAK_MODEL, "eve", AudioFormat::Wav, 15_000)),
        voices: &["eve", "ara", "rex", "sal", "leo"],
    },
];

fn row(provider_id: &str) -> Option<&'static Row> {
    ROWS.iter().find(|row| row.provider_id == provider_id)
}

/// A provider the owner has connected, with what it takes to call it.
#[derive(Clone)]
struct Connection {
    provider_id: String,
    name: String,
    base_url: String,
    key: Option<String>,
    login: Option<CredentialFile>,
    /// A custom connection's model ids, which are all we know of what it serves.
    models: Vec<String>,
    /// The room's data root, where what the provider offers is cached.
    root: Option<PathBuf>,
}

impl Connection {
    fn custom(&self) -> bool {
        crate::models::is_custom(&self.provider_id)
    }

    /// The custom connection's first model whose id says it does this job.
    fn model_named(&self, words: &[&str]) -> Option<String> {
        self.models
            .iter()
            .find(|model| {
                let model = model.to_ascii_lowercase();
                words.iter().any(|word| model.contains(word))
            })
            .cloned()
    }

    fn endpoint(&self) -> Endpoint {
        Endpoint {
            provider_id: self.provider_id.clone(),
            base_url: self.base_url.clone(),
            key: self.key.clone(),
        }
    }
}

/// The model this connection listens with: the owner's if they named one, else
/// the provider's default. A custom connection with neither has no ears.
fn listening(connection: &Connection, pick: Option<&Choice>) -> Option<String> {
    let named = pick.and_then(|pick| pick.model_id.clone());
    match row(&connection.provider_id) {
        Some(row) => Some(named.unwrap_or_else(|| row.listen.to_string())),
        None if connection.custom() => {
            named.or_else(|| connection.model_named(&["whisper", "transcribe"]))
        }
        None => None,
    }
}

/// The voice this connection speaks with, the owner's changes laid over its
/// default. A custom connection with neither has no voice.
fn speaking(connection: &Connection, pick: Option<&Choice>) -> Option<Speaking> {
    let named_model = pick.and_then(|pick| pick.model_id.clone());
    // The native TTS route chooses a voice, not a model. Never report a
    // different model as selected when the provider cannot honor that pick.
    if matches!(
        connection.provider_id.as_str(),
        xai::PROVIDER_ID | xai::SUBSCRIPTION_PROVIDER_ID
    ) && named_model
        .as_deref()
        .is_some_and(|model| model != xai::SPEAK_MODEL)
    {
        return None;
    }
    let named_voice = pick.and_then(|pick| pick.voice.clone());
    let default = match row(&connection.provider_id) {
        Some(row) => {
            let (model, voice, format, max_chars) = row.speak?;
            Some(Speaking {
                model: model.to_string(),
                voice: voice.to_string(),
                format,
                max_chars,
            })
        }
        None if connection.custom() => {
            // A server we know nothing of gets the most widely served format
            // and the voice every OpenAI-shaped server has.
            let model = named_model
                .clone()
                .or_else(|| connection.model_named(&["tts", "speech"]))?;
            Some(Speaking {
                model,
                voice: "alloy".to_string(),
                format: AudioFormat::Mp3,
                max_chars: 4096,
            })
        }
        None => None,
    }?;
    // A model the owner picked that is not the default may not know the
    // default's voice, so it speaks in its own first voice unless told.
    let voice = named_voice.or_else(|| {
        let model = named_model
            .as_deref()
            .filter(|model| *model != default.model)?;
        catalog::default_voice(connection.root.as_deref(), &connection.provider_id, model)
    });
    let model = named_model.unwrap_or(default.model);
    Some(Speaking {
        format: format_for(&connection.provider_id, &model, default.format),
        model,
        voice: voice.unwrap_or(default.voice),
        ..default
    })
}

/// The format a model will actually answer in. Gemini's voices through
/// OpenRouter refuse everything but raw PCM; the rest take the provider's.
fn format_for(provider_id: &str, model: &str, default: AudioFormat) -> AudioFormat {
    if provider_id == "openrouter" && model.starts_with("google/") {
        AudioFormat::Pcm
    } else {
        default
    }
}

/// What the owner can pick for hearing and for speaking, and what each
/// resolves to when they pick nothing.
pub struct Options {
    pub stt: Vec<CapabilityProvider>,
    pub tts: Vec<CapabilityProvider>,
    pub automatic_stt: Option<CapabilityPick>,
    pub automatic_tts: Option<CapabilityPick>,
}

/// Every connected provider's models for each speech job, from the same
/// connections `resolve` reads. A built-in provider is asked what it offers
/// (or answers from a day-old copy); if it cannot be asked and nothing is
/// cached, it offers its default. A custom connection offers the models its
/// own ids say do the job, because those ids are all we know.
pub async fn options(vault: &Vault) -> Options {
    let connections = connections(vault);
    let endpoints: Vec<_> = connections
        .iter()
        .filter(|connection| connection.login.is_none() && row(&connection.provider_id).is_some())
        .map(Connection::endpoint)
        .collect();
    let found: HashMap<String, Found> = catalog::discover_all(Some(vault.root()), &endpoints)
        .await
        .into_iter()
        .collect();
    options_from(&local::installed(vault.root()), &connections, &found)
}

/// The default first, so the picker leads with what automatic would use.
fn default_first<T>(mut models: Vec<T>, default: &str, id: impl Fn(&T) -> &str) -> Vec<T> {
    if let Some(at) = models.iter().position(|model| id(model) == default) {
        let model = models.remove(at);
        models.insert(0, model);
    }
    models
}

fn options_from(
    local: &[Installed],
    connections: &[Connection],
    found: &HashMap<String, Found>,
) -> Options {
    let provider = |connection: &Connection, models: Vec<CapabilityModel>| CapabilityProvider {
        provider_id: connection.provider_id.clone(),
        provider_name: connection.name.clone(),
        models,
    };
    let plain = |id: String| CapabilityModel {
        id,
        label: None,
        voices: None,
        efforts: None,
    };
    let mut stt = Vec::new();
    let mut tts = Vec::new();
    if !local.is_empty() {
        stt.push(CapabilityProvider {
            provider_id: local::PROVIDER_ID.into(),
            provider_name: local::PROVIDER_NAME.into(),
            models: local
                .iter()
                .map(|model| CapabilityModel {
                    id: model.id.clone(),
                    label: Some(model.name.clone()),
                    voices: None,
                    efforts: None,
                })
                .collect(),
        });
    }
    for connection in connections {
        let offered = found.get(&connection.provider_id);
        let hears: Vec<CapabilityModel> = match row(&connection.provider_id) {
            Some(row) => {
                let models: Vec<CapabilityModel> = match offered {
                    Some(found) if !found.listen.is_empty() => found
                        .listen
                        .iter()
                        .map(|model| CapabilityModel {
                            id: model.id.clone(),
                            label: model.label.clone(),
                            voices: None,
                            efforts: None,
                        })
                        .collect(),
                    _ => vec![plain(row.listen.to_string())],
                };
                default_first(models, row.listen, |model| &model.id)
            }
            None => connection
                .models
                .iter()
                .filter(|id| {
                    let id = id.to_ascii_lowercase();
                    ["whisper", "transcribe"]
                        .iter()
                        .any(|word| id.contains(word))
                })
                .cloned()
                .map(plain)
                .collect(),
        };
        if !hears.is_empty() {
            stt.push(provider(connection, hears));
        }
        let speaks: Vec<CapabilityModel> = match row(&connection.provider_id) {
            Some(Row {
                speak: Some((default, _, _, _)),
                voices,
                ..
            }) => {
                let models: Vec<CapabilityModel> = match offered {
                    Some(found) if !found.speak.is_empty() => found
                        .speak
                        .iter()
                        .map(|model| CapabilityModel {
                            id: model.id.clone(),
                            label: model.label.clone(),
                            voices: Some(model.voices.clone()),
                            efforts: None,
                        })
                        .collect(),
                    _ => vec![CapabilityModel {
                        id: default.to_string(),
                        label: None,
                        voices: Some(voices.iter().map(|voice| voice.to_string()).collect()),
                        efforts: None,
                    }],
                };
                default_first(models, default, |model| &model.id)
            }
            Some(_) => Vec::new(),
            None => connection
                .models
                .iter()
                .filter(|id| {
                    let id = id.to_ascii_lowercase();
                    ["tts", "speech"].iter().any(|word| id.contains(word))
                })
                .cloned()
                .map(plain)
                .collect(),
        };
        if !speaks.is_empty() {
            tts.push(provider(connection, speaks));
        }
    }
    let automatic_local = local.first().map(|model| CapabilityPick {
        provider_id: local::PROVIDER_ID.into(),
        provider_name: local::PROVIDER_NAME.into(),
        model_id: Some(model.id.clone()),
        voice: None,
        effort: None,
    });
    Options {
        stt,
        tts,
        automatic_stt: automatic_local.or_else(|| {
            connections.iter().find_map(|connection| {
                if connection.login.is_some() {
                    return None;
                }
                let model = listening(connection, None)?;
                Some(CapabilityPick {
                    provider_id: connection.provider_id.clone(),
                    provider_name: connection.name.clone(),
                    model_id: Some(model),
                    voice: None,
                    effort: None,
                })
            })
        }),
        automatic_tts: connections.iter().find_map(|connection| {
            if connection.login.is_some() {
                return None;
            }
            let voice = speaking(connection, None)?;
            Some(CapabilityPick {
                provider_id: connection.provider_id.clone(),
                provider_name: connection.name.clone(),
                model_id: Some(voice.model),
                voice: Some(voice.voice),
                effort: None,
            })
        }),
    }
}

/// The connected providers in the order the owner connected them, one each.
fn connections(vault: &Vault) -> Vec<Connection> {
    let auth = vault.provider_auth();
    let root = vault.root();
    let mut seen = HashSet::new();
    vault
        .list()
        .into_iter()
        .filter(|credential| !credential.revoked)
        .filter_map(|credential| {
            let auth = auth.get(&credential.provider_id)?;
            seen.insert(credential.provider_id.clone())
                .then(|| connection(&credential.provider_id, auth, root))
                .flatten()
        })
        .collect()
}

fn connection(provider_id: &str, auth: &ProviderAuth, root: &Path) -> Option<Connection> {
    // This row is derived only from the real xAI stored login, never a key
    // someone saved under the subscription's display/configuration id.
    if provider_id == xai::SUBSCRIPTION_PROVIDER_ID {
        return None;
    }
    if provider_id == xai::PROVIDER_ID
        && let ProviderAuth::StoredLogin { tokens } = auth
    {
        let row = row(xai::SUBSCRIPTION_PROVIDER_ID)?;
        return Some(Connection {
            provider_id: row.provider_id.into(),
            name: row.name.into(),
            base_url: row.base_url.into(),
            key: None,
            login: Some(tokens.clone()),
            models: Vec::new(),
            root: Some(root.to_path_buf()),
        });
    }
    if let Some(row) = row(provider_id) {
        let key = match auth {
            ProviderAuth::ApiKey(key) => key.clone(),
            // OpenRouter's sign-in is a PKCE exchange that leaves a plain key.
            ProviderAuth::StoredLogin { tokens } if provider_id == "openrouter" => {
                crate::providers::openrouter_key(tokens).ok()?
            }
            _ => return None,
        };
        return Some(Connection {
            provider_id: provider_id.to_string(),
            name: row.name.to_string(),
            base_url: row.base_url.to_string(),
            key: Some(key),
            login: None,
            models: Vec::new(),
            root: Some(root.to_path_buf()),
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
            key: api_key.clone(),
            login: None,
            models: config.models.clone(),
            root: None,
        }),
        _ => None,
    }
}

/// STT, TTS and a fallback TTS from what the owner has connected.
///
/// Each job goes to the first connected provider that can do it, unless
/// `settings.voice` names another. A named provider that is not connected, or
/// cannot do the job, is an error rather than a quiet switch to a provider the
/// owner did not choose, because the audio would go there. The error is a
/// sentence for a person.
pub fn resolve(vault: &Vault, settings: &VoiceSettings) -> Result<SpeechSet, String> {
    resolve_from(
        &local::installed(vault.root()),
        &connections(vault),
        settings,
    )
}

/// What the desk's own model is asked to listen for when it hears with
/// `settings`, for a clip heard outside a call.
pub fn listens_for(vault: &Vault, settings: &VoiceSettings) -> Vec<String> {
    words_for(
        &local::installed(vault.root()),
        &connections(vault),
        settings,
    )
}

/// The words a small model would otherwise spell as the nearest common
/// word, most wanted first, since only so many are kept (`local::hotwords`):
/// the person's own, teammates' names, the desk's name and its models', and
/// the providers it is connected to.
fn words_for(
    local: &[Installed],
    connections: &[Connection],
    settings: &VoiceSettings,
) -> Vec<String> {
    let desk = ["Hotline", "Parakeet"].map(String::from);
    let models = local.iter().map(|model| model.name.clone());
    let providers = connections.iter().map(|connection| connection.name.clone());
    settings
        .listen_for
        .iter()
        .chain(&settings.teammates)
        .cloned()
        .chain(desk)
        .chain(models)
        .chain(providers)
        .collect()
}

/// A text call needs a voice, with no speech-input provider or credential.
pub fn resolve_output(vault: &Vault, settings: &VoiceSettings) -> Result<SpeechOutput, String> {
    output_from(&connections(vault), settings, &TurnClock::default())
}

fn chosen_voice<'a>(
    connections: &'a [Connection],
    settings: &VoiceSettings,
) -> Result<Option<(&'a Connection, Speaking)>, String> {
    match &settings.tts {
        Some(pick) => {
            let connection = named(connections, pick)?;
            let voice =
                speaking(connection, Some(pick)).ok_or_else(|| cannot(connection, "speak"))?;
            Ok(Some((connection, voice)))
        }
        None => Ok(connections
            .iter()
            .filter(|connection| connection.login.is_none())
            .find_map(|connection| Some((connection, speaking(connection, None)?)))),
    }
}

fn output_from(
    connections: &[Connection],
    settings: &VoiceSettings,
    clock: &TurnClock,
) -> Result<SpeechOutput, String> {
    let Some((connection, voice)) = chosen_voice(connections, settings)? else {
        return Err(nothing_can(true, false));
    };
    output_for(connections, settings, connection, &voice, clock)
}

fn output_for(
    connections: &[Connection],
    settings: &VoiceSettings,
    speak_from: &Connection,
    voice: &Speaking,
    clock: &TurnClock,
) -> Result<SpeechOutput, String> {
    // A fallback that cannot be used never prevents a call from starting.
    let fallback = if speak_from.login.is_some() {
        // Choosing the subscription never authorizes a paid API fallback.
        None
    } else {
        match &settings.fallback_tts {
            Some(pick) => named(connections, pick)
                .ok()
                .filter(|connection| connection.login.is_none())
                .and_then(|connection| Some((connection, speaking(connection, Some(pick))?))),
            None => connections
                .iter()
                .filter(|connection| connection.provider_id != speak_from.provider_id)
                .filter(|connection| connection.login.is_none())
                .find_map(|connection| Some((connection, speaking(connection, None)?))),
        }
    };
    Ok(SpeechOutput {
        tts: speaker(speak_from, voice, clock)?,
        fallback_tts: fallback
            .map(|(connection, voice)| speaker(connection, &voice, clock))
            .transpose()?,
    })
}

/// What hears: a model installed on the desk, or a connected provider's.
enum Hearing<'a> {
    Local(&'a Installed),
    Provider(&'a Connection, String),
}

/// The owner's pick for hearing, or else the first that can: an installed
/// model before any provider, because it is free, nothing said leaves the
/// machine, and installing it was the owner's own act.
fn hearing<'a>(
    local: &'a [Installed],
    connections: &'a [Connection],
    settings: &VoiceSettings,
) -> Result<Option<Hearing<'a>>, String> {
    Ok(match &settings.stt {
        Some(pick) if pick.provider_id == local::PROVIDER_ID => {
            let model = match &pick.model_id {
                Some(id) => local.iter().find(|model| model.id == *id),
                None => local.first(),
            };
            Some(Hearing::Local(model.ok_or(
                "Voice is set to hear with a speech model on the desk that is not installed. Download it in Settings, or pick another.",
            )?))
        }
        Some(pick) => {
            let connection = named(connections, pick)?;
            let model = listening(connection, Some(pick))
                .ok_or_else(|| cannot(connection, "turn speech into text"))?;
            Some(Hearing::Provider(connection, model))
        }
        None => local.first().map(Hearing::Local).or_else(|| {
            connections
                .iter()
                .filter(|connection| connection.login.is_none())
                .find_map(|connection| {
                    Some(Hearing::Provider(connection, listening(connection, None)?))
                })
        }),
    })
}

fn resolve_from(
    local: &[Installed],
    connections: &[Connection],
    settings: &VoiceSettings,
) -> Result<SpeechSet, String> {
    let hearing = hearing(local, connections, settings)?;
    let voice = chosen_voice(connections, settings)?;
    let (Some(hear_from), Some((speak_from, voice))) = (&hearing, &voice) else {
        return Err(nothing_can(hearing.is_some(), voice.is_some()));
    };

    // One stopwatch for the set: the ears start it and the voices stop it.
    let clock = TurnClock::default();
    let output = output_for(connections, settings, speak_from, voice, &clock)?;
    let stt: Arc<dyn Speech> = match hear_from {
        Hearing::Local(model) => {
            let words = words_for(local, connections, settings);
            Arc::new(
                Local::new((*model).clone())
                    .listening_for(&words)
                    .with_clock(&clock),
            )
        }
        Hearing::Provider(connection, model) => listener(connection, model, &clock)?,
    };
    Ok(SpeechSet {
        stt,
        tts: output.tts,
        fallback_tts: output.fallback_tts,
    })
}

fn listener(
    connection: &Connection,
    model: &str,
    clock: &TurnClock,
) -> Result<Arc<dyn Speech>, String> {
    if matches!(
        connection.provider_id.as_str(),
        xai::PROVIDER_ID | xai::SUBSCRIPTION_PROVIDER_ID
    ) {
        let adapter = Xai::listener(connection.endpoint(), model)?.with_clock(clock);
        return Ok(Arc::new(match &connection.login {
            Some(tokens) => adapter.with_subscription(tokens.clone())?,
            None => adapter,
        }));
    }
    if connection.provider_id == google::PROVIDER_ID {
        let key = connection.key.as_deref().unwrap_or_default();
        return Ok(Arc::new(
            Google::listener(&connection.base_url, key, model)?.with_clock(clock),
        ));
    }
    Ok(Arc::new(
        OpenAiShape::listener(connection.endpoint(), model)?.with_clock(clock),
    ))
}

fn speaker(
    connection: &Connection,
    voice: &Speaking,
    clock: &TurnClock,
) -> Result<Arc<dyn Speech>, String> {
    if matches!(
        connection.provider_id.as_str(),
        xai::PROVIDER_ID | xai::SUBSCRIPTION_PROVIDER_ID
    ) {
        let adapter =
            Xai::speaker(connection.endpoint(), &voice.model, &voice.voice)?.with_clock(clock);
        return Ok(Arc::new(match &connection.login {
            Some(tokens) => adapter.with_subscription(tokens.clone())?,
            None => adapter,
        }));
    }
    if connection.provider_id == google::PROVIDER_ID {
        let key = connection.key.as_deref().unwrap_or_default();
        return Ok(Arc::new(
            Google::speaker(&connection.base_url, key, &voice.model, &voice.voice)?
                .with_clock(clock),
        ));
    }
    Ok(Arc::new(
        OpenAiShape::speaker(
            connection.endpoint(),
            &voice.model,
            &voice.voice,
            voice.format,
            voice.max_chars,
        )?
        .with_clock(clock),
    ))
}

fn named<'a>(connections: &'a [Connection], pick: &Choice) -> Result<&'a Connection, String> {
    connections
        .iter()
        .find(|connection| connection.provider_id == pick.provider_id)
        .ok_or_else(|| {
            let name = row(&pick.provider_id).map_or(pick.provider_id.as_str(), |row| row.name);
            format!(
                "Voice is set to use {name}, which is not connected. Connect it in Settings, or clear that choice."
            )
        })
}

fn cannot(connection: &Connection, job: &str) -> String {
    format!(
        "Voice is set to use {} to {job}, and it cannot. Pick another provider in Settings.",
        connection.name
    )
}

/// The sentence when a call cannot start for want of a provider.
fn nothing_can(hears: bool, speaks: bool) -> String {
    match (hears, speaks) {
        (false, false) => "None of your connected providers can hear or speak yet. Connect OpenAI, Google, Groq, OpenRouter or xAI in Settings and voice will use it, with no new key.".into(),
        (false, true) => "Nothing can turn speech into text yet. Download a speech model for the desk in Settings, or connect OpenAI, Google, Groq, OpenRouter, xAI or Mistral.".into(),
        _ => "Your connected providers can hear but not speak. Connect OpenAI, Google, Groq, OpenRouter or xAI in Settings.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::speech::SpeechId;

    fn connected(provider_id: &str) -> Connection {
        Connection {
            provider_id: provider_id.to_string(),
            name: row(provider_id)
                .map_or("Custom", |row| row.name)
                .to_string(),
            base_url: row(provider_id)
                .map_or("http://localhost:9000/v1", |row| row.base_url)
                .to_string(),
            key: Some("test-key".to_string()),
            login: None,
            models: Vec::new(),
            root: None,
        }
    }

    fn custom(models: &[&str]) -> Connection {
        Connection {
            provider_id: format!("custom-{}", uuid::Uuid::new_v4()),
            name: "Cloudflare".to_string(),
            base_url: "https://gateway.example/v1/openai".to_string(),
            key: None,
            models: models.iter().map(|model| model.to_string()).collect(),
            login: None,
            root: None,
        }
    }

    fn ids(set: &SpeechSet) -> (SpeechId, SpeechId, Option<SpeechId>) {
        (
            set.stt.id(),
            set.tts.id(),
            set.fallback_tts.as_ref().map(|fallback| fallback.id()),
        )
    }

    fn id(provider: &str, model: &str, voice: Option<&str>) -> SpeechId {
        SpeechId {
            provider_id: provider.into(),
            model_id: model.into(),
            voice: voice.map(str::to_string),
        }
    }

    fn pick(provider: &str, model: Option<&str>, voice: Option<&str>) -> Choice {
        Choice {
            provider_id: provider.into(),
            model_id: model.map(str::to_string),
            voice: voice.map(str::to_string),
            effort: None,
        }
    }

    #[test]
    fn the_first_connected_provider_speaks_and_the_next_is_the_fallback() {
        let set = resolve_from(
            &[],
            &[connected("groq"), connected("openai"), connected("google")],
            &VoiceSettings::default(),
        )
        .unwrap();
        assert_eq!(
            ids(&set),
            (
                id("groq", "whisper-large-v3-turbo", None),
                id("groq", "canopylabs/orpheus-v1-english", Some("hannah")),
                Some(id("openai", "gpt-4o-mini-tts", Some("marin"))),
            )
        );
    }

    #[test]
    fn a_provider_that_only_listens_hears_and_the_next_one_speaks() {
        let set = resolve_from(
            &[],
            &[connected("mistral"), connected("google")],
            &VoiceSettings::default(),
        )
        .unwrap();
        assert_eq!(
            ids(&set),
            (
                id("mistral", "voxtral-mini-latest", None),
                id("google", "gemini-3.8-flash-tts", Some("Sulafat")),
                None,
            )
        );
    }

    #[test]
    fn one_connected_provider_has_no_fallback() {
        let set = resolve_from(&[], &[connected("openai")], &VoiceSettings::default()).unwrap();
        assert!(set.fallback_tts.is_none());
        assert_eq!(set.stt.id().model_id, "gpt-4o-mini-transcribe");
    }

    #[test]
    fn native_xai_listens_live_and_speaks_wav_with_the_connected_api_key() {
        let set = resolve_from(&[], &[connected("xai")], &VoiceSettings::default()).unwrap();
        assert_eq!(
            ids(&set),
            (
                id("xai", xai::LISTEN_MODEL, None),
                id("xai", xai::SPEAK_MODEL, Some("eve")),
                None,
            )
        );
        assert!(set.stt.supports_live_input());
        assert!(!set.tts.supports_live_input());
        assert_eq!(set.tts.output_mime(), "audio/wav");
    }

    #[test]
    fn native_xai_speech_does_not_treat_subscription_login_as_a_paid_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let auth = ProviderAuth::Login {
            token_dir: dir.path().into(),
        };
        assert!(connection("xai", &auth, dir.path()).is_none());
        assert!(connection("xai", &ProviderAuth::ApiKey("test-key".into()), dir.path()).is_some());
        assert!(
            connection(
                xai::SUBSCRIPTION_PROVIDER_ID,
                &ProviderAuth::ApiKey("test-key".into()),
                dir.path()
            )
            .is_none()
        );
    }

    fn subscription(root: &Path) -> Connection {
        let tokens = crate::credentials::CredentialFiles::new(
            root.into(),
            crate::credentials::default_store(),
        )
        .file(root.join("auth.json"));
        connection("xai", &ProviderAuth::StoredLogin { tokens }, root).unwrap()
    }

    #[test]
    fn subscription_voice_is_offered_but_requires_an_explicit_choice() {
        let dir = tempfile::tempdir().unwrap();
        let connections = [subscription(dir.path())];
        let options = options_from(&[], &connections, &HashMap::new());
        assert_eq!(options.stt[0].provider_id, xai::SUBSCRIPTION_PROVIDER_ID);
        assert_eq!(options.tts[0].provider_name, "Grok subscription");
        assert!(options.automatic_stt.is_none());
        assert!(options.automatic_tts.is_none());
        assert!(resolve_from(&[], &connections, &VoiceSettings::default()).is_err());
        assert!(
            output_from(
                &connections,
                &VoiceSettings::default(),
                &TurnClock::default()
            )
            .is_err()
        );
        let settings = VoiceSettings {
            stt: Some(pick(xai::SUBSCRIPTION_PROVIDER_ID, None, None)),
            tts: Some(pick(xai::SUBSCRIPTION_PROVIDER_ID, None, Some("ara"))),
            ..VoiceSettings::default()
        };
        let set = resolve_from(&[], &connections, &settings).unwrap();
        assert!(set.stt.is_subscription());
        assert!(set.stt.supports_live_input());
        assert!(set.tts.is_subscription());
        assert_eq!(set.tts.id().voice.as_deref(), Some("ara"));
        assert!(set.fallback_tts.is_none());
    }

    #[test]
    fn explicit_subscription_never_uses_a_paid_fallback_or_changes_automatic() {
        let dir = tempfile::tempdir().unwrap();
        let connections = [subscription(dir.path()), connected("openai")];
        let options = options_from(&[], &connections, &HashMap::new());
        assert_eq!(options.automatic_tts.unwrap().provider_id, "openai");
        let automatic = resolve_from(&[], &connections, &VoiceSettings::default()).unwrap();
        assert_eq!(automatic.tts.id().provider_id, "openai");
        assert!(automatic.fallback_tts.is_none());
        let settings = VoiceSettings {
            tts: Some(pick(xai::SUBSCRIPTION_PROVIDER_ID, None, None)),
            fallback_tts: Some(pick("openai", None, None)),
            // A text call must not consult a selected STT provider at all.
            stt: Some(pick("disconnected", None, None)),
            ..VoiceSettings::default()
        };
        let output = output_from(&connections, &settings, &TurnClock::default()).unwrap();
        assert!(output.tts.is_subscription());
        assert!(output.fallback_tts.is_none());
        assert!(resolve_from(&[], &connections, &settings).is_err());
    }

    #[test]
    fn native_xai_tts_never_claims_an_unsupported_model_pick() {
        let settings = VoiceSettings {
            tts: Some(pick("xai", Some("another-model"), None)),
            ..VoiceSettings::default()
        };
        assert!(resolve_from(&[], &[connected("xai")], &settings).is_err());
    }

    #[test]
    fn the_owner_can_name_a_provider_and_change_its_model_and_voice() {
        let settings = VoiceSettings {
            stt: Some(pick("google", None, None)),
            tts: Some(pick("openai", Some("tts-1-hd"), Some("nova"))),
            fallback_tts: Some(pick("groq", None, Some("troy"))),
            ..VoiceSettings::default()
        };
        let set = resolve_from(
            &[],
            &[connected("openai"), connected("google"), connected("groq")],
            &settings,
        )
        .unwrap();
        assert_eq!(
            ids(&set),
            (
                id("google", "gemini-3.5-flash-lite", None),
                id("openai", "tts-1-hd", Some("nova")),
                Some(id("groq", "canopylabs/orpheus-v1-english", Some("troy"))),
            )
        );
    }

    #[test]
    fn a_named_provider_that_is_not_connected_is_an_error_not_a_switch() {
        let settings = VoiceSettings {
            tts: Some(pick("groq", None, None)),
            ..VoiceSettings::default()
        };
        let error = match resolve_from(&[], &[connected("openai")], &settings) {
            Ok(_) => panic!("resolved to something else"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            "Voice is set to use Groq, which is not connected. Connect it in Settings, or clear that choice."
        );
        let settings = VoiceSettings {
            tts: Some(pick("mistral", None, None)),
            ..VoiceSettings::default()
        };
        let error = match resolve_from(&[], &[connected("openai"), connected("mistral")], &settings)
        {
            Ok(_) => panic!("mistral cannot speak"),
            Err(error) => error,
        };
        assert!(error.contains("Mistral to speak, and it cannot"), "{error}");
    }

    #[test]
    fn a_fallback_the_owner_named_but_cannot_be_used_is_no_fallback() {
        let settings = VoiceSettings {
            fallback_tts: Some(pick("groq", None, None)),
            ..VoiceSettings::default()
        };
        let set = resolve_from(&[], &[connected("openai")], &settings).unwrap();
        assert!(set.fallback_tts.is_none());
    }

    #[test]
    fn nothing_connected_says_so_in_a_sentence() {
        let error = match resolve_from(&[], &[], &VoiceSettings::default()) {
            Ok(_) => panic!("resolved with nothing connected"),
            Err(error) => error,
        };
        assert!(error.starts_with("None of your connected providers can hear or speak yet."));
        let error = match resolve_from(&[], &[connected("mistral")], &VoiceSettings::default()) {
            Ok(_) => panic!("resolved with no voice"),
            Err(error) => error,
        };
        assert!(error.starts_with("Your connected providers can hear but not speak."));
        let error = match resolve_from(
            &[],
            &[custom(&["some-chat-model"])],
            &VoiceSettings::default(),
        ) {
            Ok(_) => panic!("resolved with a chat-only custom connection"),
            Err(error) => error,
        };
        assert!(error.starts_with("None of your connected providers can hear or speak yet."));
    }

    #[test]
    fn a_custom_connection_counts_only_for_the_speech_models_it_lists() {
        let cloudflare = custom(&["llama-chat", "whisper-large-v3-turbo", "aura-tts"]);
        let set = resolve_from(
            &[],
            std::slice::from_ref(&cloudflare),
            &VoiceSettings::default(),
        )
        .unwrap();
        assert_eq!(
            ids(&set),
            (
                id(&cloudflare.provider_id, "whisper-large-v3-turbo", None),
                id(&cloudflare.provider_id, "aura-tts", Some("alloy")),
                None,
            )
        );
    }

    #[test]
    fn a_custom_connection_the_owner_names_speaks_the_model_they_give() {
        let cloudflare = custom(&["llama-chat"]);
        let settings = VoiceSettings {
            stt: Some(pick(&cloudflare.provider_id, Some("whisper-1"), None)),
            tts: Some(pick(&cloudflare.provider_id, Some("tts-1"), Some("echo"))),
            ..VoiceSettings::default()
        };
        let set = resolve_from(&[], std::slice::from_ref(&cloudflare), &settings).unwrap();
        assert_eq!(set.stt.id(), id(&cloudflare.provider_id, "whisper-1", None));
        assert_eq!(
            set.tts.id(),
            id(&cloudflare.provider_id, "tts-1", Some("echo"))
        );
    }

    // The vault half: which credentials count, and in what order.

    fn vault_in(root: &std::path::Path) -> Vault {
        let log = crate::log::Log::open(root);
        Vault::open_with_store(
            root,
            log,
            Arc::new(crate::credentials::tests::MemoryStore::default()),
        )
        .unwrap()
    }

    #[test]
    fn the_vault_gives_providers_in_the_order_they_were_connected() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault_in(root.path());
        vault.create("anthropic", "Claude", "sk-ant").unwrap();
        vault.create("groq", "Groq", "gsk-first").unwrap();
        let openai = vault.create("openai", "OpenAI", "sk-second").unwrap();
        vault.create("google", "Gemini", "AIza-third").unwrap();

        let set = resolve(&vault, &VoiceSettings::default()).unwrap();
        assert_eq!(set.stt.id().provider_id, "groq");
        assert_eq!(set.tts.id().provider_id, "groq");
        assert_eq!(
            set.fallback_tts.as_ref().unwrap().id().provider_id,
            "openai"
        );

        // A revoked key is not a connection.
        vault.revoke(&openai.id).unwrap();
        let set = resolve(&vault, &VoiceSettings::default()).unwrap();
        assert_eq!(
            set.fallback_tts.as_ref().unwrap().id().provider_id,
            "google"
        );
    }

    #[test]
    fn every_voice_list_starts_with_the_voice_the_provider_speaks_with() {
        for row in ROWS {
            match row.speak {
                Some((_, voice, ..)) => assert_eq!(row.voices.first(), Some(&voice)),
                None => assert!(row.voices.is_empty()),
            }
        }
    }

    #[test]
    fn the_options_are_what_the_connected_providers_can_do() {
        let options = options_from(
            &[],
            &[
                connected("mistral"),
                connected("openai"),
                custom(&["llama-3", "whisper-1", "my-tts"]),
            ],
            &HashMap::new(),
        );
        let names = |providers: &[CapabilityProvider]| {
            providers
                .iter()
                .map(|p| p.provider_name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&options.stt), ["Mistral", "OpenAI", "Cloudflare"]);
        assert_eq!(names(&options.tts), ["OpenAI", "Cloudflare"]);
        assert_eq!(options.stt[2].models[0].id, "whisper-1");
        let openai = &options.tts[0].models[0];
        assert_eq!(openai.id, "gpt-4o-mini-tts");
        assert_eq!(openai.voices.as_ref().unwrap()[0], "marin");
        let stt = options.automatic_stt.unwrap();
        assert_eq!(
            (stt.provider_name.as_str(), stt.model_id.as_deref()),
            ("Mistral", Some("voxtral-mini-latest"))
        );
        let tts = options.automatic_tts.unwrap();
        assert_eq!(
            (
                tts.provider_id.as_str(),
                tts.model_id.as_deref(),
                tts.voice.as_deref()
            ),
            ("openai", Some("gpt-4o-mini-tts"), Some("marin"))
        );
        let none = options_from(&[], &[], &HashMap::new());
        assert!(none.stt.is_empty() && none.tts.is_empty());
        assert!(none.automatic_stt.is_none() && none.automatic_tts.is_none());
    }

    #[test]
    fn the_desks_model_listens_for_the_persons_words_then_teammates_then_its_own() {
        let settings = VoiceSettings {
            listen_for: vec!["Ophelia".into(), "Groq".into()],
            teammates: vec!["Mack".into(), "Brix".into(), "ophelia".into()],
            ..VoiceSettings::default()
        };
        let models = [local::listed("parakeet-tdt-110m-en", "Parakeet English")];
        let connections = [connected("openai"), connected("groq")];
        assert_eq!(
            words_for(&models, &connections, &settings),
            [
                "Ophelia",
                "Groq",
                "Mack",
                "Brix",
                "ophelia",
                "Hotline",
                "Parakeet",
                "Parakeet English",
                connections[0].name.as_str(),
                connections[1].name.as_str(),
            ]
        );
        // With nothing said, nobody on the team and nothing connected, the desk's own words remain.
        assert_eq!(
            words_for(&[], &[], &VoiceSettings::default()),
            ["Hotline", "Parakeet"]
        );
    }

    #[test]
    fn an_installed_model_hears_first_and_a_connected_provider_still_speaks() {
        let models = [
            local::listed("parakeet-tdt-0.6b-v3", "Parakeet"),
            local::listed("parakeet-tdt-110m-en", "Parakeet English"),
        ];
        let set = resolve_from(&models, &[connected("openai")], &VoiceSettings::default()).unwrap();
        assert_eq!(
            ids(&set),
            (
                id(local::PROVIDER_ID, "parakeet-tdt-0.6b-v3", None),
                id("openai", "gpt-4o-mini-tts", Some("marin")),
                None,
            )
        );
        assert!(set.stt.supports_live_input());

        let options = options_from(&models, &[connected("openai")], &HashMap::new());
        assert_eq!(options.stt[0].provider_name, local::PROVIDER_NAME);
        assert_eq!(
            options.stt[0]
                .models
                .iter()
                .map(|model| (model.id.as_str(), model.label.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("parakeet-tdt-0.6b-v3", Some("Parakeet")),
                ("parakeet-tdt-110m-en", Some("Parakeet English"))
            ]
        );
        assert_eq!(options.stt[1].provider_id, "openai");
        let automatic = options.automatic_stt.unwrap();
        assert_eq!(
            (
                automatic.provider_id.as_str(),
                automatic.model_id.as_deref()
            ),
            (local::PROVIDER_ID, Some("parakeet-tdt-0.6b-v3"))
        );

        // The owner can still pick a provider, or the smaller model.
        let settings = VoiceSettings {
            stt: Some(pick("openai", None, None)),
            ..VoiceSettings::default()
        };
        let set = resolve_from(&models, &[connected("openai")], &settings).unwrap();
        assert_eq!(set.stt.id().provider_id, "openai");
        let settings = VoiceSettings {
            stt: Some(pick(local::PROVIDER_ID, Some("parakeet-tdt-110m-en"), None)),
            ..VoiceSettings::default()
        };
        let set = resolve_from(&models, &[connected("openai")], &settings).unwrap();
        assert_eq!(set.stt.id().model_id, "parakeet-tdt-110m-en");
    }

    #[test]
    fn nothing_installed_offers_no_desk_model_and_a_pick_of_one_is_an_error_not_a_switch() {
        let options = options_from(&[], &[connected("openai")], &HashMap::new());
        assert!(
            options
                .stt
                .iter()
                .all(|one| one.provider_id != local::PROVIDER_ID)
        );
        assert_eq!(options.automatic_stt.unwrap().provider_id, "openai");

        let settings = VoiceSettings {
            stt: Some(pick(local::PROVIDER_ID, Some("parakeet-tdt-0.6b-v3"), None)),
            ..VoiceSettings::default()
        };
        let installed_other = [local::listed("parakeet-tdt-110m-en", "Parakeet English")];
        for models in [&[][..], &installed_other[..]] {
            let error = match resolve_from(models, &[connected("openai")], &settings) {
                Ok(_) => panic!("heard with something the owner did not pick"),
                Err(error) => error,
            };
            assert!(error.contains("not installed"), "{error}");
        }
        // Hearing on the desk with nothing to speak says what to connect.
        let error = match resolve_from(&installed_other, &[], &VoiceSettings::default()) {
            Ok(_) => panic!("spoke with nothing connected"),
            Err(error) => error,
        };
        assert!(
            error.starts_with("Your connected providers can hear but not speak."),
            "{error}"
        );
    }

    #[test]
    fn a_vault_with_only_chat_providers_cannot_speak() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault_in(root.path());
        vault.create("anthropic", "Claude", "sk-ant").unwrap();
        assert!(resolve(&vault, &VoiceSettings::default()).is_err());
    }

    #[test]
    fn a_vault_with_native_xai_can_hear_live_and_speak() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault_in(root.path());
        vault.create("xai", "Grok", "xai-key").unwrap();
        let set = resolve(&vault, &VoiceSettings::default()).unwrap();
        assert!(set.stt.supports_live_input());
        assert_eq!(set.tts.id().provider_id, "xai");
    }

    fn found(listen: &[&str], speak: &[(&str, &[&str])]) -> Found {
        Found {
            fetched_at: 0,
            listen: listen
                .iter()
                .map(|id| catalog::Heard {
                    id: id.to_string(),
                    label: None,
                })
                .collect(),
            speak: speak
                .iter()
                .map(|(id, voices)| catalog::Spoken {
                    id: id.to_string(),
                    label: Some(format!("Label of {id}")),
                    voices: voices.iter().map(|voice| voice.to_string()).collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn discovered_models_fill_the_options_with_the_default_first() {
        let discovered = HashMap::from([(
            "openrouter".to_string(),
            found(
                &["deepgram/nova-3", "openai/whisper-large-v3-turbo"],
                &[
                    ("hexgrad/kokoro-82m", &["af_heart", "af_bella"]),
                    ("x-ai/grok-voice-tts-1.0", &["eve", "ara"]),
                ],
            ),
        )]);
        // Groq is connected too but nothing was discovered for it.
        let options = options_from(
            &[],
            &[connected("openrouter"), connected("groq")],
            &discovered,
        );
        let stt: Vec<_> = options.stt[0]
            .models
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(stt, ["openai/whisper-large-v3-turbo", "deepgram/nova-3"]);
        let tts = &options.tts[0].models;
        assert_eq!(tts[0].id, "x-ai/grok-voice-tts-1.0");
        assert_eq!(
            tts[0].label.as_deref(),
            Some("Label of x-ai/grok-voice-tts-1.0")
        );
        assert_eq!(tts[1].voices.as_ref().unwrap(), &["af_heart", "af_bella"]);
        assert_eq!(options.stt[1].models[0].id, "whisper-large-v3-turbo");
        assert_eq!(options.tts[1].models[0].id, "canopylabs/orpheus-v1-english");
    }

    #[test]
    fn only_connected_providers_are_offered_even_if_more_were_discovered() {
        let discovered = HashMap::from([
            (
                "openai".to_string(),
                found(&["whisper-1"], &[("tts-1", &["alloy"])]),
            ),
            ("groq".to_string(), found(&["whisper-large-v3"], &[])),
        ]);
        let options = options_from(&[], &[connected("groq")], &discovered);
        assert_eq!(options.stt.len(), 1);
        assert_eq!(options.stt[0].provider_id, "groq");
        assert!(options.tts[0].provider_id == "groq");
    }

    #[test]
    fn a_picked_openrouter_model_and_voice_is_what_speaks() {
        let settings = VoiceSettings {
            stt: Some(pick("openrouter", Some("deepgram/nova-3"), None)),
            tts: Some(pick(
                "openrouter",
                Some("hexgrad/kokoro-82m"),
                Some("af_bella"),
            )),
            ..VoiceSettings::default()
        };
        let set = resolve_from(&[], &[connected("openrouter")], &settings).unwrap();
        assert_eq!(
            ids(&set),
            (
                id("openrouter", "deepgram/nova-3", None),
                id("openrouter", "hexgrad/kokoro-82m", Some("af_bella")),
                None,
            )
        );
        assert_eq!(set.tts.output_mime(), "audio/mpeg");
    }

    #[test]
    fn a_picked_model_without_a_voice_speaks_in_its_own_first_voice() {
        let root = tempfile::tempdir().unwrap();
        catalog::write(
            root.path(),
            "openrouter",
            &found(&[], &[("hexgrad/kokoro-82m", &["af_heart"])]),
        );
        let mut openrouter = connected("openrouter");
        openrouter.root = Some(root.path().to_path_buf());
        let settings = VoiceSettings {
            tts: Some(pick("openrouter", Some("hexgrad/kokoro-82m"), None)),
            ..VoiceSettings::default()
        };
        let set = resolve_from(&[], &[openrouter], &settings).unwrap();
        assert_eq!(
            set.tts.id(),
            id("openrouter", "hexgrad/kokoro-82m", Some("af_heart"))
        );
        // Groq's Arabic model cannot speak the English default voice.
        let settings = VoiceSettings {
            tts: Some(pick("groq", Some("canopylabs/orpheus-arabic-saudi"), None)),
            ..VoiceSettings::default()
        };
        let set = resolve_from(&[], &[connected("groq")], &settings).unwrap();
        assert_eq!(set.tts.id().voice.as_deref(), Some("abdullah"));
    }
}
