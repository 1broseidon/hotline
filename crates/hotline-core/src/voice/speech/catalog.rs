//! The speech models each connected provider offers, asked of the provider
//! itself, so the "Use for" pickers list every model and voice the owner can
//! really use and not one default each.
//!
//! Each provider's list is cached on disk for a day under the room's
//! `cache/` folder, keyed by provider id alone: what is stored is model
//! names and voices, never a key. A fetch that fails falls back to the cached
//! copy, even a stale one, and then to nothing, which the caller fills with
//! the provider's built-in default. Starting a call never comes here to ask:
//! it reads only [`default_voice`], from the cache and the curated lists.

use super::Endpoint;
use crate::paths;
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

const CACHE_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// A model list is a few hundred kilobytes at most.
const LIST_LIMIT: usize = 8 << 20;

/// A model that turns speech into text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heard {
    pub id: String,
    pub label: Option<String>,
}

/// A model that speaks, and the voices it can speak in, the default first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spoken {
    pub id: String,
    pub label: Option<String>,
    pub voices: Vec<String>,
}

/// What one provider offers for speech, and when we asked.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub fetched_at: i64,
    pub listen: Vec<Heard>,
    pub speak: Vec<Spoken>,
}

impl Found {
    fn is_empty(&self) -> bool {
        self.listen.is_empty() && self.speak.is_empty()
    }
}

const OPENAI_VOICES: &[&str] = &[
    "marin", "cedar", "alloy", "ash", "ballad", "coral", "echo", "fable", "nova", "onyx", "sage",
    "shimmer", "verse",
];
/// What `tts-1` and `tts-1-hd` take, which is fewer.
const OPENAI_CLASSIC_VOICES: &[&str] = &[
    "alloy", "ash", "coral", "echo", "fable", "nova", "onyx", "sage", "shimmer",
];
const GOOGLE_VOICES: &[&str] = &[
    "Sulafat",
    "Zephyr",
    "Puck",
    "Charon",
    "Kore",
    "Fenrir",
    "Leda",
    "Orus",
    "Aoede",
    "Callirrhoe",
    "Autonoe",
    "Enceladus",
    "Iapetus",
    "Umbriel",
    "Algieba",
    "Despina",
    "Erinome",
    "Algenib",
    "Rasalgethi",
    "Laomedeia",
    "Achernar",
    "Alnilam",
    "Schedar",
    "Gacrux",
    "Pulcherrima",
    "Achird",
    "Zubenelgenubi",
    "Vindemiatrix",
    "Sadachbia",
    "Sadaltager",
];
const GROQ_ENGLISH_VOICES: &[&str] = &["hannah", "autumn", "diana", "austin", "daniel", "troy"];
const GROQ_ARABIC_VOICES: &[&str] = &["abdullah", "fahad", "sultan", "noura", "lulwa", "aisha"];

/// The voices a provider documents for a model, the default first, for the
/// providers whose list of models does not carry them. OpenRouter's does, and
/// is read from the cache instead.
fn curated_voices(provider_id: &str, model: &str) -> &'static [&'static str] {
    match provider_id {
        "openai" if model.starts_with("tts-1") => OPENAI_CLASSIC_VOICES,
        "openai" => OPENAI_VOICES,
        "google" => GOOGLE_VOICES,
        "groq" if model.contains("arabic") => GROQ_ARABIC_VOICES,
        "groq" => GROQ_ENGLISH_VOICES,
        _ => &[],
    }
}

/// The voice a model speaks in when the owner named the model and not a voice:
/// the first of its documented voices, else the first the provider listed.
pub fn default_voice(root: Option<&Path>, provider_id: &str, model: &str) -> Option<String> {
    if let Some(voice) = curated_voices(provider_id, model).first() {
        return Some(voice.to_string());
    }
    let found = read(root?, provider_id)?;
    found
        .speak
        .iter()
        .find(|spoken| spoken.id == model)
        .and_then(|spoken| spoken.voices.first().cloned())
}

/// Every built-in provider's list, asked all at once. A provider that could
/// not be asked and has nothing cached is simply absent.
pub async fn discover_all(root: Option<&Path>, endpoints: &[Endpoint]) -> Vec<(String, Found)> {
    join_all(endpoints.iter().map(|endpoint| async move {
        let found = discover(root, endpoint).await?;
        Some((endpoint.provider_id.clone(), found))
    }))
    .await
    .into_iter()
    .flatten()
    .collect()
}

/// One provider's list: the cached copy while it is a day old, else a fresh
/// fetch, else whatever the cache still holds.
pub async fn discover(root: Option<&Path>, endpoint: &Endpoint) -> Option<Found> {
    let cached = root.and_then(|root| read(root, &endpoint.provider_id));
    if let Some(cached) = &cached
        && fresh(cached.fetched_at, now_ms())
    {
        return Some(cached.clone());
    }
    match fetch(endpoint).await {
        Some(found) => {
            if let Some(root) = root {
                write(root, &endpoint.provider_id, &found);
            }
            Some(found)
        }
        None => cached,
    }
}

/// A stamp from the future is not fresh, and neither is one that cannot be
/// subtracted: the cache is a file, so the number in it is whatever it is.
fn fresh(fetched_at: i64, now: i64) -> bool {
    (0..CACHE_TTL_MS).contains(&now.saturating_sub(fetched_at))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn read(root: &Path, provider_id: &str) -> Option<Found> {
    let bytes = std::fs::read(paths::speech_models_path(root, provider_id)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(super) fn write(root: &Path, provider_id: &str, found: &Found) {
    let path = paths::speech_models_path(root, provider_id);
    if let Some(directory) = path.parent()
        && std::fs::create_dir_all(directory).is_ok()
        && let Ok(bytes) = serde_json::to_vec(found)
    {
        let _ = std::fs::write(path, bytes);
    }
}

/// Asks the provider. All or nothing: a list that is only half there is not
/// stamped as fresh. A provider with no models of the kinds we can drive
/// counts as a failure, so the built-in default stands.
async fn fetch(endpoint: &Endpoint) -> Option<Found> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let base = endpoint.base_url.trim_end_matches('/');
    let key = endpoint.key.as_deref();
    let (listen, speak) = match endpoint.provider_id.as_str() {
        "openrouter" => {
            // The catalogue is public, so the key is not sent.
            let speech_url = format!("{base}/models?output_modalities=speech");
            let transcription_url = format!("{base}/models?output_modalities=transcription");
            let (speech, transcription) = tokio::join!(
                get(&client, &speech_url, None),
                get(&client, &transcription_url, None),
            );
            (
                parse_openrouter_listen(&transcription?),
                parse_openrouter_speak(&speech?),
            )
        }
        "google" => {
            let body = get(
                &client,
                &format!("{base}/v1beta/models?pageSize=1000"),
                Some(Auth::Google(key?)),
            )
            .await?;
            parse_google(&body)
        }
        provider_id => {
            let body = get(&client, &format!("{base}/models"), key.map(Auth::Bearer)).await?;
            parse_openai_shape(provider_id, &body)
        }
    };
    let found = Found {
        fetched_at: now_ms(),
        listen,
        speak,
    };
    (!found.is_empty()).then_some(found)
}

enum Auth<'a> {
    Bearer(&'a str),
    Google(&'a str),
}

async fn get(client: &reqwest::Client, url: &str, auth: Option<Auth<'_>>) -> Option<Value> {
    let request = client.get(url);
    let request = match auth {
        Some(Auth::Bearer(key)) => request.bearer_auth(key),
        Some(Auth::Google(key)) => request.header("x-goog-api-key", key),
        None => request,
    };
    let response = request.send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    if bytes.len() > LIST_LIMIT {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

/// A model id that names a release on a day, such as `gpt-4o-mini-tts-2025-12-15`,
/// which the undated id already stands for.
fn dated(id: &str) -> bool {
    let tail: Vec<&str> = id.rsplitn(4, '-').collect();
    matches!(tail.as_slice(), [day, month, year, _]
        if day.len() == 2 && month.len() == 2 && year.len() == 4
            && [day, month, year].iter().all(|part| part.bytes().all(|b| b.is_ascii_digit())))
}

fn ids(body: &Value) -> Vec<String> {
    let mut ids: Vec<String> = body["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| model["id"].as_str().map(str::to_string))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// OpenAI, Groq and Mistral list models as `{ data: [{ id }] }` and say what a
/// model does only in its name.
fn parse_openai_shape(provider_id: &str, body: &Value) -> (Vec<Heard>, Vec<Spoken>) {
    let mut listen = Vec::new();
    let mut speak = Vec::new();
    for id in ids(body) {
        let lower = id.to_ascii_lowercase();
        let (hears, speaks) = match provider_id {
            "openai" => (
                !dated(&id)
                    && !lower.contains("diarize")
                    && (lower.contains("transcribe") || lower.starts_with("whisper")),
                !dated(&id) && lower.contains("tts"),
            ),
            "groq" => (lower.contains("whisper"), lower.contains("orpheus")),
            // Mistral's speech-to-text is Voxtral; its speech goes back as
            // base64 in JSON, which the speech adapter does not read.
            "mistral" => (
                lower.contains("voxtral") && !lower.contains("tts") && !lower.contains("realtime"),
                false,
            ),
            _ => (false, false),
        };
        if hears {
            listen.push(Heard { id, label: None });
        } else if speaks {
            let voices = curated_voices(provider_id, &id)
                .iter()
                .map(|voice| voice.to_string())
                .collect();
            speak.push(Spoken {
                id,
                label: None,
                voices,
            });
        }
    }
    (listen, speak)
}

/// OpenRouter names a model "Vendor: Name"; the picker already groups by
/// provider, so the vendor is dropped.
fn openrouter_label(model: &Value) -> Option<String> {
    let name = model["name"].as_str()?;
    let name = name.split_once(": ").map_or(name, |(_, rest)| rest);
    Some(name.to_string())
}

fn parse_openrouter_listen(body: &Value) -> Vec<Heard> {
    body["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| {
            Some(Heard {
                id: model["id"].as_str()?.to_string(),
                label: openrouter_label(model),
            })
        })
        .collect()
}

/// A model that lists no voices is one we cannot ask for a voice, so it is
/// left out rather than offered to fail.
fn parse_openrouter_speak(body: &Value) -> Vec<Spoken> {
    body["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let voices: Vec<String> = model["supported_voices"]
                .as_array()?
                .iter()
                .filter_map(|voice| voice.as_str().map(str::to_string))
                .collect();
            (!voices.is_empty()).then(|| Spoken {
                id: model["id"].as_str().unwrap_or_default().to_string(),
                label: openrouter_label(model),
                voices,
            })
        })
        .filter(|spoken| !spoken.id.is_empty())
        .collect()
}

/// Gemini text models that take audio hear; `-tts` models speak.
fn parse_google(body: &Value) -> (Vec<Heard>, Vec<Spoken>) {
    let mut listen = Vec::new();
    let mut speak = Vec::new();
    for model in body["models"].as_array().into_iter().flatten() {
        let Some(id) = model["name"]
            .as_str()
            .map(|name| name.trim_start_matches("models/").to_string())
        else {
            continue;
        };
        let generates = model["supportedGenerationMethods"]
            .as_array()
            .is_some_and(|methods| methods.iter().any(|method| method == "generateContent"));
        if !generates {
            continue;
        }
        let label = model["displayName"].as_str().map(str::to_string);
        if id.contains("-tts") {
            let voices = curated_voices("google", &id)
                .iter()
                .map(|voice| voice.to_string())
                .collect();
            speak.push(Spoken { id, label, voices });
        } else if id.starts_with("gemini-")
            && id.contains("flash")
            && ![
                "image",
                "live",
                "embedding",
                "lyria",
                "audio",
                "robotics",
                "computer",
            ]
            .iter()
            .any(|word| id.contains(word))
        {
            listen.push(Heard { id, label });
        }
    }
    (listen, speak)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn listed(ids: &[&str]) -> Value {
        json!({ "data": ids.iter().map(|id| json!({ "id": id })).collect::<Vec<_>>() })
    }

    fn heard(listen: &[Heard]) -> Vec<&str> {
        listen.iter().map(|model| model.id.as_str()).collect()
    }

    fn spoken(speak: &[Spoken]) -> Vec<&str> {
        speak.iter().map(|model| model.id.as_str()).collect()
    }

    #[test]
    fn openai_models_are_sorted_by_what_their_names_say_they_do() {
        let body = listed(&[
            "gpt-4o",
            "gpt-4o-mini-tts",
            "gpt-4o-mini-tts-2025-12-15",
            "gpt-4o-transcribe",
            "gpt-4o-transcribe-diarize",
            "gpt-4o-mini-transcribe-2025-12-15",
            "gpt-4o-mini-transcribe",
            "tts-1",
            "tts-1-hd",
            "whisper-1",
            "text-embedding-3-small",
            "gpt-realtime",
        ]);
        let (listen, speak) = parse_openai_shape("openai", &body);
        assert_eq!(
            heard(&listen),
            ["gpt-4o-mini-transcribe", "gpt-4o-transcribe", "whisper-1"]
        );
        assert_eq!(spoken(&speak), ["gpt-4o-mini-tts", "tts-1", "tts-1-hd"]);
        assert_eq!(speak[0].voices[0], "marin");
        assert_eq!(speak[1].voices[0], "alloy");
        assert!(!speak[1].voices.contains(&"marin".to_string()));
    }

    #[test]
    fn groq_has_whisper_to_hear_and_orpheus_to_speak_in_its_own_voices() {
        let body = listed(&[
            "llama-3.3-70b-versatile",
            "whisper-large-v3",
            "whisper-large-v3-turbo",
            "canopylabs/orpheus-v1-english",
            "canopylabs/orpheus-arabic-saudi",
        ]);
        let (listen, speak) = parse_openai_shape("groq", &body);
        assert_eq!(
            heard(&listen),
            ["whisper-large-v3", "whisper-large-v3-turbo"]
        );
        assert_eq!(
            spoken(&speak),
            [
                "canopylabs/orpheus-arabic-saudi",
                "canopylabs/orpheus-v1-english"
            ]
        );
        assert_eq!(speak[0].voices[0], "abdullah");
        assert_eq!(speak[1].voices[0], "hannah");
    }

    #[test]
    fn mistral_only_listens_and_not_to_its_realtime_or_speaking_models() {
        let body = listed(&[
            "mistral-large-latest",
            "voxtral-mini-latest",
            "voxtral-mini-transcribe-realtime-2602",
            "voxtral-mini-tts-2603",
        ]);
        let (listen, speak) = parse_openai_shape("mistral", &body);
        assert_eq!(heard(&listen), ["voxtral-mini-latest"]);
        assert!(speak.is_empty());
    }

    #[test]
    fn openrouter_speakers_bring_their_voices_and_those_without_are_left_out() {
        let body = json!({ "data": [
            { "id": "x-ai/grok-voice-tts-1.0", "name": "xAI: Grok Voice TTS 1.0",
              "supported_voices": ["eve", "ara", "rex"] },
            { "id": "fish-audio/s1", "name": "Fish Audio: S1", "supported_voices": null },
            { "id": "deepgram/aura-2", "name": "Deepgram: Aura 2", "supported_voices": [] },
            { "id": "hexgrad/kokoro-82m", "name": "Kokoro 82M", "supported_voices": ["af_heart"] },
        ]});
        let speak = parse_openrouter_speak(&body);
        assert_eq!(
            spoken(&speak),
            ["x-ai/grok-voice-tts-1.0", "hexgrad/kokoro-82m"]
        );
        assert_eq!(speak[0].label.as_deref(), Some("Grok Voice TTS 1.0"));
        assert_eq!(speak[0].voices, ["eve", "ara", "rex"]);
        assert_eq!(speak[1].label.as_deref(), Some("Kokoro 82M"));
    }

    #[test]
    fn openrouter_listeners_are_every_transcription_model() {
        let body = json!({ "data": [
            { "id": "openai/whisper-large-v3-turbo", "name": "OpenAI: Whisper Large V3 Turbo" },
            { "id": "deepgram/nova-3", "name": "Deepgram: Nova-3" },
        ]});
        let listen = parse_openrouter_listen(&body);
        assert_eq!(
            heard(&listen),
            ["openai/whisper-large-v3-turbo", "deepgram/nova-3"]
        );
        assert_eq!(listen[0].label.as_deref(), Some("Whisper Large V3 Turbo"));
    }

    #[test]
    fn google_hears_with_flash_text_models_and_speaks_with_tts_ones() {
        let generate = json!(["generateContent"]);
        let body = json!({ "models": [
            { "name": "models/gemini-3.5-flash-lite", "displayName": "Gemini 3.5 Flash-Lite",
              "supportedGenerationMethods": generate },
            { "name": "models/gemini-3.5-flash", "supportedGenerationMethods": generate },
            { "name": "models/gemini-3.5-pro", "supportedGenerationMethods": generate },
            { "name": "models/gemini-3.5-flash-image", "supportedGenerationMethods": generate },
            { "name": "models/gemini-live-3.5-flash", "supportedGenerationMethods": generate },
            { "name": "models/gemini-embedding-001", "supportedGenerationMethods": ["embedContent"] },
            { "name": "models/gemini-3.8-flash-tts", "displayName": "Gemini 3.8 Flash TTS",
              "supportedGenerationMethods": generate },
            { "name": "models/lyria-3", "supportedGenerationMethods": generate },
        ]});
        let (listen, speak) = parse_google(&body);
        assert_eq!(
            heard(&listen),
            ["gemini-3.5-flash-lite", "gemini-3.5-flash"]
        );
        assert_eq!(spoken(&speak), ["gemini-3.8-flash-tts"]);
        assert_eq!(speak[0].voices.len(), 30);
        assert_eq!(speak[0].voices[0], "Sulafat");
    }

    #[test]
    fn a_day_is_fresh_and_a_stamp_from_the_future_is_not() {
        assert!(fresh(1_000, 1_000 + CACHE_TTL_MS - 1));
        assert!(!fresh(1_000, 1_000 + CACHE_TTL_MS));
        assert!(!fresh(2_000, 1_000));
    }

    #[test]
    fn a_named_model_without_a_voice_gets_one_the_model_knows() {
        assert_eq!(
            default_voice(None, "groq", "canopylabs/orpheus-arabic-saudi").as_deref(),
            Some("abdullah")
        );
        assert_eq!(default_voice(None, "openrouter", "x/y"), None);
        let root = tempfile::tempdir().unwrap();
        let found = Found {
            fetched_at: now_ms(),
            listen: Vec::new(),
            speak: vec![Spoken {
                id: "hexgrad/kokoro-82m".into(),
                label: None,
                voices: vec!["af_heart".into(), "af_bella".into()],
            }],
        };
        write(root.path(), "openrouter", &found);
        assert_eq!(
            default_voice(Some(root.path()), "openrouter", "hexgrad/kokoro-82m").as_deref(),
            Some("af_heart")
        );
        assert_eq!(read(root.path(), "openrouter"), Some(found));
    }

    use axum::Router;
    use axum::http::{HeaderMap, StatusCode, Uri};
    use std::sync::{Arc, Mutex};

    /// A server that answers `/models` with a fixed list, or fails, and
    /// remembers what it was asked and with which credentials.
    async fn serve(
        status: StatusCode,
        body: Value,
    ) -> (
        String,
        Arc<Mutex<Vec<(String, bool)>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let app = Router::new().fallback(move |uri: Uri, headers: HeaderMap| {
            let (log, body) = (log.clone(), body.clone());
            async move {
                let signed =
                    headers.contains_key("authorization") || headers.contains_key("x-goog-api-key");
                log.lock()
                    .unwrap()
                    .push((uri.path_and_query().unwrap().to_string(), signed));
                (
                    status,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    body.to_string(),
                )
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (url, seen, task)
    }

    fn endpoint(provider_id: &str, base_url: &str) -> Endpoint {
        Endpoint {
            provider_id: provider_id.into(),
            base_url: base_url.into(),
            key: Some("test-key".into()),
        }
    }

    #[tokio::test]
    async fn a_fetch_is_cached_for_a_day_and_the_cache_holds_no_key() {
        let root = tempfile::tempdir().unwrap();
        let (url, seen, server) = serve(
            StatusCode::OK,
            listed(&["whisper-large-v3", "canopylabs/orpheus-v1-english"]),
        )
        .await;
        let groq = endpoint("groq", &url);
        let first = discover(Some(root.path()), &groq).await.unwrap();
        assert_eq!(heard(&first.listen), ["whisper-large-v3"]);
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            [("/models".to_string(), true)]
        );
        // Fresh: answered from disk, the server is not asked again.
        let second = discover(Some(root.path()), &groq).await.unwrap();
        assert_eq!(second, first);
        assert_eq!(seen.lock().unwrap().len(), 1);
        let on_disk =
            std::fs::read_to_string(paths::speech_models_path(root.path(), "groq")).unwrap();
        assert!(!on_disk.contains("test-key"));
        server.abort();
    }

    #[tokio::test]
    async fn a_failed_fetch_falls_back_to_the_stale_copy_then_to_nothing() {
        let root = tempfile::tempdir().unwrap();
        let (url, _, server) = serve(StatusCode::INTERNAL_SERVER_ERROR, json!({})).await;
        let groq = endpoint("groq", &url);
        assert_eq!(discover(Some(root.path()), &groq).await, None);
        let stale = Found {
            fetched_at: 1,
            listen: vec![Heard {
                id: "whisper-large-v3".into(),
                label: None,
            }],
            speak: Vec::new(),
        };
        write(root.path(), "groq", &stale);
        assert_eq!(discover(Some(root.path()), &groq).await, Some(stale));
        server.abort();
        // Nothing listening at all is the same.
        assert!(
            discover(None, &endpoint("groq", "http://127.0.0.1:1"))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn openrouter_is_asked_twice_without_the_key_and_google_with_it_in_a_header() {
        let (url, seen, server) = serve(
            StatusCode::OK,
            json!({ "data": [
                { "id": "openai/whisper-1", "name": "OpenAI: Whisper 1",
                  "supported_voices": ["eve"] }
            ]}),
        )
        .await;
        let found = discover(None, &endpoint("openrouter", &url)).await.unwrap();
        assert_eq!(heard(&found.listen), ["openai/whisper-1"]);
        assert_eq!(spoken(&found.speak), ["openai/whisper-1"]);
        let mut paths: Vec<_> = seen.lock().unwrap().clone();
        paths.sort();
        assert_eq!(
            paths,
            [
                ("/models?output_modalities=speech".to_string(), false),
                ("/models?output_modalities=transcription".to_string(), false),
            ]
        );
        server.abort();

        let (url, seen, server) = serve(
            StatusCode::OK,
            json!({ "models": [{ "name": "models/gemini-3.8-flash-tts",
                "supportedGenerationMethods": ["generateContent"] }] }),
        )
        .await;
        let found = discover(None, &endpoint("google", &url)).await.unwrap();
        assert_eq!(spoken(&found.speak), ["gemini-3.8-flash-tts"]);
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            [("/v1beta/models?pageSize=1000".to_string(), true)]
        );
        server.abort();
    }
}
