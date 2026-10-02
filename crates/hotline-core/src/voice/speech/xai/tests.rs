use super::*;
use axum::{
    Router,
    body::{Body, Bytes},
    http::{HeaderMap, StatusCode, Uri, header},
};
use futures_util::stream;
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::handshake::server::{Request, Response},
};

/// Explicit operator proof only: never runs in the ordinary suite, and its
/// output contains status/results rather than credential or provider content.
#[tokio::test]
#[ignore = "requires explicit operator authorization and a real Hotline Grok subscription"]
async fn live_grok_subscription_voice_entitlement_probe() {
    let root = std::path::PathBuf::from(
        std::env::var("HOTLINE_XAI_VOICE_PROBE_ROOT")
            .expect("set the authorized Hotline data root"),
    );
    let mut metadata = std::collections::BTreeMap::new();
    for line in std::fs::read_to_string(root.join("room.jsonl"))
        .unwrap()
        .lines()
    {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event["kind"] == "credential"
            && let Some(id) = event["id"].as_str()
        {
            metadata.insert(id.to_string(), event);
        }
    }
    let login = metadata.into_iter().find_map(|(id, event)| {
        (event["providerId"] == "xai"
            && event["credentialKind"] == "oauth"
            && event["revoked"] != true
            && event["deleted"] != true)
            .then_some(id)
    });
    let Some(id) = login else {
        println!("grok_voice_probe login_metadata=false");
        return;
    };
    println!("grok_voice_probe login_metadata=true");
    let backend: Value =
        serde_json::from_slice(&std::fs::read(root.join("store.json")).unwrap()).unwrap();
    assert_eq!(
        backend["backend"], "native",
        "probe requires the declared native store"
    );
    let tokens = crate::credentials::CredentialFiles::new(
        root.clone(),
        Arc::new(crate::credentials::NativeStore),
    )
    .file(root.join("vault/logins").join(id).join("auth.json"));
    match tokens.read() {
        Ok(Some(_)) => println!("grok_voice_probe stored_credential_readable=true"),
        Ok(None) => {
            println!("grok_voice_probe stored_credential_readable=false missing_file=true");
            return;
        }
        Err(error) => {
            let reason = if error.to_string().contains("OS credential storage") {
                "native_store_unavailable"
            } else if error.kind() == std::io::ErrorKind::NotFound {
                "native_entry_missing"
            } else if error.kind() == std::io::ErrorKind::PermissionDenied {
                "private_path_or_store_denied"
            } else {
                "invalid_record"
            };
            println!("grok_voice_probe stored_credential_readable=false reason={reason}");
            return;
        }
    }
    match timeout(
        Duration::from_secs(30),
        crate::providers::xai::bearer(&tokens, None),
    )
    .await
    {
        Ok(Ok(_)) => {}
        _ => {
            println!("grok_voice_probe credential_available=false");
            return;
        }
    }
    println!("grok_voice_probe credential_available=true");
    let endpoint = Endpoint {
        provider_id: "xai-subscription".into(),
        base_url: BASE_URL.into(),
        key: None,
    };
    let voice = Xai::speaker(endpoint.clone(), SPEAK_MODEL, "eve")
        .unwrap()
        .with_subscription(tokens.clone())
        .unwrap();
    let clip = match voice.speak("Hello. This is a quick voice test.").await {
        Ok(clip) => {
            println!(
                "grok_voice_probe tts_success=true playable_wav={} duration_ms={}",
                clip.bytes.len() > 44,
                (clip.bytes.len().saturating_sub(44) * 1000) / (OUTPUT_RATE as usize * 2)
            );
            Some(clip)
        }
        Err(SpeechError::Refused { status, .. } | SpeechError::Entitlement { status, .. }) => {
            println!("grok_voice_probe tts_success=false status={status}");
            None
        }
        Err(_) => {
            println!("grok_voice_probe tts_success=false transport_or_format=true");
            None
        }
    };
    // The known generated mono PCM24k is converted to 16k for the tiny STT
    // proof. If TTS is denied, half a second of synthetic silence separately
    // checks STT entitlement without recording the operator. Nothing is
    // played, retained, or sent outside these voice routes.
    let pcm: Vec<u8> = if let Some(clip) = clip {
        println!("grok_voice_probe stt_sample=generated_hello");
        let samples: Vec<i16> = clip.bytes[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
            .collect();
        (0..samples.len() * 2 / 3)
            .flat_map(|index| samples[index * 3 / 2].to_le_bytes())
            .collect()
    } else {
        println!("grok_voice_probe stt_sample=synthetic_silence");
        vec![0; INPUT_RATE as usize]
    };
    let listen = Xai::listener(endpoint, LISTEN_MODEL)
        .unwrap()
        .with_subscription(tokens)
        .unwrap();
    let (sender, input) = mpsc::channel(pcm.len().div_ceil(INPUT_FRAME_LIMIT).max(1));
    for chunk in pcm.chunks(INPUT_FRAME_LIMIT) {
        sender.send(chunk.to_vec()).await.unwrap();
    }
    drop(sender);
    match listen.transcribe_live(input, INPUT_RATE).await {
        Ok(text) => println!(
            "grok_voice_probe stt_success=true nonempty_transcript={}",
            !text.is_empty()
        ),
        Err(SpeechError::Refused { status, .. } | SpeechError::Entitlement { status, .. }) => {
            println!("grok_voice_probe stt_success=false status={status}")
        }
        Err(_) => println!("grok_voice_probe stt_success=false transport_or_format=true"),
    }
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone)]
struct Seen {
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
}
type Requests = Arc<Mutex<Vec<Seen>>>;

async fn http_server(app: Router) -> (String, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, Server(task))
}

async fn answering(
    status: StatusCode,
    content_type: &'static str,
    bytes: Vec<u8>,
) -> (String, Requests, Server) {
    let requests = Requests::default();
    let seen = requests.clone();
    let app = Router::new().fallback(move |uri: Uri, headers: HeaderMap, body: Bytes| {
        let seen = seen.clone();
        let bytes = bytes.clone();
        async move {
            seen.lock().unwrap().push(Seen {
                path: uri.to_string(),
                headers,
                body: body.to_vec(),
            });
            (status, [(header::CONTENT_TYPE, content_type)], bytes)
        }
    });
    let (url, server) = http_server(app).await;
    (url, requests, server)
}

// Tungstenite requires its HTTP response as this callback's error type;
// boxing it would no longer implement the library's Callback contract.
#[allow(clippy::result_large_err)]
async fn websocket<F, Fut>(handle: F) -> (String, Requests, Server)
where
    F: FnOnce(WebSocketStream<TcpStream>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let requests = Requests::default();
    let seen = requests.clone();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let socket = accept_hdr_async(socket, |request: &Request, response: Response| {
            seen.lock().unwrap().push(Seen {
                path: request.uri().to_string(),
                headers: request.headers().clone(),
                body: vec![],
            });
            Ok(response)
        })
        .await
        .unwrap();
        handle(socket).await;
    });
    (url, requests, Server(task))
}

fn endpoint(url: &str, key: &str) -> Endpoint {
    Endpoint {
        provider_id: PROVIDER_ID.into(),
        base_url: url.into(),
        key: Some(key.into()),
    }
}
fn listener(url: &str) -> Xai {
    Xai::listener(endpoint(url, "test-key"), LISTEN_MODEL).unwrap()
}
fn speaker(url: &str) -> Xai {
    Xai::speaker(endpoint(url, "test-key"), SPEAK_MODEL, "eve").unwrap()
}
fn login(root: &std::path::Path) -> CredentialFile {
    let tokens =
        crate::credentials::CredentialFiles::new(root.into(), crate::credentials::default_store())
            .file(root.join("auth.json"));
    crate::providers::xai::sign_in_for_test(&tokens, "first-login");
    tokens
}
fn subscription_voice(tokens: CredentialFile, url: &str) -> Xai {
    let mut voice = speaker(BASE_URL).with_subscription(tokens).unwrap();
    // Only fixtures replace the production origin after the guarded constructor.
    voice.endpoint.base_url = url.into();
    voice
}
fn malformed() -> SpeechError {
    SpeechError::Malformed {
        provider_id: PROVIDER_ID.into(),
    }
}

#[tokio::test]
async fn subscription_tts_resolves_each_request_and_retries_one_rejected_login() {
    let dir = tempfile::tempdir().unwrap();
    let tokens = login(dir.path());
    let rotate = tokens.clone();
    let requests = Requests::default();
    let seen = requests.clone();
    let app = Router::new().fallback(move |uri: Uri, headers: HeaderMap, body: Bytes| {
        let seen = seen.clone();
        let rotate = rotate.clone();
        async move {
            let first = {
                let mut seen = seen.lock().unwrap();
                seen.push(Seen {
                    path: uri.to_string(),
                    headers,
                    body: body.to_vec(),
                });
                seen.len() == 1
            };
            if first {
                // Another synchronized client already refreshed this login.
                crate::providers::xai::sign_in_for_test(&rotate, "fresh-login");
                (
                    StatusCode::UNAUTHORIZED,
                    [(header::CONTENT_TYPE, "audio/pcm")],
                    vec![],
                )
            } else {
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "audio/pcm")],
                    vec![0, 0],
                )
            }
        }
    });
    let (url, _server) = http_server(app).await;
    let voice = subscription_voice(tokens.clone(), &url);
    assert!(voice.is_subscription());
    assert!(voice.endpoint.key.is_none());
    assert_eq!(voice.id().provider_id, SUBSCRIPTION_PROVIDER_ID);
    assert!(
        voice
            .speak("Hello.")
            .await
            .unwrap()
            .bytes
            .starts_with(b"RIFF")
    );
    crate::providers::xai::sign_in_for_test(&tokens, "next-login");
    voice.speak("Again.").await.unwrap();
    let seen = requests.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].headers[header::AUTHORIZATION], "Bearer first-login");
    assert_eq!(seen[1].headers[header::AUTHORIZATION], "Bearer fresh-login");
    assert_eq!(seen[2].headers[header::AUTHORIZATION], "Bearer next-login");
}

#[tokio::test]
async fn subscription_denial_is_typed_redacted_and_never_uses_the_endpoint_key() {
    let dir = tempfile::tempdir().unwrap();
    let tokens = login(dir.path());
    for status in [
        StatusCode::PAYMENT_REQUIRED,
        StatusCode::FORBIDDEN,
        StatusCode::TOO_MANY_REQUESTS,
    ] {
        let (url, seen, _server) = answering(
            status,
            "text/plain",
            b"private provider transcript".to_vec(),
        )
        .await;
        let voice = subscription_voice(tokens.clone(), &url);
        let error = voice.speak("Private words.").await.unwrap_err();
        assert_eq!(
            error,
            SpeechError::Entitlement {
                provider_id: SUBSCRIPTION_PROVIDER_ID.into(),
                status: status.as_u16()
            }
        );
        assert!(!error.to_string().contains("Private"));
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
    let (url, seen, _server) = answering(StatusCode::OK, "audio/pcm", vec![0, 0]).await;
    let voice = subscription_voice(tokens.clone(), &url);
    tokens.delete().unwrap();
    assert_eq!(
        voice.speak("Hello.").await.unwrap_err(),
        SpeechError::SignInRequired {
            provider_id: SUBSCRIPTION_PROVIDER_ID.into()
        }
    );
    assert!(seen.lock().unwrap().is_empty());
    assert!(
        speaker("https://other.example/v1")
            .with_subscription(tokens)
            .is_err()
    );
}

#[tokio::test]
async fn subscription_tts_refreshes_only_once_and_batch_stt_never_replays_uploaded_audio() {
    let dir = tempfile::tempdir().unwrap();
    let tokens = login(dir.path());
    let rotate = tokens.clone();
    let requests = Requests::default();
    let seen = requests.clone();
    let app = Router::new().fallback(move |uri: Uri, headers: HeaderMap, body: Bytes| {
        let seen = seen.clone();
        let rotate = rotate.clone();
        async move {
            let count = {
                let mut seen = seen.lock().unwrap();
                seen.push(Seen {
                    path: uri.to_string(),
                    headers,
                    body: body.to_vec(),
                });
                seen.len()
            };
            if count == 1 {
                crate::providers::xai::sign_in_for_test(&rotate, "fresh-login");
            }
            (StatusCode::UNAUTHORIZED, "private refusal")
        }
    });
    let (url, _server) = http_server(app).await;
    let voice = subscription_voice(tokens.clone(), &url);
    assert!(matches!(
        voice.speak("Hello.").await,
        Err(SpeechError::SignInRequired { .. })
    ));
    assert_eq!(requests.lock().unwrap().len(), 2);
    let mut listen = Xai::listener(endpoint(BASE_URL, "unused-api-key"), LISTEN_MODEL)
        .unwrap()
        .with_subscription(tokens)
        .unwrap();
    listen.endpoint.base_url = url;
    assert!(matches!(
        listen
            .transcribe(Clip {
                mime: "audio/wav".into(),
                bytes: vec![0, 0]
            })
            .await,
        Err(SpeechError::SignInRequired { .. })
    ));
    assert_eq!(requests.lock().unwrap().len(), 3);
}

// Tungstenite's handshake callback requires the unboxed protocol response.
#[allow(clippy::result_large_err)]
#[tokio::test]
async fn subscription_live_auth_retries_before_any_caller_audio_and_uses_fresh_login() {
    let dir = tempfile::tempdir().unwrap();
    let tokens = login(dir.path());
    let rotate = tokens.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let refused = accept_hdr_async(socket, |request: &Request, _: Response| {
            assert_eq!(
                request.headers()[header::AUTHORIZATION],
                "Bearer first-login"
            );
            crate::providers::xai::sign_in_for_test(&rotate, "fresh-login");
            Err(http::Response::builder().status(401).body(None).unwrap())
        })
        .await;
        assert!(refused.is_err());
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = accept_hdr_async(socket, |request: &Request, response: Response| {
            assert_eq!(
                request.headers()[header::AUTHORIZATION],
                "Bearer fresh-login"
            );
            Ok(response)
        })
        .await
        .unwrap();
        event(&mut socket, json!({"type":"transcript.created"})).await;
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::Binary(vec![0; 3200].into())
        );
        let done = socket.next().await.unwrap().unwrap();
        assert!(done.to_text().unwrap().contains("audio.done"));
        event(
            &mut socket,
            json!({"type":"transcript.done", "text":"hello"}),
        )
        .await;
    });
    let _server = Server(task);
    let mut listen = Xai::listener(endpoint(BASE_URL, "unused-key"), LISTEN_MODEL)
        .unwrap()
        .with_subscription(tokens)
        .unwrap();
    listen.endpoint.base_url = url;
    let (sender, input) = mpsc::channel(1);
    sender.send(vec![0; 3200]).await.unwrap();
    drop(sender);
    assert_eq!(
        timeout(
            Duration::from_secs(5),
            listen.transcribe_live(input, INPUT_RATE)
        )
        .await
        .unwrap()
        .unwrap(),
        "hello"
    );
}
async fn event(socket: &mut WebSocketStream<TcpStream>, body: Value) {
    socket
        .send(Message::Text(body.to_string().into()))
        .await
        .unwrap();
}

#[tokio::test]
async fn native_batch_stt_uses_its_route_and_model_before_the_file() {
    let (url, requests, _server) = answering(
        StatusCode::OK,
        "application/json",
        br#"{"text":"  Hello desk.  "}"#.to_vec(),
    )
    .await;
    let words = listener(&url)
        .transcribe(Clip {
            mime: "audio/wav".into(),
            bytes: b"RIFF-test".to_vec(),
        })
        .await
        .unwrap();
    assert_eq!(words, "Hello desk.");
    let request = requests.lock().unwrap()[0].clone();
    assert_eq!(request.path, "/v1/stt");
    assert_eq!(request.headers[header::AUTHORIZATION], "Bearer test-key");
    let body = String::from_utf8_lossy(&request.body);
    assert!(body.find(LISTEN_MODEL).unwrap() < body.find("RIFF-test").unwrap());
}

#[tokio::test]
async fn live_stt_waits_for_ready_sends_pcm_in_order_and_returns_only_the_full_final() {
    let expected: Vec<u8> = (0..8000).map(|n| (n % 251) as u8).collect();
    let wanted = expected.clone();
    let (finished_tx, finished_rx) = oneshot::channel();
    let (url, requests, _server) = websocket(move |mut socket| async move {
        assert!(
            timeout(Duration::from_millis(40), socket.next())
                .await
                .is_err(),
            "audio was sent before transcript.created"
        );
        event(&mut socket, json!({"type":"transcript.created"})).await;
        event(
            &mut socket,
            json!({"type":"transcript.partial","text":"wrong partial","is_final":true}),
        )
        .await;
        let mut received = vec![];
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Binary(bytes) => {
                    assert!(bytes.len() <= PCM_FRAME_BYTES);
                    assert_eq!(bytes.len() % 2, 0);
                    received.extend_from_slice(&bytes);
                }
                Message::Text(text) => {
                    assert_eq!(
                        serde_json::from_str::<Value>(&text).unwrap(),
                        json!({"type":"audio.done"})
                    );
                    break;
                }
                other => panic!("unexpected client message {other:?}"),
            }
        }
        assert_eq!(received, wanted);
        event(
            &mut socket,
            json!({"type":"transcript.done","text":"  Full transcript.  ","duration":0.25}),
        )
        .await;
        finished_tx.send(()).unwrap();
    })
    .await;
    let voice = listener(&url);
    let (tx, rx) = mpsc::channel(2);
    tx.send(expected[..6400].to_vec()).await.unwrap();
    tx.send(expected[6400..].to_vec()).await.unwrap();
    drop(tx);
    assert_eq!(
        voice.transcribe_live(rx, INPUT_RATE).await.unwrap(),
        "Full transcript."
    );
    finished_rx.await.unwrap();
    let request = requests.lock().unwrap()[0].clone();
    assert_eq!(request.headers[header::AUTHORIZATION], "Bearer test-key");
    let url = url::Url::parse(&format!("http://localhost{}", request.path)).unwrap();
    assert_eq!(url.path(), "/v1/stt");
    let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
    assert_eq!(params["encoding"], "pcm");
    assert_eq!(params["sample_rate"], "16000");
    assert_eq!(params["channels"], "1");
    assert_eq!(params["model"], LISTEN_MODEL);
}

async fn finalized_partial_words(partials: Vec<Value>, done: &str) -> String {
    let done = done.to_string();
    let (partials_tx, partials_rx) = oneshot::channel();
    let (url, _, _server) = websocket(move |mut socket| async move {
        event(&mut socket, json!({"type":"transcript.created"})).await;
        assert!(matches!(
            socket.next().await.unwrap().unwrap(),
            Message::Binary(_)
        ));
        for partial in partials {
            event(&mut socket, partial).await;
        }
        partials_tx.send(()).unwrap();
        assert!(
            socket
                .next()
                .await
                .unwrap()
                .unwrap()
                .to_text()
                .unwrap()
                .contains("audio.done")
        );
        event(&mut socket, json!({"type":"transcript.done", "text":done})).await;
    })
    .await;
    let (sender, input) = mpsc::channel(1);
    sender.send(vec![0; 3200]).await.unwrap();
    let voice = listener(&url);
    let task = tokio::spawn(async move { voice.transcribe_live(input, INPUT_RATE).await });
    partials_rx.await.unwrap();
    tokio::task::yield_now().await;
    assert!(
        !task.is_finished(),
        "a finalized partial must still wait for audio.done and transcript.done"
    );
    drop(sender);
    timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn empty_done_retains_finalized_segments_and_stitched_utterances_without_range_replays() {
    let utterance = json!({"type":"transcript.partial", "text":"Hello desk.", "is_final":true,
        "speech_final":true, "start":0.0, "duration":2.0});
    let partials = vec![
        json!({"type":"transcript.partial", "text":"Hello", "is_final":true, "start":0.0, "duration":1.0}),
        json!({"type":"transcript.partial", "text":"Hello", "is_final":true, "start":0.0, "duration":1.0}),
        json!({"type":"transcript.partial", "text":"desk.", "is_final":true, "start":1.0, "duration":1.0}),
        utterance.clone(),
        json!({"type":"transcript.partial", "text":"Again.", "is_final":true, "start":2.0, "duration":1.0}),
        // A replay of an earlier final must preserve the new pending chunk.
        utterance,
        json!({"type":"transcript.partial", "text":"uncommitted noise", "is_final":false}),
    ];
    assert_eq!(
        finalized_partial_words(partials.clone(), "").await,
        "Hello desk. Again."
    );
    assert_eq!(
        finalized_partial_words(partials, "Authoritative full transcript.").await,
        "Authoritative full transcript."
    );
}

#[tokio::test]
async fn interim_only_words_are_never_promoted_when_done_is_empty() {
    let partials = vec![
        json!({"type":"transcript.partial", "text":"a beep", "is_final":false, "speech_final":false}),
        json!({"type":"transcript.partial", "text":"unmarked words"}),
    ];
    assert_eq!(finalized_partial_words(partials, "").await, "");
}

#[test]
fn finalized_transcript_is_bounded_and_word_repetition_alone_is_not_a_replay() {
    let mut finals = FinalTranscript::default();
    for _ in 0..2 {
        finals
            .accept(&json!({"text":"yes", "speech_final":true}))
            .unwrap();
    }
    assert_eq!(finals.words(), "yes yes");
    let mut finals = FinalTranscript::default();
    finals
        .accept(&json!({"text":"x".repeat(TRANSCRIPT_LIMIT), "is_final":true}))
        .unwrap();
    assert!(
        finals
            .accept(&json!({"text":"y", "is_final":true}))
            .is_err()
    );
    let mut finals = FinalTranscript::default();
    finals
        .accept(&json!({"text":"chunk", "is_final":true}))
        .unwrap();
    finals
        .accept(&json!({"text":"Stitched utterance.", "speech_final":true}))
        .unwrap();
    assert_eq!(finals.words(), "Stitched utterance.");
    assert_eq!(
        finals
            .finish(&json!({"type":"transcript.done", "duration":1.0}))
            .unwrap(),
        "Stitched utterance."
    );
    assert!(finals.finish(&json!({"text":false})).is_err());
}

#[tokio::test]
async fn live_stt_validates_rate_and_job_before_connecting() {
    let voice = listener("http://127.0.0.1:1/v1");
    let (_tx, rx) = mpsc::channel(1);
    assert!(matches!(
        voice.transcribe_live(rx, 48_000).await,
        Err(SpeechError::UnsupportedFormat(_))
    ));
    let voice = speaker("http://127.0.0.1:1/v1");
    assert!(!voice.supports_live_input());
    let (_tx, rx) = mpsc::channel(1);
    assert_eq!(
        voice.transcribe_live(rx, INPUT_RATE).await,
        Err(SpeechError::WrongJob)
    );
}

#[test]
fn live_pcm_bounds_refuse_odd_oversized_empty_and_excess_audio_without_accepting_it() {
    let voice = listener("http://127.0.0.1:1/v1");
    for frame in [vec![], vec![0; 3], vec![0; INPUT_FRAME_LIMIT + 2]] {
        let (mut bytes, mut frames) = (0, 0);
        assert_eq!(
            voice.accept_frame(&frame, &mut bytes, &mut frames),
            Err(malformed())
        );
        assert_eq!((bytes, frames), (0, 0));
    }
    let (mut bytes, mut frames) = (INPUT_LIMIT - 2, 1);
    assert_eq!(
        voice.accept_frame(&[0; 4], &mut bytes, &mut frames),
        Err(malformed())
    );
    assert_eq!((bytes, frames), (INPUT_LIMIT - 2, 1));
    voice
        .accept_frame(&[0; 2], &mut bytes, &mut frames)
        .unwrap();
    assert_eq!(bytes, INPUT_LIMIT);
    assert_eq!(
        voice.accept_frame(&[0; 2], &mut bytes, &mut frames),
        Err(malformed())
    );
    let (mut bytes, mut frames) = (0, EVENT_LIMIT);
    assert_eq!(
        voice.accept_frame(&[0; 2], &mut bytes, &mut frames),
        Err(malformed())
    );
}

#[tokio::test]
async fn malformed_live_frames_are_not_forwarded_to_the_provider() {
    let (closed_tx, closed_rx) = oneshot::channel();
    let (url, _, _server) = websocket(move |mut socket| async move {
        event(&mut socket, json!({"type":"transcript.created"})).await;
        assert!(!matches!(socket.next().await, Some(Ok(Message::Binary(_)))));
        closed_tx.send(()).unwrap();
    })
    .await;
    let (tx, rx) = mpsc::channel(1);
    tx.send(vec![0; 3]).await.unwrap();
    drop(tx);
    assert_eq!(
        listener(&url).transcribe_live(rx, INPUT_RATE).await,
        Err(malformed())
    );
    timeout(Duration::from_secs(2), closed_rx)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn provider_errors_and_out_of_order_finals_never_leak_text() {
    for body in [
        json!({"type":"error","message":"secret spoken text and credential"}),
        json!({"type":"transcript.done","text":"unsolicited final"}),
    ] {
        let (url, _, _server) = websocket(move |mut socket| async move {
            event(&mut socket, json!({"type":"transcript.created"})).await;
            event(&mut socket, body).await;
        })
        .await;
        let (_tx, rx) = mpsc::channel(1);
        let error = listener(&url)
            .transcribe_live(rx, INPUT_RATE)
            .await
            .unwrap_err();
        assert_eq!(error, malformed());
        assert!(!error.to_string().contains("secret"));
    }
}

#[tokio::test]
async fn cancelling_live_transcription_closes_the_socket_without_a_detached_worker() {
    let (sent_tx, sent_rx) = oneshot::channel();
    let (closed_tx, closed_rx) = oneshot::channel();
    let (url, _, _server) = websocket(move |mut socket| async move {
        event(&mut socket, json!({"type":"transcript.created"})).await;
        assert!(matches!(socket.next().await, Some(Ok(Message::Binary(_)))));
        sent_tx.send(()).unwrap();
        assert!(!matches!(socket.next().await, Some(Ok(Message::Binary(_)))));
        closed_tx.send(()).unwrap();
    })
    .await;
    let voice = listener(&url);
    let (tx, rx) = mpsc::channel(1);
    tx.send(vec![0; PCM_FRAME_BYTES]).await.unwrap();
    let task = tokio::spawn(async move { voice.transcribe_live(rx, INPUT_RATE).await });
    timeout(Duration::from_secs(2), sent_rx)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    timeout(Duration::from_secs(2), closed_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(tx.is_closed());
}

#[tokio::test]
async fn a_provider_that_never_becomes_ready_hits_the_connection_deadline() {
    let (url, _, _server) = websocket(move |mut socket| async move {
        let _ = socket.next().await;
    })
    .await;
    let (_tx, rx) = mpsc::channel(1);
    assert_eq!(
        timeout(
            CONNECT_TIMEOUT + Duration::from_secs(2),
            listener(&url).transcribe_live(rx, INPUT_RATE)
        )
        .await
        .unwrap(),
        Err(SpeechError::Unreachable {
            provider_id: PROVIDER_ID.into()
        })
    );
}

#[tokio::test]
async fn native_tts_asks_for_pcm_and_wraps_a_whole_clip_without_sending_a_model_selector() {
    let (url, requests, _server) = answering(StatusCode::OK, "audio/pcm", vec![1, 0, 2, 0]).await;
    let clip = speaker(&url).speak("Hello desk.").await.unwrap();
    assert_eq!(
        clip,
        Clip {
            mime: "audio/wav".into(),
            bytes: wav::pcm16_wav(&[1, 0, 2, 0], OUTPUT_RATE)
        }
    );
    let request = requests.lock().unwrap()[0].clone();
    assert_eq!(request.path, "/v1/tts");
    assert_eq!(request.headers[header::AUTHORIZATION], "Bearer test-key");
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        json!({"text":"Hello desk.","voice_id":"eve","language":"auto","output_format":{"codec":"pcm","sample_rate":24000}})
    );
}

#[tokio::test]
async fn streamed_tts_publishes_200ms_without_waiting_for_more_audio_or_body_completion() {
    let (body_tx, body_rx) = mpsc::channel::<Bytes>(4);
    let body_rx = Arc::new(Mutex::new(Some(body_rx)));
    let app = Router::new().fallback(move || {
        let rx = body_rx.lock().unwrap().take().unwrap();
        async move {
            let body = Body::from_stream(stream::unfold(rx, |mut rx| async move {
                rx.recv()
                    .await
                    .map(|bytes| (Ok::<_, std::io::Error>(bytes), rx))
            }));
            ([(header::CONTENT_TYPE, "audio/pcm")], body)
        }
    });
    let (url, _server) = http_server(app).await;
    let pcm: Vec<u8> = (0..FIRST_CLIP_PCM_BYTES + 2)
        .map(|n| (n % 251) as u8)
        .collect();
    let voice = speaker(&url);
    let (clips_tx, mut clips_rx) = mpsc::channel(1);
    let task = tokio::spawn(async move { voice.speak_chunks("Hello desk.", clips_tx).await });
    // A network fragment may end halfway through a PCM16 sample.
    for part in [
        &pcm[..3],
        &pcm[3..FIRST_CLIP_PCM_BYTES + 1],
        &pcm[FIRST_CLIP_PCM_BYTES + 1..],
    ] {
        body_tx.send(Bytes::copy_from_slice(part)).await.unwrap();
    }
    let first = timeout(Duration::from_secs(2), clips_rx.recv())
        .await
        .expect("the first clip must not wait for the withheld body tail")
        .unwrap();
    assert_eq!(first.clip.mime, "audio/wav");
    assert_eq!(
        first.clip.bytes,
        wav::pcm16_wav(&pcm[..FIRST_CLIP_PCM_BYTES], OUTPUT_RATE)
    );
    assert_eq!(
        (first.clip.bytes.len() - 44) * 1000 / (OUTPUT_RATE as usize * 2),
        200
    );
    assert!(!first.final_chunk);
    assert!(!task.is_finished(), "the provider body is still open");
    drop(body_tx);
    let last = timeout(Duration::from_secs(2), clips_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        last.clip.bytes,
        wav::pcm16_wav(&pcm[FIRST_CLIP_PCM_BYTES..], OUTPUT_RATE)
    );
    assert!(last.final_chunk);
    task.await.unwrap().unwrap();
    assert!(clips_rx.recv().await.is_none());
}

#[tokio::test]
async fn tts_short_and_exact_startup_length_bodies_keep_one_nonempty_final_clip() {
    for size in [2, FIRST_CLIP_PCM_BYTES] {
        let pcm = vec![7; size];
        let (url, _, _server) = answering(StatusCode::OK, "audio/pcm", pcm.clone()).await;
        let (tx, mut rx) = mpsc::channel(1);
        speaker(&url).speak_chunks("Hello.", tx).await.unwrap();
        let chunk = rx.recv().await.unwrap();
        assert_eq!(chunk.clip.bytes, wav::pcm16_wav(&pcm, OUTPUT_RATE));
        assert!(chunk.final_chunk);
        assert!(rx.recv().await.is_none());
    }
}

#[tokio::test]
async fn streamed_tts_reblocks_network_fragments_into_ordered_wavs_before_the_response_ends() {
    let (body_tx, body_rx) = mpsc::channel::<Bytes>(4);
    let body_rx = Arc::new(Mutex::new(Some(body_rx)));
    let app = Router::new().fallback(move || {
        let rx = body_rx.lock().unwrap().take().unwrap();
        async move {
            let body = Body::from_stream(stream::unfold(rx, |mut rx| async move {
                rx.recv()
                    .await
                    .map(|bytes| (Ok::<_, std::io::Error>(bytes), rx))
            }));
            ([(header::CONTENT_TYPE, "audio/pcm")], body)
        }
    });
    let (url, _server) = http_server(app).await;
    let pcm: Vec<u8> = (0..96_010).map(|n| (n % 251) as u8).collect();
    let voice = speaker(&url);
    let (clips_tx, mut clips_rx) = mpsc::channel(1);
    let task = tokio::spawn(async move { voice.speak_chunks("Hello desk.", clips_tx).await });
    // Include odd network boundaries: they are not PCM frame boundaries.
    for part in [&pcm[..3], &pcm[3..8000], &pcm[8000..48_000]] {
        body_tx.send(Bytes::copy_from_slice(part)).await.unwrap();
    }
    let first = timeout(Duration::from_secs(2), clips_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&first.clip.bytes[44..], &pcm[..FIRST_CLIP_PCM_BYTES]);
    assert!(!first.final_chunk);
    assert!(!task.is_finished(), "the provider body is still open");
    body_tx
        .send(Bytes::copy_from_slice(&pcm[48_000..]))
        .await
        .unwrap();
    drop(body_tx);
    let mut clips = vec![first];
    while let Some(clip) = timeout(Duration::from_secs(2), clips_rx.recv())
        .await
        .unwrap()
    {
        clips.push(clip);
    }
    task.await.unwrap().unwrap();
    assert_eq!(
        clips
            .iter()
            .map(|chunk| chunk.final_chunk)
            .collect::<Vec<_>>(),
        vec![false, false, false, true]
    );
    let clips: Vec<_> = clips.into_iter().map(|chunk| chunk.clip).collect();
    assert_eq!(
        clips
            .iter()
            .map(|clip| clip.bytes.len() - 44)
            .collect::<Vec<_>>(),
        vec![
            FIRST_CLIP_PCM_BYTES,
            24_000,
            24_000,
            pcm.len() - FIRST_CLIP_PCM_BYTES - 48_000,
        ]
    );
    for clip in &clips {
        assert_eq!(clip.mime, "audio/wav");
        assert_eq!((clip.bytes.len() - 44) % 2, 0);
        assert_eq!(&clip.bytes[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(clip.bytes[24..28].try_into().unwrap()),
            OUTPUT_RATE
        );
        assert_eq!(
            u32::from_le_bytes(clip.bytes[40..44].try_into().unwrap()) as usize,
            clip.bytes.len() - 44
        );
    }
    assert_eq!(
        clips
            .into_iter()
            .flat_map(|clip| clip.bytes.into_iter().skip(44))
            .collect::<Vec<_>>(),
        pcm
    );
}

#[tokio::test]
async fn the_bounded_output_applies_backpressure_and_closing_it_cancels_tts() {
    let (url, _, _server) =
        answering(StatusCode::OK, "audio/pcm", vec![0; CLIP_PCM_BYTES * 4]).await;
    let voice = speaker(&url);
    let (tx, rx) = mpsc::channel(1);
    let mut task = tokio::spawn(async move { voice.speak_chunks("Hello desk.", tx).await });
    assert!(
        timeout(Duration::from_millis(80), &mut task).await.is_err(),
        "synthesis must wait for the full output channel"
    );
    assert_eq!(rx.len(), 1);
    drop(rx);
    assert_eq!(
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap(),
        Err(SpeechError::Cancelled)
    );
}

#[tokio::test]
async fn cancellation_also_stops_waiting_for_the_rest_of_a_provider_body() {
    let (body_tx, body_rx) = mpsc::channel::<Bytes>(1);
    let body_rx = Arc::new(Mutex::new(Some(body_rx)));
    let app = Router::new().fallback(move || {
        let rx = body_rx.lock().unwrap().take().unwrap();
        async move {
            let body = Body::from_stream(stream::unfold(rx, |mut rx| async move {
                rx.recv()
                    .await
                    .map(|bytes| (Ok::<_, std::io::Error>(bytes), rx))
            }));
            ([(header::CONTENT_TYPE, "audio/pcm")], body)
        }
    });
    let (url, _server) = http_server(app).await;
    let voice = speaker(&url);
    let (tx, mut rx) = mpsc::channel(1);
    let task = tokio::spawn(async move { voice.speak_chunks("Hello desk.", tx).await });
    body_tx
        .send(Bytes::from(vec![0; FIRST_CLIP_PCM_BYTES + 2]))
        .await
        .unwrap();
    timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    drop(rx);
    assert_eq!(
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap(),
        Err(SpeechError::Cancelled)
    );
    // The response never ends until this sender is dropped, so completion
    // above proves receiver cancellation interrupts a pending body read.
    drop(body_tx);
}

#[tokio::test]
async fn native_tts_never_interprets_mp3_or_partial_samples_as_pcm() {
    for (mime, body) in [
        ("audio/mpeg", vec![1, 2]),
        ("audio/pcm", vec![1, 2, 3]),
        ("audio/pcm", vec![]),
    ] {
        let (url, _, _server) = answering(StatusCode::OK, mime, body).await;
        assert_eq!(speaker(&url).speak("Hello.").await, Err(malformed()));
        let (tx, mut rx) = mpsc::channel(1);
        assert_eq!(
            speaker(&url).speak_chunks("Hello.", tx).await,
            Err(malformed())
        );
        assert!(rx.recv().await.is_none());
    }
}

#[tokio::test]
async fn a_late_pcm_error_never_marks_an_incomplete_utterance_final() {
    let (url, _, _server) =
        answering(StatusCode::OK, "audio/pcm", vec![0; CLIP_PCM_BYTES * 2 + 1]).await;
    let voice = speaker(&url);
    let (tx, mut rx) = mpsc::channel(2);
    assert_eq!(voice.speak_chunks("Hello.", tx).await, Err(malformed()));
    let chunk = rx.recv().await.unwrap();
    assert_eq!(chunk.clip.bytes.len() - 44, FIRST_CLIP_PCM_BYTES);
    assert!(!chunk.final_chunk);
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn native_refusals_are_redacted_and_redirects_are_not_followed() {
    let (url, requests, _server) = answering(
        StatusCode::UNAUTHORIZED,
        "text/plain",
        b"secret text and credential".to_vec(),
    )
    .await;
    let error = speaker(&url).speak("Hello.").await.unwrap_err();
    assert_eq!(
        error,
        SpeechError::Refused {
            provider_id: PROVIDER_ID.into(),
            status: 401
        }
    );
    assert!(!error.to_string().contains("secret"));
    assert_eq!(requests.lock().unwrap().len(), 1);
    let app = Router::new().fallback(|| async {
        (
            StatusCode::TEMPORARY_REDIRECT,
            [(header::LOCATION, "http://127.0.0.1:1/steal")],
            "secret body",
        )
    });
    let (url, _server) = http_server(app).await;
    assert_eq!(
        speaker(&url).speak("Hello.").await,
        Err(SpeechError::Refused {
            provider_id: PROVIDER_ID.into(),
            status: 307
        })
    );
}

#[tokio::test]
async fn other_providers_keep_one_whole_clip_and_the_shared_pool_keeps_request_keys_separate() {
    let (url, requests, _server) = answering(StatusCode::OK, "audio/pcm", vec![1, 0]).await;
    speaker(&url).speak("Hello.").await.unwrap();
    let endpoint = Endpoint {
        provider_id: "openrouter".into(),
        base_url: url,
        key: Some("another-key".into()),
    };
    let voice = super::super::OpenAiShape::speaker(
        endpoint,
        "test-tts",
        "test-voice",
        super::super::AudioFormat::Pcm,
        4096,
    )
    .unwrap();
    assert!(!voice.supports_live_input());
    let (_tx, rx) = mpsc::channel(1);
    assert!(matches!(
        voice.transcribe_live(rx, INPUT_RATE).await,
        Err(SpeechError::UnsupportedFormat(_))
    ));
    let (tx, mut rx) = mpsc::channel(1);
    voice.speak_chunks("Hello.", tx).await.unwrap();
    let chunk = rx.recv().await.unwrap();
    assert_eq!(chunk.clip.mime, "audio/wav");
    assert!(chunk.final_chunk);
    assert!(rx.recv().await.is_none());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].headers[header::AUTHORIZATION],
        "Bearer test-key"
    );
    assert_eq!(
        requests[1].headers[header::AUTHORIZATION],
        "Bearer another-key"
    );
}
