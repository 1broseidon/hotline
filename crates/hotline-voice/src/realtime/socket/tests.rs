use super::*;
use crate::realtime::RealtimeTool;
use crate::speech::xai::{BASE_URL, PROVIDER_ID, test_login};
use http::{HeaderMap, header};
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::{
    WebSocketStream, accept_hdr_async,
    tungstenite::handshake::server::{Request, Response},
};

type Fixture = WebSocketStream<TcpStream>;

/// What one handshake asked for.
#[derive(Clone, Default)]
struct Seen {
    path: String,
    headers: HeaderMap,
}

async fn listen() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    (listener, url)
}

// Tungstenite requires its HTTP response as this callback's error type.
#[expect(clippy::result_large_err, reason = "tungstenite fixes this error type")]
async fn accept(listener: &TcpListener) -> (Fixture, Seen) {
    let (socket, _) = listener.accept().await.unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let record = seen.clone();
    let socket = accept_hdr_async(socket, move |request: &Request, response: Response| {
        *record.lock().unwrap() = Seen {
            path: request.uri().to_string(),
            headers: request.headers().clone(),
        };
        Ok(response)
    })
    .await
    .unwrap();
    let seen = seen.lock().unwrap().clone();
    (socket, seen)
}

async fn send(socket: &mut Fixture, event: Value) {
    socket
        .send(Message::Text(event.to_string().into()))
        .await
        .unwrap();
}

async fn receive(socket: &mut Fixture) -> Value {
    loop {
        match timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("expected an event, got {other:?}"),
        }
    }
}

/// Accepts the client's config and returns it.
async fn handshake(socket: &mut Fixture) -> Value {
    let update = receive(socket).await;
    assert_eq!(update["type"], "session.update");
    send(socket, json!({"type": "session.updated"})).await;
    update["session"].clone()
}

/// The client closed this connection.
async fn closed(socket: &mut Fixture) {
    loop {
        match timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
        {
            None | Some(Err(_) | Ok(Message::Close(_))) => return,
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(other)) => panic!("expected the close, got {other:?}"),
        }
    }
}

fn config() -> RealtimeConfig {
    RealtimeConfig {
        instructions: "You are the front.".into(),
        voice: "eve".into(),
        tools: vec![RealtimeTool {
            name: "work_on_it".into(),
            description: "Give the teammate the caller's words.".into(),
            parameters: json!({"type": "object", "properties": {"words": {"type": "string"}}}),
        }],
        input_rate: 16_000,
        output_rate: 24_000,
    }
}

fn voice(url: &str) -> RealtimeSocket {
    RealtimeSocket::grok(Endpoint {
        provider_id: PROVIDER_ID.into(),
        base_url: url.into(),
        key: Some("test-key".into()),
    })
}

async fn next(session: &mut RealtimeSession) -> RealtimeEvent {
    timeout(Duration::from_secs(5), session.events.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

fn pcm(samples: &[u8]) -> String {
    STANDARD.encode(samples)
}

async fn finish(server: JoinHandle<()>) {
    timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn a_session_applies_its_config_and_carries_audio_tools_and_barge_in() {
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, seen) = accept(&listener).await;
        assert_eq!(seen.path, "/v1/realtime?model=grok-voice-latest");
        assert_eq!(seen.headers[header::AUTHORIZATION], "Bearer test-key");
        // Events before the config is applied are not the session yet.
        send(&mut socket, json!({"type": "session.created"})).await;
        let session = handshake(&mut socket).await;
        assert_eq!(session["instructions"], "You are the front.");
        assert_eq!(session["voice"], "eve");
        assert_eq!(session["turn_detection"], json!({"type": "server_vad"}));
        assert_eq!(
            session["audio"],
            json!({
                "input": {"format": {"type": "audio/pcm", "rate": 16000}},
                "output": {"format": {"type": "audio/pcm", "rate": 24000}},
            })
        );
        assert_eq!(session["tools"][0]["type"], "function");
        assert_eq!(session["tools"][0]["name"], "work_on_it");
        assert_eq!(session["tools"][0]["parameters"]["type"], "object");

        let append = receive(&mut socket).await;
        assert_eq!(append["type"], "input_audio_buffer.append");
        assert_eq!(append["audio"], pcm(&[1, 0, 2, 0]));

        send(&mut socket, json!({"type": "response.created"})).await;
        send(
            &mut socket,
            json!({"type": "response.output_audio.delta", "item_id": "item_1", "delta": pcm(&[5, 0, 6, 0])}),
        )
        .await;
        send(
            &mut socket,
            json!({"type": "response.output_audio_transcript.delta", "delta": "Hello"}),
        )
        .await;
        send(
            &mut socket,
            json!({"type": "input_audio_buffer.speech_started"}),
        )
        .await;
        send(&mut socket, json!({"type": "response.done"})).await;

        let mut truncate = receive(&mut socket).await;
        assert!(
            truncate
                .as_object_mut()
                .unwrap()
                .remove("event_id")
                .is_some()
        );
        assert_eq!(
            truncate,
            json!({"type": "conversation.item.truncate", "item_id": "item_1", "content_index": 0, "audio_end_ms": 120})
        );

        send(
            &mut socket,
            json!({"type": "input_audio_buffer.speech_stopped"}),
        )
        .await;
        send(&mut socket, json!({"type": "input_audio_buffer.committed"})).await;
        send(
            &mut socket,
            json!({"type": "conversation.item.input_audio_transcription.completed", "transcript": " Fix the build. "}),
        )
        .await;
        send(&mut socket, json!({"type": "response.created"})).await;
        send(
            &mut socket,
            json!({"type": "response.function_call_arguments.done", "call_id": "c1", "name": "work_on_it", "arguments": "{\"words\":\"Fix the build.\"}"}),
        )
        .await;
        // The result arrives mid-reply: the next reply waits for this one.
        let mut result = receive(&mut socket).await;
        assert!(result.as_object_mut().unwrap().remove("event_id").is_some());
        assert_eq!(
            result,
            json!({"type": "conversation.item.create", "item": {"type": "function_call_output", "call_id": "c1", "output": "{\"ok\":true}"}})
        );
        send(&mut socket, json!({"type": "response.done"})).await;
        assert_eq!(receive(&mut socket).await["type"], "response.create");
        send(&mut socket, json!({"type": "response.created"})).await;
        send(&mut socket, json!({"type": "response.done"})).await;

        let text = receive(&mut socket).await;
        assert_eq!(text["item"]["role"], "user");
        assert_eq!(text["item"]["content"][0]["text"], "The tests pass.");
        assert_eq!(receive(&mut socket).await["type"], "response.create");
        send(&mut socket, json!({"type": "response.created"})).await;
        send(&mut socket, json!({"type": "response.done"})).await;
        closed(&mut socket).await;
    });

    let grok = voice(&url);
    assert_eq!(grok.id().model_id, GROK_MODEL);
    assert!(!grok.is_subscription());
    let mut session = grok.open(config()).await.unwrap();
    session.input.push_audio(vec![1, 0, 2, 0]).await.unwrap();
    assert_eq!(
        next(&mut session).await,
        RealtimeEvent::Audio(vec![5, 0, 6, 0])
    );
    assert_eq!(
        next(&mut session).await,
        RealtimeEvent::Saying("Hello".into())
    );
    assert_eq!(next(&mut session).await, RealtimeEvent::SpeechStarted);
    assert_eq!(next(&mut session).await, RealtimeEvent::ResponseDone);
    session.input.truncate(120).await.unwrap();
    assert_eq!(
        next(&mut session).await,
        RealtimeEvent::Heard("Fix the build.".into())
    );
    assert_eq!(
        next(&mut session).await,
        RealtimeEvent::ToolCall {
            call_id: "c1".into(),
            name: "work_on_it".into(),
            arguments: "{\"words\":\"Fix the build.\"}".into(),
        }
    );
    session
        .input
        .tool_result("c1", "{\"ok\":true}", true)
        .await
        .unwrap();
    assert_eq!(next(&mut session).await, RealtimeEvent::ResponseDone);
    assert_eq!(next(&mut session).await, RealtimeEvent::ResponseDone);
    session.input.text("The tests pass.").await.unwrap();
    assert_eq!(next(&mut session).await, RealtimeEvent::ResponseDone);
    session.close().await;
    finish(server).await;
}

/// `remain_silent` is answered so the model is not left waiting, but asking
/// for a reply to it would have the model speak after choosing not to.
#[tokio::test]
async fn an_empty_tool_result_asks_for_no_reply() {
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, _) = accept(&listener).await;
        handshake(&mut socket).await;
        let output = receive(&mut socket).await;
        assert_eq!(output["item"]["type"], "function_call_output");
        assert_eq!(output["item"]["output"], "");
        assert_eq!(
            receive(&mut socket).await["type"],
            "input_audio_buffer.append"
        );
        closed(&mut socket).await;
    });
    let session = voice(&url).open(config()).await.unwrap();
    session.input.tool_result("c1", "", false).await.unwrap();
    session.input.push_audio(vec![0, 0]).await.unwrap();
    session.close().await;
    finish(server).await;
}

#[tokio::test]
async fn provider_errors_reach_the_caller_as_a_code_never_a_message() {
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, _) = accept(&listener).await;
        handshake(&mut socket).await;
        send(
            &mut socket,
            json!({"type": "error", "error": {"code": "invalid_value", "message": "you said: secret words"}}),
        )
        .await;
        send(
            &mut socket,
            json!({"type": "error", "error": {"code": "you said secret words", "message": "secret words"}}),
        )
        .await;
        // The session goes on after a refusal.
        assert_eq!(
            receive(&mut socket).await["type"],
            "input_audio_buffer.append"
        );
        closed(&mut socket).await;
    });
    let mut session = voice(&url).open(config()).await.unwrap();
    let first = next(&mut session).await;
    let second = next(&mut session).await;
    assert_eq!(
        first,
        RealtimeEvent::Refused {
            code: Some("invalid_value".into())
        }
    );
    assert_eq!(second, RealtimeEvent::Refused { code: None });
    assert!(!format!("{first:?}{second:?}").contains("secret"));
    session.input.push_audio(vec![0, 0]).await.unwrap();
    session.close().await;
    finish(server).await;
}

#[tokio::test]
async fn a_rejected_config_fails_open_without_the_providers_words() {
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, _) = accept(&listener).await;
        receive(&mut socket).await;
        send(
            &mut socket,
            json!({"type": "error", "error": {"code": "invalid_tool", "message": "secret words"}}),
        )
        .await;
    });
    let error = voice(&url).open(config()).await.err().unwrap();
    assert_eq!(
        error,
        SpeechError::Malformed {
            provider_id: PROVIDER_ID.into()
        }
    );
    finish(server).await;
}

// Tungstenite requires its HTTP response as this callback's error type.
#[expect(clippy::result_large_err, reason = "tungstenite fixes this error type")]
#[tokio::test]
async fn a_refused_handshake_is_a_status_without_its_body() {
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let refused = accept_hdr_async(socket, |_: &Request, _: Response| {
            Err(http::Response::builder()
                .status(403)
                .body(Some("secret words".into()))
                .unwrap())
        })
        .await;
        assert!(refused.is_err());
    });
    let error = voice(&url).open(config()).await.err().unwrap();
    assert_eq!(
        error,
        SpeechError::Refused {
            provider_id: PROVIDER_ID.into(),
            status: 403
        }
    );
    assert!(!error.to_string().contains("secret"));
    finish(server).await;
}

#[tokio::test]
async fn rates_the_provider_cannot_take_fail_before_connecting() {
    let mut odd = config();
    odd.output_rate = 11_025;
    // Nothing listens here; a connection attempt would be Unreachable.
    let error = voice("http://127.0.0.1:9/v1")
        .open(odd)
        .await
        .err()
        .unwrap();
    assert!(matches!(error, SpeechError::UnsupportedFormat(_)));
}

#[tokio::test]
async fn a_dropped_connection_reconnects_with_the_transcript_carried() {
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (mut first, _) = accept(&listener).await;
        handshake(&mut first).await;
        send(
            &mut first,
            json!({"type": "conversation.item.input_audio_transcription.completed", "transcript": "Check the tests."}),
        )
        .await;
        send(&mut first, json!({"type": "response.created"})).await;
        send(
            &mut first,
            json!({"type": "response.output_audio_transcript.delta", "delta": "On "}),
        )
        .await;
        send(
            &mut first,
            json!({"type": "response.output_audio_transcript.delta", "delta": "it."}),
        )
        .await;
        send(&mut first, json!({"type": "response.done"})).await;
        drop(first);

        let (mut second, seen) = accept(&listener).await;
        assert_eq!(seen.headers[header::AUTHORIZATION], "Bearer test-key");
        let session = handshake(&mut second).await;
        let instructions = session["instructions"].as_str().unwrap();
        assert!(instructions.starts_with("You are the front.\n\n"));
        assert!(instructions.ends_with("\nCaller: Check the tests.\nYou: On it."));
        assert_eq!(receive(&mut second).await["audio"], pcm(&[7, 0]));
        closed(&mut second).await;
    });
    let mut session = voice(&url).open(config()).await.unwrap();
    assert_eq!(
        next(&mut session).await,
        RealtimeEvent::Heard("Check the tests.".into())
    );
    assert_eq!(
        next(&mut session).await,
        RealtimeEvent::Saying("On ".into())
    );
    assert_eq!(
        next(&mut session).await,
        RealtimeEvent::Saying("it.".into())
    );
    assert_eq!(next(&mut session).await, RealtimeEvent::ResponseDone);
    assert_eq!(next(&mut session).await, RealtimeEvent::Reconnecting);
    assert_eq!(next(&mut session).await, RealtimeEvent::Renewed);
    session.input.push_audio(vec![7, 0]).await.unwrap();
    session.close().await;
    finish(server).await;
}

#[tokio::test]
async fn a_session_renews_before_the_providers_limit_and_closes_the_old_one() {
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (mut first, _) = accept(&listener).await;
        handshake(&mut first).await;
        let (mut second, _) = accept(&listener).await;
        let session = handshake(&mut second).await;
        // Nothing was said, so there is nothing to carry.
        assert_eq!(session["instructions"], "You are the front.");
        closed(&mut first).await;
        closed(&mut second).await;
    });
    let mut grok = voice(&url);
    grok.renew_after = Duration::from_millis(200);
    let mut session = grok.open(config()).await.unwrap();
    assert_eq!(next(&mut session).await, RealtimeEvent::Renewed);
    session.close().await;
    finish(server).await;
}

#[tokio::test]
async fn a_lost_provider_is_retried_then_reported_unreachable() {
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, _) = accept(&listener).await;
        handshake(&mut socket).await;
        drop(socket);
        drop(listener);
    });
    let mut session = voice(&url).open(config()).await.unwrap();
    assert_eq!(next(&mut session).await, RealtimeEvent::Reconnecting);
    let error = timeout(Duration::from_secs(10), session.events.recv())
        .await
        .unwrap()
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(
        error,
        SpeechError::Unreachable {
            provider_id: PROVIDER_ID.into()
        }
    );
    assert!(session.events.recv().await.is_none());
    finish(server).await;
}

// Tungstenite requires its HTTP response as this callback's error type.
#[expect(clippy::result_large_err, reason = "tungstenite fixes this error type")]
#[tokio::test]
async fn the_grok_login_refreshes_once_when_the_handshake_rejects_it() {
    let dir = tempfile::tempdir().unwrap();
    let tokens = hotline_support::credentials::CredentialFiles::new(
        dir.path().into(),
        std::sync::Arc::new(hotline_support::credentials::MemoryStore::default()),
    )
    .file(dir.path().join("auth.json"));
    crate::speech::xai::sign_in_for_test(&tokens, "first-login");
    let rotate = tokens.clone();
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let refused = accept_hdr_async(socket, |request: &Request, _: Response| {
            assert_eq!(
                request.headers()[header::AUTHORIZATION],
                "Bearer first-login"
            );
            // Another client already refreshed this login.
            crate::speech::xai::sign_in_for_test(&rotate, "fresh-login");
            Err(http::Response::builder().status(401).body(None).unwrap())
        })
        .await;
        assert!(refused.is_err());
        let (mut socket, seen) = accept(&listener).await;
        assert_eq!(seen.headers[header::AUTHORIZATION], "Bearer fresh-login");
        handshake(&mut socket).await;
        closed(&mut socket).await;
    });
    let mut grok = RealtimeSocket::grok(Endpoint {
        provider_id: PROVIDER_ID.into(),
        base_url: BASE_URL.into(),
        key: Some("unused-key".into()),
    })
    .with_subscription(test_login(tokens))
    .unwrap();
    // Only fixtures replace the production origin after the guarded constructor.
    grok.access.endpoint.base_url = url;
    assert!(grok.is_subscription());
    assert_eq!(grok.id().provider_id, "xai-subscription");
    grok.open(config()).await.unwrap().close().await;
    finish(server).await;
}

#[test]
fn a_subscription_login_is_only_sent_to_xai() {
    let dir = tempfile::tempdir().unwrap();
    let tokens = hotline_support::credentials::CredentialFiles::new(
        dir.path().into(),
        std::sync::Arc::new(hotline_support::credentials::MemoryStore::default()),
    )
    .file(dir.path().join("auth.json"));
    assert!(
        voice("https://example.com/v1")
            .with_subscription(test_login(tokens))
            .is_err()
    );
}

fn openai(url: &str) -> RealtimeSocket {
    RealtimeSocket::openai(Endpoint {
        provider_id: "openai".into(),
        base_url: url.into(),
        key: Some("test-key".into()),
    })
}

#[tokio::test]
async fn gpt_realtime_asks_for_its_model_and_puts_voice_turns_and_transcription_under_audio() {
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, seen) = accept(&listener).await;
        assert_eq!(seen.path, format!("/v1/realtime?model={OPENAI_MODEL}"));
        assert_eq!(seen.headers[header::AUTHORIZATION], "Bearer test-key");
        let session = handshake(&mut socket).await;
        assert_eq!(session["type"], "realtime");
        assert_eq!(session["instructions"], "You are the front.");
        assert_eq!(session["audio"]["output"]["voice"], "marin");
        assert_eq!(
            session["audio"]["input"]["format"],
            json!({"type": "audio/pcm", "rate": 24_000})
        );
        assert_eq!(
            session["audio"]["input"]["turn_detection"],
            json!({"type":"semantic_vad", "eagerness":"low"})
        );
        assert_eq!(
            session["audio"]["input"]["transcription"]["model"],
            OPENAI_TRANSCRIBE
        );
        assert!(session.get("voice").is_none() && session.get("turn_detection").is_none());
        assert_eq!(session["tools"][0]["name"], "work_on_it");

        // The same events as Grok's carry the call.
        send(
            &mut socket,
            json!({"type": "conversation.item.input_audio_transcription.completed", "transcript": "hello"}),
        )
        .await;
        send(
            &mut socket,
            json!({"type": "response.output_audio.delta", "item_id": "a1", "delta": pcm(&[1, 0])}),
        )
        .await;
        closed(&mut socket).await;
    });
    let mut config = config();
    config.input_rate = 24_000;
    config.voice = "marin".into();
    let gpt = openai(&url);
    assert_eq!(gpt.id().model_id, OPENAI_MODEL);
    assert!(!gpt.is_subscription());
    let mut session = gpt.open(config).await.unwrap();
    assert_eq!(
        next(&mut session).await,
        RealtimeEvent::Heard("hello".into())
    );
    assert_eq!(next(&mut session).await, RealtimeEvent::Audio(vec![1, 0]));
    session.close().await;
    finish(server).await;
}

#[tokio::test]
async fn gpt_realtime_takes_only_24_khz_and_never_a_grok_login() {
    // 16 kHz suits Grok; OpenAI refuses it before connecting.
    let error = openai("http://127.0.0.1:9/v1")
        .open(config())
        .await
        .err()
        .unwrap();
    assert!(matches!(error, SpeechError::UnsupportedFormat(_)));
    let dir = tempfile::tempdir().unwrap();
    let tokens = hotline_support::credentials::CredentialFiles::new(
        dir.path().into(),
        std::sync::Arc::new(hotline_support::credentials::MemoryStore::default()),
    )
    .file(dir.path().join("auth.json"));
    assert!(
        openai(BASE_URL)
            .with_subscription(test_login(tokens))
            .is_err()
    );
}

#[tokio::test]
async fn gpt_realtime_takes_a_line_to_say_as_a_system_item_and_replies_without_tools() {
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, _) = accept(&listener).await;
        handshake(&mut socket).await;
        let item = receive(&mut socket).await;
        assert_eq!(item["item"]["role"], "system");
        let create = receive(&mut socket).await;
        assert_eq!(create["type"], "response.create");
        assert_eq!(create["response"]["tool_choice"], "none");
        closed(&mut socket).await;
    });
    let mut config = config();
    config.input_rate = 24_000;
    let session = openai(&url).open(config).await.unwrap();
    session
        .input
        .text("Say exactly this to the person: The checks passed.")
        .await
        .unwrap();
    session.close().await;
    finish(server).await;
}

#[tokio::test]
async fn a_refused_reply_does_not_block_the_next() {
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, _) = accept(&listener).await;
        handshake(&mut socket).await;
        assert_eq!(
            receive(&mut socket).await["item"]["type"],
            "function_call_output"
        );
        assert_eq!(receive(&mut socket).await["type"], "response.create");
        send(
            &mut socket,
            json!({"type": "error", "error": {"code": "rate_limit_exceeded", "message": "slow down"}}),
        )
        .await;
        // The refusal left no reply in progress: the next result asks for one at once.
        assert_eq!(
            receive(&mut socket).await["item"]["type"],
            "function_call_output"
        );
        assert_eq!(receive(&mut socket).await["type"], "response.create");
        closed(&mut socket).await;
    });
    let mut session = voice(&url).open(config()).await.unwrap();
    session.input.tool_result("c1", "done", true).await.unwrap();
    assert_eq!(
        next(&mut session).await,
        RealtimeEvent::Refused {
            code: Some("rate_limit_exceeded".into())
        }
    );
    session.input.tool_result("c2", "more", true).await.unwrap();
    session.close().await;
    finish(server).await;
}

#[tokio::test]
async fn a_reply_refused_for_one_in_progress_follows_that_reply() {
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, _) = accept(&listener).await;
        handshake(&mut socket).await;
        assert_eq!(
            receive(&mut socket).await["item"]["type"],
            "function_call_output"
        );
        assert_eq!(receive(&mut socket).await["type"], "response.create");
        send(
            &mut socket,
            json!({"type": "error", "error": {"code": "conversation_already_has_active_response", "message": "busy"}}),
        )
        .await;
        send(&mut socket, json!({"type": "response.done"})).await;
        // The reply waited for the one in progress, and goes at its end.
        assert_eq!(receive(&mut socket).await["type"], "response.create");
        closed(&mut socket).await;
    });
    let mut session = voice(&url).open(config()).await.unwrap();
    session.input.tool_result("c1", "done", true).await.unwrap();
    assert_eq!(
        next(&mut session).await,
        RealtimeEvent::Refused {
            code: Some("conversation_already_has_active_response".into())
        }
    );
    assert_eq!(next(&mut session).await, RealtimeEvent::ResponseDone);
    session.close().await;
    finish(server).await;
}

#[tokio::test]
async fn a_tap_cancels_generation_before_truncating_playback() {
    let (listener, url) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, _) = accept(&listener).await;
        handshake(&mut socket).await;
        send(&mut socket, json!({"type":"response.created"})).await;
        send(
            &mut socket,
            json!({"type":"response.output_audio.delta", "item_id":"cut", "delta":pcm(&[1, 0])}),
        )
        .await;
        assert_eq!(receive(&mut socket).await["type"], "response.cancel");
        let truncated = receive(&mut socket).await;
        assert_eq!(truncated["type"], "conversation.item.truncate");
        assert_eq!(truncated["audio_end_ms"], 80);
        send(&mut socket, json!({"type":"response.done"})).await;
        closed(&mut socket).await;
    });
    let mut session = voice(&url).open(config()).await.unwrap();
    assert!(matches!(next(&mut session).await, RealtimeEvent::Audio(_)));
    session.input.truncate(80).await.unwrap();
    assert_eq!(next(&mut session).await, RealtimeEvent::ResponseDone);
    drop(session);
    server.await.unwrap();
}

#[tokio::test]
async fn instructions_refresh_without_starting_speech_and_survive_reconnect() {
    for openai in [false, true] {
        let (listener, url) = listen().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = accept(&listener).await;
            handshake(&mut socket).await;
            let update = receive(&mut socket).await;
            assert_eq!(update["type"], "session.update");
            assert_eq!(
                update["session"]["instructions"],
                "Work running. Already said: inspecting tests."
            );
            socket.close(None).await.unwrap();
            let (mut socket, _) = accept(&listener).await;
            let fresh = handshake(&mut socket).await;
            assert_eq!(
                fresh["instructions"],
                "Work running. Already said: inspecting tests."
            );
            closed(&mut socket).await;
        });
        let endpoint = Endpoint {
            provider_id: if openai { "openai" } else { "xai" }.into(),
            base_url: url,
            key: Some("fixture".into()),
        };
        let provider = if openai {
            RealtimeSocket::openai(endpoint)
        } else {
            RealtimeSocket::grok(endpoint)
        };
        let mut configuration = config();
        configuration.input_rate = 24_000;
        let mut session = provider.open(configuration).await.unwrap();
        session
            .input
            .instructions("Work running. Already said: inspecting tests.")
            .await
            .unwrap();
        assert_eq!(next(&mut session).await, RealtimeEvent::Reconnecting);
        assert_eq!(next(&mut session).await, RealtimeEvent::Renewed);
        session.close().await;
        finish(server).await;
    }
}

mod barge;
