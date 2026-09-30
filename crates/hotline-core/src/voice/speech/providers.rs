//! Which of the owner's connected providers can hear and speak, and with what
//! by default. There is never a new key: a provider counts only if the owner
//! has already connected it.

use super::google::{self, Google};
use super::{AudioFormat, Endpoint, OpenAiShape, Speech, SpeechSet};
use crate::session::ProviderAuth;
use crate::vault::Vault;
use crate::voice::settings::{Choice, VoiceSettings};
use std::collections::HashSet;
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
}

/// The one table of what each provider hears and says by default, chosen for
/// a short first sound. xAI is left out because its speech is `/v1/tts` and
/// `/v1/stt`, not the OpenAI shape (its voice is reachable through OpenRouter),
/// and Mistral's voice answers with base64 in JSON, so Mistral only listens.
const ROWS: &[Row] = &[
    Row {
        provider_id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        listen: "gpt-4o-mini-transcribe",
        speak: Some(("gpt-4o-mini-tts", "marin", AudioFormat::Wav, 4096)),
    },
    Row {
        provider_id: google::PROVIDER_ID,
        name: "Google",
        base_url: google::BASE_URL,
        listen: "gemini-3.5-flash-lite",
        speak: Some(("gemini-3.8-flash-tts", "Sulafat", AudioFormat::Wav, 4096)),
    },
    Row {
        provider_id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        listen: "openai/whisper-large-v3-turbo",
        speak: Some(("x-ai/grok-voice-tts-1.0", "eve", AudioFormat::Mp3, 4096)),
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
    },
    Row {
        provider_id: "mistral",
        name: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        listen: "voxtral-mini-latest",
        speak: None,
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
    /// A custom connection's model ids, which are all we know of what it serves.
    models: Vec<String>,
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
    Some(Speaking {
        model: named_model.unwrap_or(default.model),
        voice: named_voice.unwrap_or(default.voice),
        ..default
    })
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
            key: api_key.clone(),
            models: config.models.clone(),
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
    resolve_from(&connections(vault), settings)
}

fn resolve_from(connections: &[Connection], settings: &VoiceSettings) -> Result<SpeechSet, String> {
    let hearing = match &settings.stt {
        Some(pick) => {
            let connection = named(connections, pick)?;
            let model = listening(connection, Some(pick))
                .ok_or_else(|| cannot(connection, "turn speech into text"))?;
            Some((connection, model))
        }
        None => connections
            .iter()
            .find_map(|connection| Some((connection, listening(connection, None)?))),
    };
    let voice = match &settings.tts {
        Some(pick) => {
            let connection = named(connections, pick)?;
            let voice =
                speaking(connection, Some(pick)).ok_or_else(|| cannot(connection, "speak"))?;
            Some((connection, voice))
        }
        None => connections
            .iter()
            .find_map(|connection| Some((connection, speaking(connection, None)?))),
    };
    let (Some((hear_from, model)), Some((speak_from, voice))) = (&hearing, &voice) else {
        return Err(nothing_can(hearing.is_some(), voice.is_some()));
    };

    // A fallback that cannot be used is no fallback: it is never the reason
    // a call cannot start.
    let fallback = match &settings.fallback_tts {
        Some(pick) => named(connections, pick)
            .ok()
            .and_then(|connection| Some((connection, speaking(connection, Some(pick))?))),
        None => connections
            .iter()
            .filter(|connection| connection.provider_id != speak_from.provider_id)
            .find_map(|connection| Some((connection, speaking(connection, None)?))),
    };

    Ok(SpeechSet {
        stt: listener(hear_from, model)?,
        tts: speaker(speak_from, voice)?,
        fallback_tts: fallback
            .map(|(connection, voice)| speaker(connection, &voice))
            .transpose()?,
    })
}

fn listener(connection: &Connection, model: &str) -> Result<Arc<dyn Speech>, String> {
    if connection.provider_id == google::PROVIDER_ID {
        let key = connection.key.as_deref().unwrap_or_default();
        return Ok(Arc::new(Google::listener(
            &connection.base_url,
            key,
            model,
        )?));
    }
    Ok(Arc::new(OpenAiShape::listener(
        connection.endpoint(),
        model,
    )?))
}

fn speaker(connection: &Connection, voice: &Speaking) -> Result<Arc<dyn Speech>, String> {
    if connection.provider_id == google::PROVIDER_ID {
        let key = connection.key.as_deref().unwrap_or_default();
        return Ok(Arc::new(Google::speaker(
            &connection.base_url,
            key,
            &voice.model,
            &voice.voice,
        )?));
    }
    Ok(Arc::new(OpenAiShape::speaker(
        connection.endpoint(),
        &voice.model,
        &voice.voice,
        voice.format,
        voice.max_chars,
    )?))
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
        (false, false) => "None of your connected providers can hear or speak yet. Connect OpenAI, Google, Groq or OpenRouter in Settings and voice will use it, with no new key.".into(),
        (false, true) => "None of your connected providers can turn speech into text. Connect OpenAI, Google, Groq, OpenRouter or Mistral in Settings.".into(),
        _ => "Your connected providers can hear but not speak. Connect OpenAI, Google, Groq or OpenRouter in Settings.".into(),
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
            models: Vec::new(),
        }
    }

    fn custom(models: &[&str]) -> Connection {
        Connection {
            provider_id: format!("custom-{}", uuid::Uuid::new_v4()),
            name: "Cloudflare".to_string(),
            base_url: "https://gateway.example/v1/openai".to_string(),
            key: None,
            models: models.iter().map(|model| model.to_string()).collect(),
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
        }
    }

    #[test]
    fn the_first_connected_provider_speaks_and_the_next_is_the_fallback() {
        let set = resolve_from(
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
        let set = resolve_from(&[connected("openai")], &VoiceSettings::default()).unwrap();
        assert!(set.fallback_tts.is_none());
        assert_eq!(set.stt.id().model_id, "gpt-4o-mini-transcribe");
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
        let error = match resolve_from(&[connected("openai")], &settings) {
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
        let error = match resolve_from(&[connected("openai"), connected("mistral")], &settings) {
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
        let set = resolve_from(&[connected("openai")], &settings).unwrap();
        assert!(set.fallback_tts.is_none());
    }

    #[test]
    fn nothing_connected_says_so_in_a_sentence() {
        let error = match resolve_from(&[], &VoiceSettings::default()) {
            Ok(_) => panic!("resolved with nothing connected"),
            Err(error) => error,
        };
        assert!(error.starts_with("None of your connected providers can hear or speak yet."));
        let error = match resolve_from(&[connected("mistral")], &VoiceSettings::default()) {
            Ok(_) => panic!("resolved with no voice"),
            Err(error) => error,
        };
        assert!(error.starts_with("Your connected providers can hear but not speak."));
        let error = match resolve_from(&[custom(&["some-chat-model"])], &VoiceSettings::default()) {
            Ok(_) => panic!("resolved with a chat-only custom connection"),
            Err(error) => error,
        };
        assert!(error.starts_with("None of your connected providers can hear or speak yet."));
    }

    #[test]
    fn a_custom_connection_counts_only_for_the_speech_models_it_lists() {
        let cloudflare = custom(&["llama-chat", "whisper-large-v3-turbo", "aura-tts"]);
        let set =
            resolve_from(std::slice::from_ref(&cloudflare), &VoiceSettings::default()).unwrap();
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
        let set = resolve_from(std::slice::from_ref(&cloudflare), &settings).unwrap();
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
    fn a_vault_with_only_chat_providers_cannot_speak() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault_in(root.path());
        vault.create("anthropic", "Claude", "sk-ant").unwrap();
        vault.create("xai", "Grok", "xai-key").unwrap();
        assert!(resolve(&vault, &VoiceSettings::default()).is_err());
    }
}
