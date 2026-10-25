//! A connected Ollama server as a voice, through the seam the core calls:
//! what Settings is offered (`speech::options`), what a call speaks with
//! (`speech::resolve_output`) and what it sends. The Ollama here is a fake
//! on loopback that answers as Ollama 0.40 does; no test reaches the network
//! or needs Ollama installed.

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use futures_util::future::BoxFuture;
use hotline_contract::CapabilityProvider;
use hotline_room::provider_auth::ProviderAuth;
use hotline_room::voice::ledger;
use hotline_room::voice::settings::{Choice, VoiceSettings};
use hotline_support::credentials::CredentialFile;
use hotline_voice::realtime::live::Login;
use hotline_voice::speech::{self, Connected, SignIns, SpeechError};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// What the fake was asked: the path, whether it was signed, and the body.
type Asked = Arc<Mutex<Vec<(String, bool, Vec<u8>)>>>;

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Half a second of 24 kHz mono PCM16, as Ollama's `pcm` answer is.
const SAMPLES: usize = 12_000;

/// An Ollama with these models installed, `(name, capabilities)`, each
/// speech model knowing these voices. `speech` says whether
/// `/v1/audio/speech` speaks or answers as Ollama does for a model it no
/// longer has.
async fn ollama(
    models: &[(&str, &[&str])],
    voices: &[&str],
    speech: bool,
) -> (String, Asked, Server) {
    let tags = json!({ "models": models.iter().map(|(name, capabilities)| json!({
        "name": name, "model": name, "capabilities": capabilities,
    })).collect::<Vec<_>>() });
    let show = json!({ "capabilities": ["speech"], "voices": voices });
    let asked = Asked::default();
    let log = asked.clone();
    let app = Router::new().fallback(move |uri: Uri, headers: HeaderMap, body: Bytes| {
        let (log, tags, show) = (log.clone(), tags.clone(), show.clone());
        async move {
            let path = uri.path().to_string();
            log.lock().unwrap().push((
                path.clone(),
                headers.contains_key(header::AUTHORIZATION),
                body.to_vec(),
            ));
            let json = |value: Value| {
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    value.to_string().into_bytes(),
                )
            };
            match path.as_str() {
                "/api/tags" => json(tags),
                "/api/show" => json(show),
                "/v1/audio/speech" if speech => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "audio/pcm")],
                    vec![0u8; SAMPLES * 2],
                ),
                _ => (
                    StatusCode::NOT_FOUND,
                    [(header::CONTENT_TYPE, "application/json")],
                    br#"{"error":"model not found"}"#.to_vec(),
                ),
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, asked, Server(task))
}

fn no_bearer<'a>(
    _: &'a CredentialFile,
    _: Option<&'a str>,
) -> BoxFuture<'a, Result<String, String>> {
    Box::pin(async { Err("No Grok login in this test.".to_string()) })
}

fn no_login(_: &Path) -> BoxFuture<'_, Result<Login, String>> {
    Box::pin(async { Err("No ChatGPT login in this test.".to_string()) })
}

fn connected(root: &Path, providers: Vec<(&str, ProviderAuth)>) -> Connected {
    Connected {
        root: root.to_path_buf(),
        providers: providers
            .into_iter()
            .map(|(id, auth)| (id.to_string(), auth))
            .collect(),
        sign_ins: SignIns {
            xai: no_bearer,
            chatgpt: no_login,
        },
    }
}

fn local(url: &str) -> ProviderAuth {
    ProviderAuth::Local {
        base_url: url.to_string(),
    }
}

fn speaking_with(model: Option<&str>, voice: Option<&str>) -> VoiceSettings {
    VoiceSettings {
        tts: Some(Choice {
            provider_id: "ollama".into(),
            model_id: model.map(str::to_string),
            voice: voice.map(str::to_string),
            effort: None,
        }),
        ..VoiceSettings::default()
    }
}

fn ollama_offered(tts: &[CapabilityProvider]) -> Option<&CapabilityProvider> {
    tts.iter().find(|provider| provider.provider_id == "ollama")
}

const PHONON: (&str, &[&str]) = ("phonon:latest", &["speech"]);
const LLAMA: (&str, &[&str]) = ("llama3.1:8b", &["completion", "tools"]);

#[tokio::test]
async fn ollama_speech_models_are_offered_with_the_voices_ollama_lists() {
    let (url, asked, _server) = ollama(&[LLAMA, PHONON], &["Alfie", "default", "Zoey"], true).await;
    let root = tempfile::tempdir().unwrap();
    let options = speech::options(&connected(root.path(), vec![("ollama", local(&url))])).await;

    let offered = ollama_offered(&options.tts).expect("Ollama is offered for speaking");
    assert_eq!(offered.provider_name, "Ollama");
    assert_eq!(offered.models.len(), 1, "a chat model does not speak");
    let phonon = &offered.models[0];
    assert_eq!(phonon.id, "phonon:latest");
    assert_eq!(phonon.label.as_deref(), Some("phonon"));
    assert_eq!(
        phonon.voices.as_deref(),
        Some(&["default", "Alfie", "Zoey"].map(String::from)[..])
    );
    // Ollama hears nothing, and pulling a model is not choosing it.
    assert!(options.stt.iter().all(|one| one.provider_id != "ollama"));
    assert!(options.automatic_tts.is_none());
    let asked = asked.lock().unwrap();
    assert!(asked.iter().all(|(_, signed, _)| !signed));
    let shown: Vec<Value> = asked
        .iter()
        .filter(|(path, ..)| path == "/api/show")
        .map(|(_, _, body)| serde_json::from_slice(body).unwrap())
        .collect();
    assert_eq!(shown, [json!({ "model": "phonon:latest" })]);
}

#[tokio::test]
async fn a_chosen_ollama_voice_speaks_through_v1_audio_speech_for_nothing() {
    let (url, asked, _server) = ollama(&[PHONON], &["default", "Zoey"], true).await;
    let root = tempfile::tempdir().unwrap();
    let desk = connected(root.path(), vec![("ollama", local(&format!("{url}/")))]);

    let output =
        speech::resolve_output(&desk, &speaking_with(Some("phonon:latest"), Some("Zoey"))).unwrap();
    let id = output.tts.id();
    assert_eq!(
        (
            id.provider_id.as_str(),
            id.model_id.as_str(),
            id.voice.as_deref()
        ),
        ("ollama", "phonon:latest", Some("Zoey"))
    );
    assert_eq!(output.tts.output_mime(), "audio/wav");
    assert!(output.fallback_tts.is_none());

    let clip = output
        .tts
        .speak("Mack has the failing test.")
        .await
        .unwrap();
    assert_eq!(clip.mime, "audio/wav");
    assert_eq!(&clip.bytes[..4], b"RIFF");
    assert_eq!(
        u32::from_le_bytes(clip.bytes[24..28].try_into().unwrap()),
        24_000
    );
    assert_eq!(clip.bytes.len(), 44 + SAMPLES * 2);
    let asked = asked.lock().unwrap();
    let (path, signed, body) = asked.last().unwrap();
    assert_eq!(path, "/v1/audio/speech");
    assert!(!signed);
    assert_eq!(
        serde_json::from_slice::<Value>(body).unwrap(),
        json!({ "model": "phonon:latest", "voice": "Zoey",
                "input": "Mack has the failing test.", "response_format": "pcm" })
    );
    assert_eq!(ledger::tts_usd(&id.provider_id, 8_000), 0.0);
    assert!(ledger::is_free(&id.provider_id));
}

#[tokio::test]
async fn a_model_chosen_without_a_voice_speaks_in_its_first_and_the_provider_alone_in_its_first_model()
 {
    let (url, _asked, _server) = ollama(&[PHONON], &["Alfie", "default"], true).await;
    let root = tempfile::tempdir().unwrap();
    let desk = connected(root.path(), vec![("ollama", local(&url))]);
    speech::options(&desk).await;
    for settings in [
        speaking_with(Some("phonon:latest"), None),
        speaking_with(None, None),
    ] {
        let id = speech::resolve_output(&desk, &settings).unwrap().tts.id();
        assert_eq!(
            (id.model_id.as_str(), id.voice.as_deref()),
            ("phonon:latest", Some("default"))
        );
    }
}

#[tokio::test]
async fn ollama_not_running_or_without_a_speech_model_is_not_offered() {
    let root = tempfile::tempdir().unwrap();
    // Nothing listens on port 1.
    let stopped = connected(root.path(), vec![("ollama", local("http://127.0.0.1:1"))]);
    let options = speech::options(&stopped).await;
    assert_eq!(options.tts, []);

    let (url, asked, _server) = ollama(&[LLAMA], &[], true).await;
    let chat_only = connected(root.path(), vec![("ollama", local(&url))]);
    let options = speech::options(&chat_only).await;
    assert!(ollama_offered(&options.tts).is_none());
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .all(|(path, ..)| path != "/api/show"),
        "only speech models are asked for their voices"
    );
    // A speech model that lists no voices cannot be asked for one.
    let (url, _asked, _server) = ollama(&[PHONON], &[], true).await;
    let voiceless = connected(root.path(), vec![("ollama", local(&url))]);
    assert!(ollama_offered(&speech::options(&voiceless).await.tts).is_none());

    // Offered once, then the server quits: not offered from yesterday's list.
    let (url, _asked, server) = ollama(&[PHONON], &["default"], true).await;
    let desk = connected(root.path(), vec![("ollama", local(&url))]);
    assert!(ollama_offered(&speech::options(&desk).await.tts).is_some());
    drop(server);
    tokio::task::yield_now().await;
    assert!(ollama_offered(&speech::options(&desk).await.tts).is_none());
}

#[tokio::test]
async fn a_saved_ollama_voice_whose_model_is_gone_fails_aloud_and_has_the_next_voice_to_fall_back_on()
 {
    let (ollama_url, _asked, _ollama) = ollama(&[], &[], false).await;
    // A keyless openai-compatible server that speaks: the next voice.
    let (gateway, _gateway_asked, _gateway_server) = ollama(&[], &[], true).await;
    let root = tempfile::tempdir().unwrap();
    let desk = connected(
        root.path(),
        vec![
            ("ollama", local(&ollama_url)),
            (
                "custom-6f1c1f2e-4d5a-4b8e-9a51-1f2d3c4b5a69",
                ProviderAuth::Custom {
                    name: "Gateway".into(),
                    base_url: format!("{gateway}/v1"),
                    api_key: None,
                    config: hotline_contract::CustomProvider {
                        api: hotline_contract::OpenAiApi::ChatCompletions,
                        models: vec!["tts-1".into()],
                    },
                },
            ),
        ],
    );
    // The picker no longer lists it, and still says what was chosen.
    let options = speech::options(&desk).await;
    assert!(ollama_offered(&options.tts).is_none());

    let output =
        speech::resolve_output(&desk, &speaking_with(Some("phonon:latest"), Some("Zoey"))).unwrap();
    assert_eq!(output.tts.id().provider_id, "ollama");
    let error = output.tts.speak("Still there?").await.unwrap_err();
    assert_eq!(
        error,
        SpeechError::Refused {
            provider_id: "ollama".into(),
            status: 404
        }
    );
    let fallback = output
        .fallback_tts
        .expect("the next connected voice is the fallback");
    assert_eq!(fallback.id().model_id, "tts-1");
    assert!(fallback.speak("Still there?").await.is_ok());

    // Ollama is never the fallback nobody named.
    let automatic = speech::resolve_output(&desk, &VoiceSettings::default()).unwrap();
    assert_eq!(automatic.tts.id().model_id, "tts-1");
    assert!(automatic.fallback_tts.is_none());
}
