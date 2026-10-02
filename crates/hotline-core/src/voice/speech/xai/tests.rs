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
fn malformed() -> SpeechError {
    SpeechError::Malformed {
        provider_id: PROVIDER_ID.into(),
    }
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
    assert_eq!(&first.clip.bytes[44..], &pcm[..24_000]);
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
        vec![24_000, 24_000, 24_000, 24_010]
    );
    for clip in &clips {
        assert_eq!(clip.mime, "audio/wav");
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
        .send(Bytes::from(vec![0; CLIP_PCM_BYTES * 2]))
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
    assert_eq!(chunk.clip.bytes.len() - 44, CLIP_PCM_BYTES);
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
