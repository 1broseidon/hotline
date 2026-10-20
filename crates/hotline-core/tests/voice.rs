//! A scripted voice client uses the same door as the desktop.
mod common;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use uuid::Uuid;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn send(socket: &mut Socket, frame: Value) {
    socket.send(Message::text(frame.to_string())).await.unwrap();
}

/// The next frame that `matches`, within `common::ANSWER`; the frames before
/// it are dropped.
async fn frame_where(socket: &mut Socket, matches: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(common::ANSWER, async {
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

use hotline_core::contract::{Command, VoiceModel};
use hotline_core::voice::{
    Services,
    dispatcher::{Context, Dispatcher},
    metering::Budget,
    realtime::{Command as Asked, Realtime, RealtimeConfig, RealtimeEvent, RealtimeSession},
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
        Ok("I am passing that to Mack.".into())
    }
    async fn narrate(&self, name: &str, text: &str, _: Arc<Budget>) -> Result<String, String> {
        Ok(format!("{name} says: {text}"))
    }
    /// Answers a delegated "what did we fix?" from the teammate's tools it
    /// is offered, and nothing else.
    async fn delegated(
        &self,
        preamble: &str,
        request: &str,
        tools: Vec<rig::tool::DynamicTool>,
        _: Arc<Budget>,
    ) -> Result<String, String> {
        let names: Vec<&str> = tools.iter().map(|tool| tool.name().as_str()).collect();
        assert!(names.contains(&"search_thread"), "{names:?}");
        assert!(names.contains(&"send_to_teammate"), "{names:?}");
        assert!(preamble.contains("cannot run your tools"), "{preamble}");
        if request == "what did we fix?" {
            Ok("We fixed the zebracrossing bug.".into())
        } else {
            Err("Not a lookup.".into())
        }
    }
    /// Says the untagged reply below again, given what the person said.
    async fn rewrite(&self, words: &str, written: &str, _: Arc<Budget>) -> Result<String, String> {
        assert_eq!(words, "ask Mack to check the failing PR");
        assert_eq!(written, UNTAGGED_REPLY);
        Ok(REWRITTEN.into())
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
        dispatcher: Some(Arc::new(ScriptDispatcher)),
        realtime: None,
    }
}
fn wav() -> Vec<u8> {
    include_bytes!("fixtures/voice/ask-mack.wav").to_vec()
}
async fn provider(reply: &'static str) -> String {
    provider_seeing(reply).await.0
}

/// A chat-completions fixture that answers every request with `reply`, and
/// keeps every request it was sent.
async fn provider_seeing(reply: &'static str) -> (String, Arc<std::sync::Mutex<Vec<Value>>>) {
    use axum::{Router, body::Bytes, routing::post};
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let requests = seen.clone();
    let app = Router::new().route("/v1/chat/completions", post(move |body: Bytes| {
        let requests = requests.clone();
        async move {
        if let Ok(request) = serde_json::from_slice::<Value>(&body) {
            requests.lock().unwrap().push(request);
        }
        let delta = json!({"id":"voice_fixture","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{"role":"assistant","content":reply},"finish_reason":null}]});
        let done = json!({"id":"voice_fixture","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}});
        ([("Content-Type", "text/event-stream")], format!("data: {delta}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
    }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, seen)
}

/// The teammate's reply on a call to it, as its agent wrote it: the version
/// to say, then the version to show, a table.
const TAGGED_REPLY: &str = "<spoken>The checks passed.</spoken>\n<written>All checks passed:\n\n| check | result |\n| --- | --- |\n| unit | ok |</written>";
const TAGGED_WRITTEN: &str =
    "All checks passed:\n\n| check | result |\n| --- | --- |\n| unit | ok |";

/// A reply its agent wrote without the tags, to be read.
const UNTAGGED_REPLY: &str =
    "Here is the fix:\n```rust\nlet ready = true;\n```\nThe checks pass now.";
/// That reply as the call assistant says it again to be heard.
const REWRITTEN: &str = "The fix is in and the checks pass now. Details are in the chat.";

/// A message the teammate posted to its DM from a call: stamped as from it.
fn is_call_post(event: &Value) -> bool {
    event["kind"] == "agent" && event["from"]["kind"] == "call"
}

/// A realtime provider played by the test: it hands out one scripted session.
struct ScriptedFront {
    session: std::sync::Mutex<Option<RealtimeSession>>,
    config: std::sync::Mutex<Option<RealtimeConfig>>,
    delegates: bool,
}
#[async_trait::async_trait]
impl Realtime for ScriptedFront {
    fn id(&self) -> SpeechId {
        SpeechId {
            provider_id: "test".into(),
            model_id: "front".into(),
            voice: None,
        }
    }
    fn is_subscription(&self) -> bool {
        true
    }
    fn delegates(&self) -> bool {
        self.delegates
    }
    async fn open(&self, config: RealtimeConfig) -> Result<RealtimeSession, SpeechError> {
        *self.config.lock().unwrap() = Some(config);
        self.session
            .lock()
            .unwrap()
            .take()
            .ok_or(SpeechError::Cancelled)
    }
}

struct FrontDesk {
    socket: Socket,
    asked: tokio::sync::mpsc::Receiver<Asked>,
    events: tokio::sync::mpsc::Sender<Result<RealtimeEvent, SpeechError>>,
    front: Arc<ScriptedFront>,
    log: hotline_core::log::Log,
    root: tempfile::TempDir,
}

/// A desk whose front is scripted, with a socket on it. With `front: false`
/// the desk has no realtime provider at all.
async fn front_desk(front: bool, queue: usize) -> FrontDesk {
    front_desk_with(front, queue, false).await
}

/// The same, with a front that runs no tools itself when `delegates`.
async fn front_desk_with(front: bool, queue: usize, delegates: bool) -> FrontDesk {
    let root = tempfile::tempdir().unwrap();
    let (session, asked, events) = RealtimeSession::scripted(queue);
    let scripted = Arc::new(ScriptedFront {
        session: std::sync::Mutex::new(Some(session)),
        config: std::sync::Mutex::new(None),
        delegates,
    });
    let mut services = services();
    if front {
        services.realtime = Some(scripted.clone());
    }
    let desk = Arc::new(
        hotline_core::desk::Desk::open_with_voice_services(
            root.path(),
            common::store(),
            Some(services),
        )
        .unwrap(),
    );
    let log = desk.log.clone();
    let parts = desk.parts();
    let door = Door::bind(log.clone(), "front-test".into(), desk, parts).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    let (socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=front-test"))
        .await
        .unwrap();
    FrontDesk {
        socket,
        asked,
        events,
        front: scripted,
        log,
        root,
    }
}

fn pcm(samples: &[i16]) -> String {
    STANDARD.encode(
        samples
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect::<Vec<u8>>(),
    )
}

async fn answer(socket: &mut Socket, id: u32, command: Value) -> Value {
    let mut frame = command;
    frame["id"] = json!(id);
    send(socket, frame).await;
    frame_where(socket, |f| f["id"] == id).await
}

async fn next_asked(asked: &mut tokio::sync::mpsc::Receiver<Asked>) -> Asked {
    tokio::time::timeout(Duration::from_secs(15), asked.recv())
        .await
        .expect("the front was asked nothing")
        .expect("the front closed")
}

// The tests, one file per topic in `tests/voice/`. The inline module is what
// puts them there: a crate root's own `mod` would look beside it in `tests/`,
// where Cargo would take each file for a binary of its own.
mod voice {
    use super::*;

    mod after_call;
    mod direct;
    mod front;
    mod front_tools;
    mod pipeline;
    #[cfg(unix)]
    mod processes;
}
