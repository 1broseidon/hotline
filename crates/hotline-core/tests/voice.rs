//! A scripted voice client uses the same door as the desktop.
mod common;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use uuid::Uuid;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn send(socket: &mut Socket, frame: Value) {
    socket.send(Message::text(frame.to_string())).await.unwrap();
}

async fn until(socket: &mut Socket, matches: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = socket.next().await.unwrap().unwrap();
            let frame: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            if matches(&frame) {
                return frame;
            }
        }
    })
    .await
    .expect("the voice wire did not answer")
}

#[tokio::test]
async fn a_voice_call_carries_heard_said_and_a_playable_clip() {
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        hotline_core::desk::Desk::open_with_voice_services(
            root.path(),
            common::store(),
            Some(services()),
        )
        .unwrap(),
    );
    let door = Door::bind(desk.log.clone(), "voice-test".into(), desk).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run());
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=voice-test"))
        .await
        .unwrap();
    // A real desk command creates the teammate; the dispatcher routes through
    // the same session.start/session.prompt commands the window uses.
    let provider = provider().await;
    send(&mut socket, json!({"id":90,"cmd":"credential.custom_save","params":{"draft":{"name":"Voice fixture","baseUrl":provider,"api":"chat_completions","models":["test"]}}})).await;
    assert_eq!(until(&mut socket, |f| f["id"] == 90).await["ok"], true);
    send(&mut socket, json!({"id":91,"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.path().to_str().unwrap()}}})).await;
    let mack = until(&mut socket, |f| f["id"] == 91).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let call = Uuid::new_v4().to_string();
    send(
        &mut socket,
        json!({"id":1,"cmd":"voice.call_start","params":{"callId":call}}),
    )
    .await;
    assert_eq!(
        until(&mut socket, |f| f["id"] == 1).await["result"]["callId"],
        call
    );
    send(&mut socket, json!({"id":2,"sub":{"call":call}})).await;
    assert_eq!(
        until(&mut socket, |f| f["sub"] == 2 && f["snapshot"].is_array()).await["snapshot"][0]["state"],
        "listening"
    );
    send(&mut socket, json!({"id":3,"cmd":"voice.utterance","params":{"callId":call,"seq":0,"mimeType":"audio/wav","data":STANDARD.encode(wav()),"durationMs":2390}})).await;
    let started = std::time::Instant::now();
    let heard = until(&mut socket, |f| f["event"]["type"] == "heard").await;
    assert_eq!(heard["event"]["seq"], 0);
    let said = until(&mut socket, |f| f["event"]["type"] == "said").await;
    let clip = until(&mut socket, |f| f["event"]["type"] == "clip").await;
    assert_eq!(
        said["event"]["text"],
        "I've asked Mack to check the failing PR."
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "fake speech acknowledgement took {:?}",
        started.elapsed()
    );
    assert_eq!(clip["event"]["id"], said["event"]["id"]);
    let tape =
        hotline_core::log::Log::open(root.path()).load(&hotline_core::log::StreamId::Tape(mack));
    assert!(
        tape.iter()
            .any(|e| e["kind"] == "user" && e["text"] == "Check the failing PR."),
        "{tape:?}"
    );
    assert_eq!(clip["event"]["final"], true);
    let wav = STANDARD
        .decode(clip["event"]["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(
        u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize + 8,
        wav.len()
    );
    send(
        &mut socket,
        json!({"id":4,"cmd":"voice.call_start","params":{"callId":call}}),
    )
    .await;
    assert_eq!(until(&mut socket, |f| f["id"] == 4).await["ok"], true);
    send(
        &mut socket,
        json!({"id":5,"cmd":"voice.call_start","params":{"callId":Uuid::new_v4().to_string()}}),
    )
    .await;
    assert_eq!(
        until(&mut socket, |f| f["event"]["state"] == "ended").await["event"]["reason"],
        "replaced"
    );
    socket.close(None).await.unwrap();
    server.abort();
}

use hotline_core::contract::{Command, VoiceModel};
use hotline_core::voice::{
    Services,
    dispatcher::{Context, Dispatcher},
    metering::Budget,
    speech::{Clip, Speech, SpeechError, SpeechId, SpeechSet},
};

struct FakeSpeech;
#[async_trait::async_trait]
impl Speech for FakeSpeech {
    fn id(&self) -> SpeechId {
        SpeechId {
            provider_id: "test".into(),
            model_id: "fake".into(),
            voice: None,
        }
    }
    fn accepts(&self) -> &[&str] {
        &["audio/wav", "audio/mp4"]
    }
    async fn transcribe(&self, clip: Clip) -> Result<String, SpeechError> {
        assert_eq!(clip.bytes, wav());
        Ok("ask Mack to check the failing PR".into())
    }
    async fn speak(&self, _: &str) -> Result<Clip, SpeechError> {
        Ok(Clip {
            mime: "audio/wav".into(),
            bytes: include_bytes!("fixtures/voice/acknowledgement.wav").to_vec(),
        })
    }
}
struct ScriptDispatcher;
#[async_trait::async_trait]
impl Dispatcher for ScriptDispatcher {
    fn id(&self) -> VoiceModel {
        VoiceModel {
            provider_id: "test".into(),
            model_id: "fake".into(),
            voice: None,
        }
    }
    async fn answer(&self, context: Context, text: &str, _: Arc<Budget>) -> Result<String, String> {
        assert_eq!(text, "ask Mack to check the failing PR");
        let mack = hotline_core::room::roster(&context.log)
            .into_iter()
            .find(|p| p.name == "Mack")
            .unwrap();
        context
            .execute(Command::SessionPrompt {
                persona_id: mack.id,
                text: "Check the failing PR.".into(),
                reply_to: None,
                attachments: None,
            })
            .await?;
        Ok("I've asked Mack to check the failing PR.".into())
    }
    async fn narrate(&self, name: &str, text: &str, _: Arc<Budget>) -> Result<String, String> {
        Ok(format!("{name} says: {text}"))
    }
}
fn services() -> Services {
    let speech = Arc::new(FakeSpeech);
    Services {
        speech: SpeechSet {
            stt: speech.clone(),
            tts: speech,
            fallback_tts: None,
        },
        dispatcher: Arc::new(ScriptDispatcher),
    }
}
fn wav() -> Vec<u8> {
    include_bytes!("fixtures/voice/ask-mack.wav").to_vec()
}
async fn provider() -> String {
    use axum::{Router, body::Bytes, routing::post};
    let app = Router::new().route("/v1/chat/completions", post(|_: Bytes| async {
        let delta = json!({"id":"voice_fixture","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{"role":"assistant","content":"The failing PR needs a test fix."},"finish_reason":null}]});
        let done = json!({"id":"voice_fixture","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}});
        ([("Content-Type", "text/event-stream")], format!("data: {delta}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    url
}
