//! The adapters against mock servers on localhost: what they send, what they
//! make of the answers, and that a failing voice falls back. No test here
//! reaches the network.

use super::wav::pcm16_wav;
use super::*;
use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use std::sync::Mutex;

struct Seen {
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Seen {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

type Requests = Arc<Mutex<Vec<Seen>>>;

/// Stops the server when the test is done with it.
struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn serve(app: Router) -> (String, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, Server(task))
}

/// A server that answers every request the same way, and remembers them.
async fn answering(
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
) -> (String, Requests, Server) {
    let requests = Requests::default();
    let seen = requests.clone();
    let app = Router::new().fallback(move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let seen = seen.clone();
        let body = body.clone();
        async move {
            seen.lock().unwrap().push(Seen {
                path: uri.path_and_query().unwrap().to_string(),
                headers,
                body: bytes.to_vec(),
            });
            (status, [(header::CONTENT_TYPE, content_type)], body)
        }
    });
    let (url, server) = serve(app).await;
    (url, requests, server)
}

async fn ok(content_type: &'static str, body: impl Into<Vec<u8>>) -> (String, Requests, Server) {
    answering(StatusCode::OK, content_type, body.into()).await
}

fn endpoint(url: &str, key: Option<&str>) -> Endpoint {
    Endpoint {
        provider_id: "openai".into(),
        base_url: format!("{url}/v1"),
        key: key.map(str::to_string),
    }
}

fn wav_clip() -> Clip {
    Clip {
        mime: "audio/wav".into(),
        bytes: b"RIFF-a-sixteen-kilohertz-clip".to_vec(),
    }
}

fn m4a_clip() -> Clip {
    Clip {
        mime: "audio/mp4".into(),
        bytes: b"ftypM4A-an-aac-clip".to_vec(),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

// ── The OpenAI shape ────────────────────────────────────────────────────────

#[tokio::test]
async fn a_wav_clip_is_posted_as_a_form_with_the_model_first_and_the_key() {
    let (url, requests, _server) = ok(
        "application/json",
        br#"{"text": "  Ask Mack to check the failing PR.  "}"#,
    )
    .await;
    let ears =
        OpenAiShape::listener(endpoint(&url, Some("test-key")), "gpt-4o-mini-transcribe").unwrap();

    let words = ears.transcribe(wav_clip()).await.unwrap();

    assert_eq!(words, "Ask Mack to check the failing PR.");
    let requests = requests.lock().unwrap();
    let seen = &requests[0];
    assert_eq!(seen.path, "/v1/audio/transcriptions");
    assert_eq!(seen.headers[header::AUTHORIZATION], "Bearer test-key");
    assert!(
        seen.headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("multipart/form-data; boundary=")
    );
    let form = seen.text();
    let model = form.find("name=\"model\"").expect("a model field");
    let file = form.find("name=\"file\"").expect("a file field");
    assert!(model < file, "the file is the last part");
    assert!(form.contains("gpt-4o-mini-transcribe"));
    assert!(form.contains("filename=\"clip.wav\""));
    assert!(form.contains("Content-Type: audio/wav"));
    assert!(contains(&seen.body, &wav_clip().bytes));
    assert_eq!(
        form.matches("Content-Disposition").count(),
        2,
        "the model and the file, and no field a strict server would refuse"
    );
}

#[tokio::test]
async fn an_aac_clip_goes_as_m4a() {
    let (url, requests, _server) = ok("application/json", br#"{"text": "hello"}"#).await;
    let ears = OpenAiShape::listener(endpoint(&url, Some("k")), "whisper-large-v3-turbo").unwrap();

    assert_eq!(ears.transcribe(m4a_clip()).await.unwrap(), "hello");

    let form = requests.lock().unwrap()[0].text();
    assert!(form.contains("filename=\"clip.m4a\""));
    assert!(form.contains("Content-Type: audio/mp4"));
    assert!(contains(form.as_bytes(), &m4a_clip().bytes));
}

#[tokio::test]
async fn a_type_is_read_as_a_mime_is_and_types_other_than_wav_and_mp4_are_refused_before_a_request()
{
    let (url, requests, _server) = ok("application/json", br#"{"text": "x"}"#).await;
    let ears = OpenAiShape::listener(endpoint(&url, None), "whisper-1").unwrap();

    // A parameter or a different case is still the same type.
    let clip = Clip {
        mime: "Audio/WAV; rate=16000".into(),
        ..wav_clip()
    };
    assert_eq!(ears.transcribe(clip).await.unwrap(), "x");

    let webm = Clip {
        mime: "audio/webm".into(),
        bytes: vec![1, 2, 3],
    };
    assert_eq!(
        ears.transcribe(webm).await,
        Err(SpeechError::UnsupportedFormat("audio/webm".into()))
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_keyless_server_gets_no_authorization_header() {
    let (url, requests, _server) = ok("application/json", br#"{"text": "x"}"#).await;
    let ears = OpenAiShape::listener(endpoint(&url, None), "whisper-1").unwrap();
    ears.transcribe(wav_clip()).await.unwrap();
    assert!(
        requests.lock().unwrap()[0]
            .headers
            .get(header::AUTHORIZATION)
            .is_none()
    );
}

#[tokio::test]
async fn an_adapter_built_for_one_job_refuses_the_other() {
    let (url, requests, _server) = ok("audio/wav", b"audio".to_vec()).await;
    let ears = OpenAiShape::listener(endpoint(&url, None), "whisper-1").unwrap();
    let mouth = OpenAiShape::speaker(
        endpoint(&url, None),
        "gpt-4o-mini-tts",
        "marin",
        AudioFormat::Wav,
        4096,
    )
    .unwrap();
    assert_eq!(ears.speak("Hello.").await, Err(SpeechError::WrongJob));
    assert_eq!(
        mouth.transcribe(wav_clip()).await,
        Err(SpeechError::WrongJob)
    );
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_sentence_is_posted_as_bare_text_and_comes_back_as_a_clip() {
    let (url, requests, _server) = ok("audio/wav", pcm16_wav(&[1, 0, 2, 0], 24_000)).await;
    let mouth = OpenAiShape::speaker(
        endpoint(&url, Some("test-key")),
        "gpt-4o-mini-tts",
        "marin",
        AudioFormat::Wav,
        4096,
    )
    .unwrap();

    let clip = mouth.speak("Handing that to Mack.").await.unwrap();

    assert_eq!(clip.mime, "audio/wav");
    assert_eq!(clip.bytes, pcm16_wav(&[1, 0, 2, 0], 24_000));
    let requests = requests.lock().unwrap();
    let seen = &requests[0];
    assert_eq!(seen.path, "/v1/audio/speech");
    assert_eq!(seen.headers[header::AUTHORIZATION], "Bearer test-key");
    assert_eq!(
        seen.json(),
        json!({
            "model": "gpt-4o-mini-tts",
            "input": "Handing that to Mack.",
            "voice": "marin",
            "response_format": "wav",
        }),
        "the words and the voice, and no instruction to perform them"
    );
    assert_eq!(
        mouth.id(),
        SpeechId {
            provider_id: "openai".into(),
            model_id: "gpt-4o-mini-tts".into(),
            voice: Some("marin".into())
        }
    );
}

#[tokio::test]
async fn the_clip_carries_the_type_the_provider_says_it_sent() {
    let (url, _requests, _server) = ok("audio/mpeg", b"ID3-mp3".to_vec()).await;
    let mouth = OpenAiShape::speaker(
        endpoint(&url, None),
        "tts-1",
        "alloy",
        AudioFormat::Wav,
        4096,
    )
    .unwrap();
    assert_eq!(mouth.speak("Hi.").await.unwrap().mime, "audio/mpeg");
}

#[tokio::test]
async fn text_over_a_voices_limit_is_spoken_in_parts_and_joined() {
    let (url, requests, _server) = ok("audio/wav", pcm16_wav(&[7, 0], 24_000)).await;
    let mouth = OpenAiShape::speaker(
        endpoint(&url, None),
        "canopylabs/orpheus-v1-english",
        "hannah",
        AudioFormat::Wav,
        30,
    )
    .unwrap();

    let clip = mouth
        .speak("Mack is on the failing PR. Clem finished the phone side.")
        .await
        .unwrap();

    let requests = requests.lock().unwrap();
    let said: Vec<_> = requests
        .iter()
        .map(|seen| seen.json()["input"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        said,
        [
            "Mack is on the failing PR.",
            "Clem finished the phone side."
        ]
    );
    assert_eq!(clip.mime, "audio/wav");
    assert_eq!(clip.bytes, pcm16_wav(&[7, 0, 7, 0], 24_000));
}

#[tokio::test]
async fn nothing_to_say_is_not_sent() {
    let (url, requests, _server) = ok("audio/wav", b"x".to_vec()).await;
    let mouth = OpenAiShape::speaker(
        endpoint(&url, None),
        "tts-1",
        "alloy",
        AudioFormat::Wav,
        4096,
    )
    .unwrap();
    assert_eq!(mouth.speak("  \n").await, Err(SpeechError::NothingToSay));
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_refusal_names_the_status_and_never_the_body() {
    let (url, _requests, _server) = answering(
        StatusCode::UNAUTHORIZED,
        "application/json",
        br#"{"error": "the key sk-test-secret is wrong, you said: hello"}"#.to_vec(),
    )
    .await;
    let ears = OpenAiShape::listener(endpoint(&url, Some("sk-test-secret")), "whisper-1").unwrap();

    let error = ears.transcribe(wav_clip()).await.unwrap_err();

    assert_eq!(
        error,
        SpeechError::Refused {
            provider_id: "openai".into(),
            status: 401
        }
    );
    let sentence = error.to_string();
    assert_eq!(sentence, "openai refused the request (HTTP 401).");
    assert!(!sentence.contains("sk-test-secret") && !sentence.contains("hello"));
}

#[tokio::test]
async fn an_answer_that_is_not_what_was_asked_for_is_malformed() {
    for body in ["not json", r#"{"words": "x"}"#, r#"{"text": 5}"#] {
        let (url, _requests, _server) = ok("application/json", body).await;
        let ears = OpenAiShape::listener(endpoint(&url, None), "whisper-1").unwrap();
        assert_eq!(ears.transcribe(wav_clip()).await, Err(malformed()));
    }
    let (url, _requests, _server) = ok("audio/wav", Vec::new()).await;
    let mouth = OpenAiShape::speaker(
        endpoint(&url, None),
        "tts-1",
        "alloy",
        AudioFormat::Wav,
        4096,
    )
    .unwrap();
    assert_eq!(mouth.speak("Hi.").await, Err(malformed()));
}

fn malformed() -> SpeechError {
    SpeechError::Malformed {
        provider_id: "openai".into(),
    }
}

#[tokio::test]
async fn a_server_that_is_not_there_is_unreachable() {
    let gone = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    let ears = OpenAiShape::listener(endpoint(&gone, None), "whisper-1").unwrap();
    assert_eq!(
        ears.transcribe(wav_clip()).await,
        Err(SpeechError::Unreachable {
            provider_id: "openai".into()
        })
    );
}

#[tokio::test]
async fn a_redirect_is_refused_and_the_key_stays_home() {
    let elsewhere = Requests::default();
    let seen = elsewhere.clone();
    let app = Router::new()
        .route(
            "/v1/audio/transcriptions",
            axum::routing::post(|| async {
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [(header::LOCATION, "/elsewhere")],
                )
            }),
        )
        .route(
            "/elsewhere",
            axum::routing::post(move |headers: HeaderMap| {
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push(Seen {
                        path: "/elsewhere".into(),
                        headers,
                        body: Vec::new(),
                    });
                    "{}"
                }
            }),
        );
    let (url, _server) = serve(app).await;
    let ears = OpenAiShape::listener(endpoint(&url, Some("test-key")), "whisper-1").unwrap();

    assert_eq!(
        ears.transcribe(wav_clip()).await,
        Err(SpeechError::Refused {
            provider_id: "openai".into(),
            status: 307
        })
    );
    assert!(elsewhere.lock().unwrap().is_empty());
}

// ── The call ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn an_answer_bigger_than_the_limit_is_refused_as_malformed() {
    let (url, _requests, _server) = ok("audio/wav", vec![0u8; 4096]).await;
    let id = SpeechId {
        provider_id: "openai".into(),
        model_id: "tts-1".into(),
        voice: None,
    };
    let request = reqwest::Client::new().post(format!("{url}/v1/audio/speech"));
    assert!(matches!(
        call(request, &id, "speak", 1024).await,
        Err(SpeechError::Malformed { .. })
    ));
}

// ── Google ──────────────────────────────────────────────────────────────────

fn google_listener(url: &str) -> Google {
    Google::listener(url, "test-key", "gemini-3.5-flash-lite").unwrap()
}

fn google_speaker(url: &str) -> Google {
    Google::speaker(url, "test-key", "gemini-3.8-flash-tts", "Sulafat").unwrap()
}

#[tokio::test]
async fn gemini_is_asked_to_transcribe_a_clip_sent_inline() {
    let (url, requests, _server) = ok(
        "application/json",
        json!({"candidates": [{"content": {"parts": [{"text": " Ask Mack "}, {"text": "to check the PR. "}]}}]})
            .to_string(),
    )
    .await;

    let words = google_listener(&url).transcribe(wav_clip()).await.unwrap();

    assert_eq!(words, "Ask Mack to check the PR.");
    let requests = requests.lock().unwrap();
    let seen = &requests[0];
    assert_eq!(
        seen.path,
        "/v1beta/models/gemini-3.5-flash-lite:generateContent"
    );
    assert_eq!(seen.headers["x-goog-api-key"], "test-key");
    assert!(
        !seen.path.contains("test-key"),
        "the key is never in the URL"
    );
    let body = seen.json();
    let parts = &body["contents"][0]["parts"];
    assert!(parts[0]["text"].as_str().unwrap().starts_with("Transcribe"));
    assert_eq!(parts[1]["inlineData"]["mimeType"], "audio/wav");
    assert_eq!(
        parts[1]["inlineData"]["data"],
        STANDARD.encode(&wav_clip().bytes)
    );
}

#[tokio::test]
async fn an_aac_clip_goes_to_gemini_as_m4a() {
    let (url, requests, _server) = ok(
        "application/json",
        json!({"candidates": [{"content": {"parts": [{"text": "hi"}]}}]}).to_string(),
    )
    .await;
    google_listener(&url).transcribe(m4a_clip()).await.unwrap();
    assert_eq!(
        requests.lock().unwrap()[0].json()["contents"][0]["parts"][1]["inlineData"]["mimeType"],
        "audio/m4a"
    );
}

#[tokio::test]
async fn a_clip_with_nobody_in_it_is_empty_words_and_a_refusal_to_answer_is_malformed() {
    let (url, _requests, _server) = ok(
        "application/json",
        json!({"candidates": [{"content": {"parts": []}}]}).to_string(),
    )
    .await;
    assert_eq!(
        google_listener(&url).transcribe(wav_clip()).await.unwrap(),
        ""
    );

    let (url, _requests, _server) = ok(
        "application/json",
        json!({"promptFeedback": {"blockReason": "OTHER"}}).to_string(),
    )
    .await;
    assert!(matches!(
        google_listener(&url).transcribe(wav_clip()).await,
        Err(SpeechError::Malformed { .. })
    ));
}

#[tokio::test]
async fn gemini_speaks_bare_text_and_its_pcm_is_wrapped_as_a_wav() {
    let pcm = [1u8, 0, 2, 0, 3, 0];
    let (url, requests, _server) = ok(
        "application/json",
        json!({"candidates": [{"content": {"parts": [{"inlineData": {
            "mimeType": "audio/L16;codec=pcm;rate=24000",
            "data": STANDARD.encode(pcm),
        }}]}}]})
        .to_string(),
    )
    .await;

    let clip = google_speaker(&url)
        .speak("Handing that to Mack.")
        .await
        .unwrap();

    assert_eq!(clip.mime, "audio/wav");
    assert_eq!(clip.bytes, pcm16_wav(&pcm, 24_000));
    let requests = requests.lock().unwrap();
    let seen = &requests[0];
    assert_eq!(
        seen.path,
        "/v1beta/models/gemini-3.8-flash-tts:generateContent"
    );
    assert_eq!(seen.headers["x-goog-api-key"], "test-key");
    assert_eq!(
        seen.json(),
        json!({
            "contents": [{"role": "user", "parts": [{"text": "Handing that to Mack."}]}],
            "generationConfig": {
                "responseModalities": ["AUDIO"],
                "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Sulafat"}}},
            },
        }),
        "Gemini TTS reads any instruction aloud, so the only text sent is the words"
    );
}

#[tokio::test]
async fn the_rate_gemini_reports_is_the_rate_of_the_wav() {
    let (url, _requests, _server) = ok(
        "application/json",
        json!({"candidates": [{"content": {"parts": [{"inlineData": {
            "mimeType": "audio/L16;rate=16000",
            "data": STANDARD.encode([1u8, 0]),
        }}]}}]})
        .to_string(),
    )
    .await;
    let clip = google_speaker(&url).speak("Hi.").await.unwrap();
    assert_eq!(clip.bytes, pcm16_wav(&[1, 0], 16_000));
}

#[tokio::test]
async fn gemini_answering_without_audio_is_malformed() {
    for body in [
        json!({"candidates": []}),
        json!({"candidates": [{"content": {"parts": [{"text": "I cannot"}]}}]}),
        json!({"candidates": [{"content": {"parts": [{"inlineData": {"mimeType": "audio/L16", "data": "!!not base64!!"}}]}}]}),
        json!({"candidates": [{"content": {"parts": [{"inlineData": {"mimeType": "video/mp4", "data": STANDARD.encode([1u8])}}]}}]}),
    ] {
        let (url, _requests, _server) = ok("application/json", body.to_string()).await;
        assert!(
            matches!(
                google_speaker(&url).speak("Hi.").await,
                Err(SpeechError::Malformed { .. })
            ),
            "{body}"
        );
    }
}

#[tokio::test]
async fn a_gemini_adapter_refuses_the_job_it_was_not_built_for() {
    let (url, requests, _server) = ok("application/json", "{}").await;
    assert_eq!(
        google_listener(&url).speak("Hi.").await,
        Err(SpeechError::WrongJob)
    );
    assert_eq!(
        google_speaker(&url).transcribe(wav_clip()).await,
        Err(SpeechError::WrongJob)
    );
    assert!(requests.lock().unwrap().is_empty());
    assert_eq!(
        google_speaker(&url).id(),
        SpeechId {
            provider_id: "google".into(),
            model_id: "gemini-3.8-flash-tts".into(),
            voice: Some("Sulafat".into())
        }
    );
}

// ── The utterance clock ─────────────────────────────────────────────────────

#[tokio::test]
async fn the_ears_start_the_clock_and_the_first_clip_of_the_answer_stops_it() {
    let (url, _requests, _server) = ok("application/json", br#"{"text": "ask Mack"}"#).await;
    let (speaking, _requests, _server) = ok("audio/wav", b"sound".to_vec()).await;
    let clock = TurnClock::default();
    let ears = OpenAiShape::listener(endpoint(&url, None), "whisper-1")
        .unwrap()
        .with_clock(&clock);
    let mouth = OpenAiShape::speaker(
        endpoint(&speaking, None),
        "tts-1",
        "alloy",
        AudioFormat::Wav,
        4096,
    )
    .unwrap()
    .with_clock(&clock);

    assert!(!clock.waiting());
    assert_eq!(ears.transcribe(wav_clip()).await.unwrap(), "ask Mack");
    assert!(clock.waiting(), "the answer is still to come");
    mouth.speak("Handing that to Mack.").await.unwrap();
    assert!(!clock.waiting());
    // A delivery, with no utterance behind it, starts nothing and reports nothing.
    mouth.speak("Mack is done.").await.unwrap();
    assert!(!clock.waiting());
}

#[tokio::test]
async fn an_utterance_that_says_nothing_or_fails_is_not_waited_on() {
    let (silent, _requests, _server) = ok("application/json", br#"{"text": "  "}"#).await;
    let (refusing, _requests, _server) =
        answering(StatusCode::INTERNAL_SERVER_ERROR, "text/plain", Vec::new()).await;
    for url in [silent, refusing] {
        let clock = TurnClock::default();
        let ears = OpenAiShape::listener(endpoint(&url, None), "whisper-1")
            .unwrap()
            .with_clock(&clock);
        let _ = ears.transcribe(wav_clip()).await;
        assert!(!clock.waiting());
    }
}

#[tokio::test]
async fn gemini_shares_the_clock_too_and_a_failed_clip_leaves_it_running() {
    let (listening, _requests, _server) = ok(
        "application/json",
        json!({"candidates": [{"content": {"parts": [{"text": "ask Mack"}]}}]}).to_string(),
    )
    .await;
    let (broken, _requests, _server) =
        answering(StatusCode::BAD_GATEWAY, "text/plain", Vec::new()).await;
    let (working, _requests, _server) = ok(
        "application/json",
        json!({"candidates": [{"content": {"parts": [{"inlineData": {
            "mimeType": "audio/L16;rate=24000",
            "data": STANDARD.encode([1u8, 0]),
        }}]}}]})
        .to_string(),
    )
    .await;
    let clock = TurnClock::default();
    let ears = google_listener(&listening).with_clock(&clock);
    ears.transcribe(wav_clip()).await.unwrap();
    assert!(clock.waiting());
    // The voice that failed answered nothing, so the answer is still awaited.
    assert!(
        google_speaker(&broken)
            .with_clock(&clock)
            .speak("Hi.")
            .await
            .is_err()
    );
    assert!(clock.waiting());
    google_speaker(&working)
        .with_clock(&clock)
        .speak("Hi.")
        .await
        .unwrap();
    assert!(!clock.waiting());
}
