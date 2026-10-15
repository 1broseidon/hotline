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

async fn until(socket: &mut Socket, matches: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(Duration::from_secs(15), async {
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
    voice_reply_is_delivered("The failing PR needs a test fix.", 1).await;
}

#[tokio::test]
async fn a_reply_with_trailing_whitespace_is_delivered_in_the_call() {
    voice_reply_is_delivered("The failing PR needs a test fix. \n\t", 1).await;
}

#[tokio::test]
async fn a_reply_paced_into_several_bubbles_is_delivered_whole_in_the_call() {
    voice_reply_is_delivered(
        "The failing PR needs a regression test for the reconnect path before it can merge.\n\nThe existing checks pass locally, but I still need to verify the result behind the tunnel.",
        2,
    )
    .await;
}

#[tokio::test]
async fn a_goodbye_needs_enough_audio_before_the_call_can_end() {
    for duration in [300u32, 400] {
        let mut bytes = wav()[..36].to_vec();
        bytes.extend(b"data");
        bytes.extend(0u32.to_le_bytes());
        let samples = duration * 32;
        bytes[4..8].copy_from_slice(&(samples + 36).to_le_bytes());
        bytes[40..44].copy_from_slice(&samples.to_le_bytes());
        bytes.resize(44 + samples as usize, 0);
        check_clip(
            bytes,
            "audio/wav",
            duration,
            duration >= 400,
            duration,
            false,
        )
        .await;
    }
    // A valid container with a plausible duration but too few bytes for a farewell.
    check_clip(mp4(1000, 100), "audio/mp4", 1000, false, 1000, false).await;
}

#[tokio::test]
async fn an_mp4_header_cannot_reduce_its_stt_reservation() {
    // The mdhd and wire both claim one second; the byte bound reserves fifteen.
    check_clip(mp4(1000, 60_000), "audio/mp4", 1000, true, 15_000, false).await;
    // A two-second budget must refuse this before the STT provider sees the clip.
    check_clip(mp4(1000, 60_000), "audio/mp4", 1000, true, 15_000, true).await;
}

fn mp4(duration: u32, size: usize) -> Vec<u8> {
    fn atom(kind: &[u8; 4], body: Vec<u8>) -> Vec<u8> {
        let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend(kind);
        bytes.extend(body);
        bytes
    }
    let mut mdhd = vec![0; 12];
    mdhd.extend(1000u32.to_be_bytes());
    mdhd.extend(duration.to_be_bytes());
    let mut bytes = atom(b"ftyp", b"M4A ".to_vec());
    bytes.extend(atom(
        b"moov",
        atom(b"trak", atom(b"mdia", atom(b"mdhd", mdhd))),
    ));
    bytes.extend(atom(b"mdat", vec![0; size - bytes.len() - 8]));
    bytes
}

struct ClipCheck {
    expected: Clip,
    transcribed: AtomicUsize,
    routed: AtomicUsize,
}

#[async_trait::async_trait]
impl Speech for ClipCheck {
    fn id(&self) -> SpeechId {
        FakeSpeech.id()
    }
    async fn transcribe(&self, clip: Clip) -> Result<String, SpeechError> {
        assert_eq!(clip, self.expected);
        self.transcribed.fetch_add(1, Ordering::SeqCst);
        Ok("Bye.".into())
    }
    async fn speak(&self, text: &str) -> Result<Clip, SpeechError> {
        FakeSpeech.speak(text).await
    }
}

#[async_trait::async_trait]
impl Dispatcher for ClipCheck {
    fn id(&self) -> VoiceModel {
        ScriptDispatcher.id()
    }
    async fn answer(&self, _: Context, text: &str, _: Arc<Budget>) -> Result<String, String> {
        assert_eq!(text, "Bye.");
        self.routed.fetch_add(1, Ordering::SeqCst);
        Ok("Please continue.".into())
    }
    async fn narrate(&self, _: &str, text: &str, _: Arc<Budget>) -> Result<String, String> {
        Ok(text.into())
    }
    async fn rewrite(&self, _: &str, _: &str, _: Arc<Budget>) -> Result<String, String> {
        unreachable!("a desk call has no teammate's reply to rewrite")
    }
}

async fn check_clip(
    bytes: Vec<u8>,
    mime: &str,
    duration: u32,
    goodbye: bool,
    billed: u32,
    denied: bool,
) {
    let root = tempfile::tempdir().unwrap();
    let fake = Arc::new(ClipCheck {
        expected: Clip {
            mime: mime.into(),
            bytes: bytes.clone(),
        },
        transcribed: AtomicUsize::new(0),
        routed: AtomicUsize::new(0),
    });
    let desk = Arc::new(
        hotline_core::desk::Desk::open_with_voice_services(
            root.path(),
            common::store(),
            Some(Services {
                speech: SpeechSet {
                    stt: fake.clone(),
                    tts: fake.clone(),
                    fallback_tts: None,
                },
                dispatcher: Some(fake.clone()),
                realtime: None,
            }),
        )
        .unwrap(),
    );
    let door = Door::bind(desk.log.clone(), "voice-check".into(), desk).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run());
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=voice-check"))
        .await
        .unwrap();
    if denied {
        let cap = hotline_core::voice::ledger::stt_usd("test", 2.0);
        send(
            &mut socket,
            json!({"id":10,"cmd":"settings.update","params":{"patch":{"voice":{"dayUsd":cap}}}}),
        )
        .await;
        assert_eq!(until(&mut socket, |f| f["id"] == 10).await["ok"], true);
    }
    let call = Uuid::new_v4().to_string();
    send(
        &mut socket,
        json!({"id":1,"cmd":"voice.call_start","params":{"callId":call}}),
    )
    .await;
    assert_eq!(until(&mut socket, |f| f["id"] == 1).await["ok"], true);
    send(&mut socket, json!({"id":2,"sub":{"call":call}})).await;
    until(&mut socket, |f| f["snapshot"].is_array()).await;
    send(&mut socket, json!({"id":3,"cmd":"voice.utterance","params":{"callId":call,"seq":0,"mimeType":mime,"data":STANDARD.encode(bytes),"durationMs":duration}})).await;
    assert_eq!(until(&mut socket, |f| f["id"] == 3).await["ok"], true);
    let state = until(&mut socket, |f| {
        matches!(f["event"]["state"].as_str(), Some("listening" | "ended"))
    })
    .await;
    assert_eq!(
        state["event"]["state"],
        if goodbye || denied {
            "ended"
        } else {
            "listening"
        }
    );
    assert_eq!(
        state["event"]["reason"],
        if denied {
            json!("budget")
        } else if goodbye {
            json!("goodbye")
        } else {
            Value::Null
        }
    );
    assert_eq!(
        fake.transcribed.load(Ordering::SeqCst),
        usize::from(!denied)
    );
    assert_eq!(
        fake.routed.load(Ordering::SeqCst),
        usize::from(!denied && !goodbye)
    );
    if !denied {
        let ledger: Value =
            serde_json::from_slice(&std::fs::read(root.path().join("voice-ledger.json")).unwrap())
                .unwrap();
        let expected = hotline_core::voice::ledger::stt_usd("test", billed as f64 / 1000.0);
        assert!((ledger["daySpend"]["stt"].as_f64().unwrap() - expected).abs() < 1e-9);
    }
    socket.close(None).await.unwrap();
    server.abort();
}

async fn voice_reply_is_delivered(reply: &'static str, expected_bubbles: usize) {
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
    let provider = provider(reply).await;
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
    let heard = until(&mut socket, |f| f["event"]["type"] == "heard").await;
    assert_eq!(heard["event"]["seq"], 0);
    let said = until(&mut socket, |f| f["event"]["type"] == "said").await;
    let clip = until(&mut socket, |f| f["event"]["type"] == "clip").await;
    // No bundled acknowledgement: the first words are the dispatcher's own.
    assert_ne!(said["event"]["text"], "");
    assert_eq!(clip["event"]["id"], said["event"]["id"]);
    // The answer is one line: an empty final clip closes it once it is over.
    let closing = until(&mut socket, |f| {
        f["event"]["type"] == "clip" && f["event"]["final"] == true
    })
    .await;
    assert_eq!(closing["event"]["id"], said["event"]["id"]);
    assert_eq!(closing["event"]["data"], "");
    let delivery = until(&mut socket, |f| f["event"]["type"] == "delivery").await;
    let narrated = until(&mut socket, |f| f["event"]["type"] == "said").await;
    assert_eq!(delivery["event"]["personaId"], mack);
    assert_eq!(
        delivery["event"]["text"],
        if expected_bubbles == 1 {
            reply.trim().to_string()
        } else {
            format!(
                "Mack says: {}",
                reply.split_whitespace().collect::<Vec<_>>().join(" ")
            )
        }
    );
    assert_eq!(narrated["event"]["text"], delivery["event"]["text"]);
    loop {
        let frame = until(&mut socket, |f| f["event"].is_object()).await;
        assert_ne!(frame["event"]["type"], "delivery", "one delivery per reply");
        if frame["event"]["type"] == "clip" {
            assert_eq!(frame["event"]["id"], narrated["event"]["id"]);
            if frame["event"]["final"] == true {
                break;
            }
        }
    }
    let tape =
        hotline_core::log::Log::open(root.path()).load(&hotline_core::log::StreamId::Tape(mack));
    let bubbles: Vec<_> = tape.iter().filter(|e| e["kind"] == "agent").collect();
    assert_eq!(bubbles.len(), expected_bubbles, "{bubbles:?}");
    assert_eq!(delivery["event"]["eventId"], bubbles[0]["id"]);
    assert_eq!(
        bubbles
            .iter()
            .map(|e| e["text"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n\n"),
        reply.trim(),
    );
    assert!(
        tape.iter()
            .any(|e| e["kind"] == "user" && e["text"] == "Check the failing PR."),
        "{tape:?}"
    );
    assert_eq!(clip["event"]["final"], false);
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

#[tokio::test]
async fn a_direct_call_uses_the_existing_agent_conversation_without_dispatching() {
    direct_call_uses_existing_conversation(
        false,
        TAGGED_REPLY,
        "The checks passed.",
        TAGGED_WRITTEN,
        Some("The checks passed."),
        "both",
    )
    .await;
}

#[tokio::test]
async fn device_text_reuses_the_direct_agent_session_and_call_origin() {
    direct_call_uses_existing_conversation(
        true,
        TAGGED_REPLY,
        "The checks passed.",
        TAGGED_WRITTEN,
        Some("The checks passed."),
        "both",
    )
    .await;
}

/// A reply its agent wrote without the tags, to be read.
const UNTAGGED_REPLY: &str =
    "Here is the fix:\n```rust\nlet ready = true;\n```\nThe checks pass now.";
/// That reply as the call assistant says it again to be heard.
const REWRITTEN: &str = "The fix is in and the checks pass now. Details are in the chat.";

/// An agent that ignores the tags wrote to be read: the call assistant says
/// it again to be heard, that is said and kept as what was said for it, and
/// the chat shows all of the reply.
#[tokio::test]
async fn a_reply_without_tags_is_rewritten_to_be_heard() {
    direct_call_uses_existing_conversation(
        true,
        UNTAGGED_REPLY,
        REWRITTEN,
        UNTAGGED_REPLY,
        Some(REWRITTEN),
        "rewritten",
    )
    .await;
}

/// One brain, two outputs: what is said on a call to a teammate is a turn of
/// the teammate's own conversation (the person's exact words on its tape,
/// marked as said on the call, in its open chapter), the agent is handed the
/// contract after the words, the call says the reply's spoken version, and
/// the chat shows its written version, with what was said kept beside it.
/// No call assistant is asked anything, and the teammate's turn is metered
/// as Chat.
async fn direct_call_uses_existing_conversation(
    device_text: bool,
    reply: &'static str,
    spoken: &str,
    written: &str,
    kept: Option<&str>,
    counted: &str,
) {
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        hotline_core::desk::Desk::open_with_voice_services(
            root.path(),
            common::store(),
            Some(services()),
        )
        .unwrap(),
    );
    let log = desk.log.clone();
    let door = Door::bind(log.clone(), "direct-test".into(), desk).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run());
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=direct-test"))
        .await
        .unwrap();
    let (provider, requests) = provider_seeing(reply).await;
    // A key, so the teammate's turns are billed per token.
    send(&mut socket, json!({"id":90,"cmd":"credential.custom_save","params":{"draft":{"name":"Direct fixture","baseUrl":provider,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    assert_eq!(until(&mut socket, |f| f["id"] == 90).await["ok"], true);
    send(&mut socket, json!({"id":91,"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.path().to_str().unwrap()}}})).await;
    let persona = until(&mut socket, |f| f["id"] == 91).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    send(
        &mut socket,
        json!({"id":92,"cmd":"session.start","params":{"personaId":persona}}),
    )
    .await;
    assert_eq!(until(&mut socket, |f| f["id"] == 92).await["ok"], true);
    send(
        &mut socket,
        json!({"id":93,"cmd":"voice.status","params":{}}),
    )
    .await;
    assert!(
        until(&mut socket, |f| f["id"] == 93).await["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "voiceDirectCalls")
    );
    let call = Uuid::new_v4().to_string();
    let mut params = json!({"callId":call,"personaId":persona,"streamAudio":true});
    if device_text {
        params["inputMode"] = json!("text");
    }
    send(
        &mut socket,
        json!({"id":1,"cmd":"voice.call_start","params":params}),
    )
    .await;
    let started = until(&mut socket, |f| f["id"] == 1).await;
    assert_eq!(started["result"]["personaId"], persona);
    assert_eq!(
        started["result"]["inputMode"],
        if device_text { "text" } else { "audio" }
    );
    if device_text {
        assert_eq!(started["result"]["input"], json!(["text/plain"]));
    }
    send(&mut socket, json!({"id":2,"sub":{"call":call}})).await;
    until(&mut socket, |f| f["snapshot"].is_array()).await;
    let input = if device_text {
        json!({"id":3,"cmd":"voice.text","params":{"callId":call,"seq":1,"text":"ask Mack to check the failing PR"}})
    } else {
        json!({"id":3,"cmd":"voice.utterance","params":{"callId":call,"seq":1,"mimeType":"audio/wav","data":STANDARD.encode(wav()),"durationMs":2390}})
    };
    send(&mut socket, input.clone()).await;
    let heard = until(&mut socket, |f| f["event"]["type"] == "heard").await;
    // The reply is one line: its spoken part, said in clips under its id and
    // closed by an empty final clip. The teammate is the voice, so there is no
    // `delivery` naming it.
    let mut said = Vec::new();
    let mut clips = Vec::new();
    loop {
        let frame = until(&mut socket, |f| f["event"].is_object()).await;
        let event = &frame["event"];
        assert_ne!(event["type"], "delivery", "the teammate speaks for itself");
        match event["type"].as_str() {
            Some("said") => said.push(event.clone()),
            Some("clip") => {
                clips.push(event.clone());
                if event["final"] == true {
                    break;
                }
            }
            _ => {}
        }
    }
    let line = said[0]["id"].clone();
    assert!(said.iter().all(|said| said["id"] == line), "{said:?}");
    assert_eq!(said.last().unwrap()["text"], spoken);
    assert!(
        said.iter().all(|said| {
            let text = said["text"].as_str().unwrap();
            !text.contains("spoken>") && !text.contains("written>")
        }),
        "{said:?}"
    );
    assert!(clips.iter().all(|clip| clip["id"] == line));
    let last = clips.last().unwrap();
    assert_eq!(last["data"], "");
    assert_eq!(last["index"], clips.len() - 1);
    // Its turn over, the call listens.
    until(&mut socket, |f| f["event"]["state"] == "listening").await;
    let mut replay = input;
    replay["id"] = json!(7);
    send(&mut socket, replay).await;
    assert_eq!(until(&mut socket, |f| f["id"] == 7).await["ok"], false);

    // The agent heard the words, then the contract.
    let words = heard["event"]["text"].as_str().unwrap().to_string();
    let asked = requests
        .lock()
        .unwrap()
        .iter()
        .flat_map(|request| request["messages"].as_array().cloned().unwrap_or_default())
        .filter(|message| message["role"] == "user")
        .map(|message| message["content"].to_string())
        .find(|content| content.contains("[Voice call: answer twice"))
        .expect("the turn's input carries the contract");
    assert!(asked.contains(&words), "{asked}");
    assert!(
        asked.contains("Do not assume the reader heard the spoken version."),
        "{asked}"
    );

    // What was said is kept beside the reply as soon as the reply is
    // written, which for a rewrite may be just after it is said.
    let mut tape = Vec::new();
    for _ in 0..200 {
        tape = log.load(&hotline_core::log::StreamId::Tape(persona.clone()));
        let first = tape.iter().find(|v| v["kind"] == "agent");
        if first.is_some_and(|reply| reply["spoken"].as_str() == kept) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        tape.iter().filter(|v| v["kind"] == "chapter").count(),
        1,
        "{tape:?}"
    );
    let user = tape.iter().find(|v| v["kind"] == "user").unwrap();
    assert_eq!(tape.iter().filter(|v| v["kind"] == "user").count(), 1);
    assert_eq!(user["text"], words, "the person's words, as they said them");
    assert!(
        user["id"]
            .as_str()
            .unwrap()
            .starts_with(&format!("voice:{call}:1:agent:"))
    );
    let agents: Vec<&Value> = tape.iter().filter(|v| v["kind"] == "agent").collect();
    let shown: Vec<&str> = agents.iter().map(|v| v["text"].as_str().unwrap()).collect();
    // Paced into bubbles, so compared word for word.
    let words_of = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(
        words_of(&shown.join("\n")),
        words_of(written),
        "the chat shows the written version"
    );
    // What was said is kept on the reply's first bubble, for the transcript
    // line and the model's history.
    assert_eq!(agents[0]["spoken"].as_str(), kept, "{tape:?}");
    assert!(agents[1..].iter().all(|v| v.get("spoken").is_none()));
    assert!(
        !tape.iter().any(|v| {
            let event = v.to_string();
            event.contains("spoken>") || event.contains("written>")
        }),
        "{tape:?}"
    );
    assert!(
        !log.load(&hotline_core::log::StreamId::Tape(
            hotline_core::voice::TAPE_ID.into()
        ))
        .iter()
        .any(|v| matches!(v["kind"].as_str(), Some("user" | "agent")))
    );
    // The turn spent the Chat budget as the teammate's; nothing went on a
    // call assistant.
    let ledger = |file: &str| -> Value {
        std::fs::read(root.path().join(file))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null)
    };
    assert!(
        ledger("chat-ledger.json")["daySpend"]["teammates"]
            .as_f64()
            .is_some_and(|spent| spent > 0.0),
        "{}",
        ledger("chat-ledger.json")
    );
    assert_eq!(
        ledger("voice-ledger.json")["daySpend"]["dispatcher"]
            .as_f64()
            .unwrap_or(0.0),
        0.0
    );
    // The reply is counted by how it was said, under the teammate's agent
    // and model, and the count is on the wire.
    send(
        &mut socket,
        json!({"id":8,"cmd":"voice.status","params":{}}),
    )
    .await;
    let replies = until(&mut socket, |f| f["id"] == 8).await["result"]["replies"].clone();
    assert_eq!(replies.as_array().map(Vec::len), Some(1), "{replies}");
    let model = replies[0]["model"].as_str().unwrap();
    assert!(
        model.starts_with("hotline/") && model.ends_with("/test"),
        "{model}"
    );
    for path in ["both", "spokenOnly", "unclosed", "untagged", "rewritten"] {
        let expected = u64::from(path == counted);
        assert_eq!(replies[0][path], expected, "{path}: {replies}");
    }
    send(
        &mut socket,
        json!({"id":4,"cmd":"voice.call_start","params":{"callId":call}}),
    )
    .await;
    assert_eq!(
        until(&mut socket, |f| f["id"] == 4).await["ok"],
        false,
        "a live target cannot silently change to desk"
    );
    send(&mut socket, json!({"id":5,"cmd":"voice.call_start","params":{"callId":Uuid::new_v4().to_string(),"personaId":"missing"}})).await;
    assert_eq!(until(&mut socket, |f| f["id"] == 5).await["ok"], false);
    send(
        &mut socket,
        json!({"id":6,"cmd":"voice.hold","params":{"callId":call,"hold":true}}),
    )
    .await;
    assert_eq!(
        until(&mut socket, |f| f["id"] == 6).await["ok"],
        true,
        "invalid target did not replace the existing call"
    );
    socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn direct_call_readiness_and_start_survive_an_unavailable_desk_dispatcher() {
    let root = tempfile::tempdir().unwrap();
    let desk =
        Arc::new(hotline_core::desk::Desk::open_with_store(root.path(), common::store()).unwrap());
    let door = Door::bind(desk.log.clone(), "readiness-test".into(), desk).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run());
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=readiness-test"))
        .await
        .unwrap();
    send(&mut socket, json!({"id":1,"cmd":"credential.create","params":{"providerId":"openai","label":"Speech fixture","secret":"unused-test-key"}})).await;
    assert_eq!(until(&mut socket, |f| f["id"] == 1).await["ok"], true);
    send(&mut socket, json!({"id":2,"cmd":"settings.update","params":{"patch":{"voice":{"dispatcher":{"provider":"not-connected"}}}}})).await;
    assert_eq!(until(&mut socket, |f| f["id"] == 2).await["ok"], true);
    send(&mut socket, json!({"id":3,"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.path().to_str().unwrap()}}})).await;
    let persona = until(&mut socket, |f| f["id"] == 3).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    send(
        &mut socket,
        json!({"id":4,"cmd":"voice.status","params":{}}),
    )
    .await;
    let status = until(&mut socket, |f| f["id"] == 4).await;
    assert_eq!(status["result"]["available"], false);
    assert_eq!(status["result"]["directAvailable"], true);
    send(
        &mut socket,
        json!({"id":5,"cmd":"voice.call_start","params":{"callId":Uuid::new_v4().to_string()}}),
    )
    .await;
    assert_eq!(until(&mut socket, |f| f["id"] == 5).await["ok"], false);
    let call = Uuid::new_v4().to_string();
    send(&mut socket, json!({"id":6,"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"streamAudio":true}})).await;
    let started = until(&mut socket, |f| f["id"] == 6).await;
    assert_eq!(started["ok"], true);
    assert_eq!(started["result"]["personaId"], persona);
    send(
        &mut socket,
        json!({"id":7,"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
    assert_eq!(until(&mut socket, |f| f["id"] == 7).await["ok"], true);
    send(&mut socket, json!({"id":8,"cmd":"settings.update","params":{"patch":{"voice":{"dispatcher":{"provider":"not-connected"},"stt":{"provider":"not-connected"}}}}})).await;
    assert_eq!(until(&mut socket, |f| f["id"] == 8).await["ok"], true);
    send(
        &mut socket,
        json!({"id":9,"cmd":"voice.status","params":{"inputMode":"text"}}),
    )
    .await;
    let status = until(&mut socket, |f| f["id"] == 9).await;
    assert_eq!(status["result"]["available"], false);
    assert_eq!(status["result"]["directAvailable"], true);
    assert!(status["result"].get("stt").is_none());
    send(
        &mut socket,
        json!({"id":10,"cmd":"voice.call_start","params":{"callId":Uuid::new_v4().to_string(),"inputMode":"text"}}),
    )
    .await;
    assert_eq!(until(&mut socket, |f| f["id"] == 10).await["ok"], false);
    let text_call = Uuid::new_v4().to_string();
    send(&mut socket, json!({"id":11,"cmd":"voice.call_start","params":{"callId":text_call,"personaId":persona,"inputMode":"text","streamAudio":true}})).await;
    let started = until(&mut socket, |f| f["id"] == 11).await;
    assert_eq!(started["ok"], true);
    assert_eq!(started["result"]["personaId"], persona);
    assert_eq!(started["result"]["inputMode"], "text");
    assert_eq!(started["result"]["input"], json!(["text/plain"]));
    send(
        &mut socket,
        json!({"id":12,"cmd":"voice.call_end","params":{"callId":text_call}}),
    )
    .await;
    assert_eq!(until(&mut socket, |f| f["id"] == 12).await["ok"], true);
    socket.close(None).await.unwrap();
    server.abort();
}

/// A realtime provider played by the test: it hands out one scripted session.
struct ScriptedFront {
    session: std::sync::Mutex<Option<RealtimeSession>>,
    config: std::sync::Mutex<Option<RealtimeConfig>>,
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
    let root = tempfile::tempdir().unwrap();
    let (session, asked, events) = RealtimeSession::scripted(queue);
    let scripted = Arc::new(ScriptedFront {
        session: std::sync::Mutex::new(Some(session)),
        config: std::sync::Mutex::new(None),
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
    let door = Door::bind(log.clone(), "front-test".into(), desk).unwrap();
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
    until(socket, |f| f["id"] == id).await
}

async fn next_asked(asked: &mut tokio::sync::mpsc::Receiver<Asked>) -> Asked {
    tokio::time::timeout(Duration::from_secs(15), asked.recv())
        .await
        .expect("the front was asked nothing")
        .expect("the front closed")
}

#[tokio::test]
async fn a_front_call_carries_pcm_both_ways_and_truncates_on_barge_in() {
    let mut desk = front_desk(true, 64).await;
    let socket = &mut desk.socket;
    let status = answer(socket, 1, json!({"cmd":"voice.status","params":{}})).await;
    assert!(
        status["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "voiceFront")
    );

    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        2,
        json!({"cmd":"voice.call_start","params":{"callId":call,"mode":"front","inputRate":16000}}),
    )
    .await;
    assert_eq!(
        started["result"]["front"],
        json!({"inputRate":16000,"outputRate":24000})
    );
    // The front is told what the desk sends and takes: 24 kHz both ways.
    send(socket, json!({"id":3,"sub":{"call":call}})).await;
    until(socket, |f| f["sub"] == 3 && f["snapshot"].is_array()).await;

    // Microphone PCM at 16 kHz reaches the front at 24 kHz.
    for _ in 0..50 {
        let sent = answer(socket, 4, json!({"cmd":"voice.front_audio","params":{"callId":call,"seq":0,"data":pcm(&[0, 3000, 6000, 9000])}})).await;
        if sent["ok"] == true {
            break;
        }
        assert_eq!(sent["error"], "The front is still connecting.");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let Asked::Audio(heard) = next_asked(&mut desk.asked).await else {
        panic!("the front was not handed audio")
    };
    assert_eq!(heard.len(), 12, "four samples at 16 kHz are six at 24 kHz");
    let config = desk.front.config.lock().unwrap().clone().unwrap();
    assert_eq!((config.input_rate, config.output_rate), (24_000, 24_000));

    // Sequence numbers only rise.
    let repeated = answer(
        socket,
        5,
        json!({"cmd":"voice.front_audio","params":{"callId":call,"seq":0,"data":pcm(&[1,2])}}),
    )
    .await;
    assert_eq!(repeated["ok"], false);

    // The front's speech comes back in small numbered frames.
    desk.events
        .send(Ok(RealtimeEvent::Audio(vec![1; 2 * 4_800 + 100])))
        .await
        .unwrap();
    // The call is speaking before the first frame arrives.
    let speaking = until(socket, |f| f["event"]["type"] == "state").await;
    assert_eq!(speaking["event"]["state"], "speaking");
    let first = until(socket, |f| f["event"]["type"] == "frontAudio").await;
    assert_eq!(first["event"]["seq"], 0);
    assert_eq!(
        STANDARD
            .decode(first["event"]["data"].as_str().unwrap())
            .unwrap()
            .len(),
        4_800
    );
    let third = until(socket, |f| {
        f["event"]["type"] == "frontAudio" && f["event"]["seq"] == 2
    })
    .await;
    assert_eq!(
        STANDARD
            .decode(third["event"]["data"].as_str().unwrap())
            .unwrap()
            .len(),
        100
    );

    // The provider hears the person over the front: stop playing, then say
    // how much was heard, and the front keeps only that.
    desk.events
        .send(Ok(RealtimeEvent::SpeechStarted))
        .await
        .unwrap();
    until(socket, |f| f["event"]["type"] == "frontStop").await;
    let listening = until(socket, |f| f["event"]["type"] == "state").await;
    assert_eq!(listening["event"]["state"], "listening");
    let barged = answer(
        socket,
        6,
        json!({"cmd":"voice.front_barge","params":{"callId":call,"playedMs":250}}),
    )
    .await;
    assert_eq!(barged["ok"], true, "{barged}");
    assert_eq!(
        next_asked(&mut desk.asked).await,
        Asked::Truncate { played_ms: 250 }
    );

    // What each side said reaches the call card.
    desk.events
        .send(Ok(RealtimeEvent::Heard("check the PR".into())))
        .await
        .unwrap();
    assert_eq!(
        until(socket, |f| f["event"]["type"] == "heard").await["event"]["text"],
        "check the PR"
    );
    desk.events
        .send(Ok(RealtimeEvent::Saying("On it.".into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::ResponseDone))
        .await
        .unwrap();
    assert_eq!(
        until(socket, |f| f["event"]["type"] == "said").await["event"]["text"],
        "On it."
    );

    // A front call takes front audio only: the pipeline's inputs are shut.
    let utterance = answer(
        socket,
        7,
        json!({"cmd":"voice.text","params":{"callId":call,"seq":1,"text":"hi"}}),
    )
    .await;
    assert_eq!(utterance["ok"], false);
    let ended = answer(
        socket,
        8,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
    assert_eq!(ended["ok"], true);
    // Ending the call closes the front's session.
    tokio::time::timeout(Duration::from_secs(5), async {
        while desk.asked.recv().await.is_some() {}
    })
    .await
    .expect("the front outlived its call");
}

#[tokio::test]
async fn a_desk_without_a_front_answers_a_front_request_as_a_pipeline_call() {
    let mut desk = front_desk(false, 64).await;
    let socket = &mut desk.socket;
    let status = answer(socket, 1, json!({"cmd":"voice.status","params":{}})).await;
    assert!(
        !status["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "voiceFront")
    );
    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        2,
        json!({"cmd":"voice.call_start","params":{"callId":call,"mode":"front"}}),
    )
    .await;
    assert_eq!(started["ok"], true);
    assert!(started["result"].get("front").is_none(), "{started}");
    let sent = answer(
        socket,
        3,
        json!({"cmd":"voice.front_audio","params":{"callId":call,"seq":0,"data":pcm(&[1,2])}}),
    )
    .await;
    assert_eq!(sent["ok"], false);
    // A client that never heard of fronts starts the same call as before.
    let plain = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        4,
        json!({"cmd":"voice.call_start","params":{"callId":plain}}),
    )
    .await;
    assert!(started["result"].get("front").is_none());
}

#[tokio::test]
async fn a_front_that_cannot_open_hands_the_call_back_to_the_pipeline() {
    let mut desk = front_desk(true, 64).await;
    desk.front.session.lock().unwrap().take();
    let socket = &mut desk.socket;
    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call,"mode":"front"}}),
    )
    .await;
    assert_eq!(started["result"]["front"]["outputRate"], 24000);
    // Once the front is gone the call is a pipeline call, and no longer
    // refuses the pipeline's input.
    for id in 2..100 {
        let text = answer(
            socket,
            id,
            json!({"cmd":"voice.text","params":{"callId":call,"seq":1,"text":"hi"}}),
        )
        .await;
        if text["error"] != "This call has a voice front. Send front audio." {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the call kept a front that could not open");
}

#[tokio::test]
async fn a_front_that_is_behind_refuses_audio_instead_of_stalling_the_wire() {
    let mut desk = front_desk(true, 2).await;
    let socket = &mut desk.socket;
    let call = Uuid::new_v4().to_string();
    answer(
        socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call,"mode":"front"}}),
    )
    .await;
    let mut refused = None;
    for seq in 0..20u32 {
        let sent = answer(socket, 10 + seq, json!({"cmd":"voice.front_audio","params":{"callId":call,"seq":seq,"data":pcm(&[1,2])}})).await;
        if sent["ok"] == false && sent["error"] != "The front is still connecting." {
            refused = Some(sent);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let refused = refused.expect("a full front queue refuses audio");
    assert_eq!(
        refused["error"],
        "The front is behind. Drop this audio and send the next."
    );
    // The same socket still answers.
    let status = answer(socket, 99, json!({"cmd":"voice.status","params":{}})).await;
    assert_eq!(status["ok"], true);
}

/// The front wired end to end, against a scripted provider: it opens with
/// the teammate's prompt and the bridge's two tools; its `send_to_teammate`
/// hands the words to the teammate's own session unrephrased, as a typed
/// message would go (its open chapter, the call's origin on the line), told
/// they are a transcript, and is answered; the call
/// thinks with `listening` while the turn works; the teammate's spoken line
/// comes back to the front to say verbatim, and the desk synthesizes none of
/// it; `remain_silent` asks for no reply; and the call's thread keeps what
/// the front heard and said, each once.
#[tokio::test]
async fn a_front_call_relays_to_the_teammate_and_says_its_spoken_line() {
    let mut desk = front_desk(true, 64).await;
    let root = desk.root.path().to_path_buf();
    let socket = &mut desk.socket;
    let (provider, requests) = provider_seeing(TAGGED_REPLY).await;
    let saved = answer(socket, 90, json!({"cmd":"credential.custom_save","params":{"draft":{"name":"Front fixture","baseUrl":provider,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    assert_eq!(saved["ok"], true);
    let persona = answer(socket, 91, json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.to_str().unwrap()}}})).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let started = answer(
        socket,
        92,
        json!({"cmd":"session.start","params":{"personaId":persona}}),
    )
    .await;
    assert_eq!(started["ok"], true);

    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"mode":"front"}}),
    )
    .await;
    assert_eq!(started["result"]["front"]["outputRate"], 24000, "{started}");
    send(socket, json!({"id":2,"sub":{"call":call}})).await;
    until(socket, |f| f["sub"] == 2 && f["snapshot"].is_array()).await;

    // The front opens as the teammate's voice, with the bridge's tools.
    let config = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(config) = desk.front.config.lock().unwrap().clone() {
                return config;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the front was opened");
    assert!(config.instructions.contains("You are Mack."));
    assert!(config.instructions.contains("first person"));
    let tools: Vec<&str> = config.tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(tools, ["send_to_teammate", "remain_silent"]);
    assert_eq!(config.voice, "eve");

    // The person asks, and the front hands it over in their words.
    let words = "check the failing PR from monday";
    desk.events
        .send(Ok(RealtimeEvent::Heard(words.into())))
        .await
        .unwrap();
    until(socket, |f| f["event"]["type"] == "heard").await;
    desk.events
        .send(Ok(RealtimeEvent::ToolCall {
            call_id: "c1".into(),
            name: "send_to_teammate".into(),
            arguments: json!({ "words": words }).to_string(),
        }))
        .await
        .unwrap();
    let Asked::ToolResult {
        call_id,
        output,
        reply,
    } = next_asked(&mut desk.asked).await
    else {
        panic!("the front's tool call went unanswered")
    };
    assert_eq!(call_id, "c1");
    assert!(output.contains("Nothing is done yet"), "{output}");
    assert!(
        output.contains("working session") && !output.contains("Handed to"),
        "a direct call hands over as the teammate: {output}"
    );
    assert!(
        !reply,
        "the front acknowledged it as it sent it; no second word"
    );

    // The call thinks with the floor open while the teammate works, then
    // the teammate's spoken line goes to the front to be said, and the desk
    // speaks none of it itself.
    let mut listening = false;
    let said = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                frame = socket.next() => {
                    let frame: Value = serde_json::from_str(
                        frame.unwrap().unwrap().to_text().unwrap(),
                    )
                    .unwrap();
                    assert_ne!(frame["event"]["type"], "clip", "the desk spoke: {frame}");
                    if frame["event"]["type"] == "state"
                        && frame["event"]["state"] == "thinking"
                        && frame["event"]["listening"] == true
                    {
                        listening = true;
                    }
                }
                asked = desk.asked.recv() => {
                    if let Some(Asked::Say(text)) = asked {
                        return text;
                    }
                }
            }
        }
    })
    .await
    .expect("the teammate's line reached the front");
    assert!(listening, "the call showed the teammate working");
    assert_eq!(said, "The checks passed.");

    // A front that relays the line it was given back to the teammate is
    // refused, or the teammate would answer its own reply, round and round.
    desk.events
        .send(Ok(RealtimeEvent::ToolCall {
            call_id: "c2".into(),
            name: "send_to_teammate".into(),
            arguments: json!({ "words": said }).to_string(),
        }))
        .await
        .unwrap();
    let Asked::ToolResult {
        call_id,
        output,
        reply,
    } = next_asked(&mut desk.asked).await
    else {
        panic!("the echo went unanswered")
    };
    assert_eq!(call_id, "c2");
    assert!(output.starts_with("Not sent"), "{output}");
    assert!(reply, "a refusal is spoken to");

    // The front says it; that is the call's line, kept once.
    desk.events
        .send(Ok(RealtimeEvent::Saying("The checks passed.".into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::ResponseDone))
        .await
        .unwrap();
    let line = until(socket, |f| {
        f["event"]["type"] == "said" && f["event"]["id"] == "front-0"
    })
    .await;
    assert_eq!(line["event"]["text"], "The checks passed.");

    // Choosing silence is answered, and asks the front for nothing.
    desk.events
        .send(Ok(RealtimeEvent::ToolCall {
            call_id: "c2".into(),
            name: "remain_silent".into(),
            arguments: "{}".into(),
        }))
        .await
        .unwrap();
    assert_eq!(
        next_asked(&mut desk.asked).await,
        Asked::ToolResult {
            call_id: "c2".into(),
            output: String::new(),
            reply: false
        }
    );

    let ended = answer(
        socket,
        3,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
    assert_eq!(ended["ok"], true);
    let log = &desk.log;
    let record = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let lines = log.load(&hotline_core::log::StreamId::Call(call.clone()));
            if lines.iter().any(|v| v["text"] == "The checks passed.") {
                return lines;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the front's line reached the call's record");
    let texts: Vec<&str> = record.iter().filter_map(|v| v["text"].as_str()).collect();
    assert_eq!(
        texts.iter().filter(|text| **text == words).count(),
        1,
        "{record:?}"
    );
    assert_eq!(
        texts
            .iter()
            .filter(|text| **text == "The checks passed.")
            .count(),
        1,
        "{record:?}"
    );

    // The words went in as typed, with the call's origin, and the teammate
    // was told they are a transcript; nothing the front said is on its tape.
    let asked = requests
        .lock()
        .unwrap()
        .iter()
        .flat_map(|request| request["messages"].as_array().cloned().unwrap_or_default())
        .filter(|message| message["role"] == "user")
        .map(|message| message["content"].as_str().unwrap_or_default().to_string())
        .find(|content| content.contains("[Voice call: answer twice"))
        .expect("the turn's input carries the contract");
    assert!(
        asked.contains(&format!("{words}\n\n[This turn came from a voice call")),
        "{asked}"
    );
    assert!(asked.contains("behind an intermediary"), "{asked}");
    let tape = log.load(&hotline_core::log::StreamId::Tape(persona));
    let users: Vec<&Value> = tape.iter().filter(|v| v["kind"] == "user").collect();
    assert_eq!(users.len(), 1, "{tape:?}");
    assert_eq!(users[0]["text"], words, "the person's words, unrephrased");
    assert!(
        users[0]["id"]
            .as_str()
            .unwrap()
            .starts_with(&format!("voice:{call}:1:agent:"))
    );
    assert_eq!(tape.iter().filter(|v| v["kind"] == "chapter").count(), 1);
    assert!(
        !tape.iter().any(|v| {
            let line = v.to_string();
            line.contains("intermediary") || line.contains("Sent to your working session")
        }),
        "the front's side stays off the teammate's tape: {tape:?}"
    );
}

/// A front on an API key costs its minutes, on a subscription login nothing,
/// and a call is refused before it starts when the budget has no room.
#[tokio::test]
async fn a_fronts_minutes_are_counted_by_login_kind() {
    use async_trait::async_trait;
    use hotline_core::voice::realtime::{Realtime, RealtimeConfig, RealtimeSession};
    use hotline_core::voice::speech::{SpeechError, SpeechId};
    use hotline_core::wire::RoomHandle;

    struct Front {
        provider: &'static str,
        subscription: bool,
    }
    #[async_trait]
    impl Realtime for Front {
        fn id(&self) -> SpeechId {
            SpeechId {
                provider_id: self.provider.into(),
                model_id: "fixture".into(),
                voice: None,
            }
        }
        fn is_subscription(&self) -> bool {
            self.subscription
        }
        async fn open(&self, _: RealtimeConfig) -> Result<RealtimeSession, SpeechError> {
            Err(SpeechError::NothingToSay)
        }
    }

    let root = tempfile::tempdir().unwrap();
    let desk = hotline_core::desk::Desk::open_with_voice_services(
        root.path(),
        common::store(),
        Some(services()),
    )
    .unwrap();
    let calls = desk.voice().unwrap();
    let grok = Front {
        provider: "xai",
        subscription: false,
    };
    let gpt = Front {
        provider: "openai",
        subscription: false,
    };
    let login = Front {
        provider: "xai-subscription",
        subscription: true,
    };

    calls.front_ready(&grok).unwrap();
    calls.front_spend(&login, 600.0).unwrap();
    assert_eq!(calls.status().budget.spent_day_usd, 0.0);
    calls.front_spend(&grok, 60.0).unwrap();
    calls.front_spend(&gpt, 60.0).unwrap();
    let spent = calls.status().budget.spent_day_usd;
    assert!((spent - 0.18).abs() < 1e-9, "{spent}");
}

/// The PCM16 samples of a WAV file's `data` chunk.
fn wav_pcm(wav: &[u8]) -> Vec<u8> {
    let mut at = 12;
    while at + 8 <= wav.len() {
        let size = u32::from_le_bytes(wav[at + 4..at + 8].try_into().unwrap()) as usize;
        if &wav[at..at + 4] == b"data" {
            return wav[at + 8..(at + 8 + size).min(wav.len())].to_vec();
        }
        at += 8 + size + size % 2;
    }
    panic!("a WAV with no data chunk")
}

/// Sends `pcm` as a phone would, 100 ms of 16 kHz at a time and in time.
async fn speak_into(socket: &mut Socket, call: &str, seq: &mut u32, pcm: &[u8]) {
    for frame in pcm.chunks(3_200) {
        let id = 1_000 + *seq;
        send(socket, json!({"id":id,"cmd":"voice.front_audio","params":{"callId":call,"seq":*seq,"data":STANDARD.encode(frame)}})).await;
        *seq += 1;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The wired front against Grok Voice itself, with a scripted teammate:
/// the person's 16 kHz speech is heard and transcribed, the front calls
/// `send_to_teammate`, the teammate's spoken line comes back as 24 kHz
/// speech, and speaking over it is a barge-in. Needs an xAI API key in
/// `HOTLINE_LIVE_XAI_KEY` and is charged by the minute;
/// `cargo test -p hotline-core --test voice live -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "talks to Grok Voice with HOTLINE_LIVE_XAI_KEY"]
async fn live_a_grok_front_relays_to_the_teammate_and_yields_the_floor() {
    use hotline_core::voice::realtime::RealtimeSocket;
    use hotline_core::voice::speech::Endpoint;

    let key = std::env::var("HOTLINE_LIVE_XAI_KEY").expect("set HOTLINE_LIVE_XAI_KEY");
    let root = tempfile::tempdir().unwrap();
    let mut services = services();
    services.realtime = Some(Arc::new(RealtimeSocket::grok(Endpoint {
        provider_id: "xai".into(),
        base_url: "https://api.x.ai/v1".into(),
        key: Some(key),
    })));
    let desk = Arc::new(
        hotline_core::desk::Desk::open_with_voice_services(
            root.path(),
            common::store(),
            Some(services),
        )
        .unwrap(),
    );
    let log = desk.log.clone();
    let door = Door::bind(log.clone(), "front-test".into(), desk).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=front-test"))
        .await
        .unwrap();
    let socket = &mut socket;
    let provider = provider(TAGGED_REPLY).await;
    answer(socket, 90, json!({"cmd":"credential.custom_save","params":{"draft":{"name":"Front fixture","baseUrl":provider,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    let persona = answer(socket, 91, json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.path().to_str().unwrap()}}})).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    answer(
        socket,
        92,
        json!({"cmd":"session.start","params":{"personaId":persona}}),
    )
    .await;

    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"mode":"front","inputRate":16000}}),
    )
    .await;
    assert_eq!(
        started["result"]["front"],
        json!({"inputRate":16000,"outputRate":24000})
    );
    send(socket, json!({"id":2,"sub":{"call":call}})).await;
    until(socket, |f| f["sub"] == 2 && f["snapshot"].is_array()).await;
    let mut seq = 0;
    for attempt in 0..100 {
        let sent = answer(socket, 3, json!({"cmd":"voice.front_audio","params":{"callId":call,"seq":seq,"data":STANDARD.encode([0u8; 320])}})).await;
        seq += 1;
        if sent["ok"] == true {
            break;
        }
        if attempt < 3 {
            eprintln!("front not ready: {sent}");
        }
        assert!(attempt < 99, "the front never connected: {sent}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let speech = wav_pcm(&wav());
    let silence = vec![0u8; 32_000 * 2];
    speak_into(socket, &call, &mut seq, &speech).await;
    speak_into(socket, &call, &mut seq, &silence).await;

    // Heard, relayed, answered and said, within a minute.
    let mut heard = None;
    let mut frames = 0;
    let mut longest = 0;
    let mut said = Vec::new();
    let relayed = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let frame = until(socket, |f| f["event"].is_object()).await;
            let event = &frame["event"];
            match event["type"].as_str().unwrap() {
                "heard" => heard = Some(event["text"].as_str().unwrap().to_string()),
                "frontAudio" => {
                    frames += 1;
                    let bytes = STANDARD.decode(event["data"].as_str().unwrap()).unwrap();
                    longest = longest.max(bytes.len());
                }
                "said" => {
                    let text = event["text"].as_str().unwrap().to_string();
                    eprintln!("said {}: {text}", event["id"]);
                    let done = event["id"].as_str().unwrap().starts_with("front-")
                        && text.contains("checks passed");
                    said.push(text);
                    if done {
                        return;
                    }
                }
                _ => {}
            }
        }
    })
    .await;
    let tape = log.load(&hotline_core::log::StreamId::Tape(persona.clone()));
    let handed: Vec<&str> = tape
        .iter()
        .filter(|v| v["kind"] == "user")
        .filter_map(|v| v["text"].as_str())
        .collect();
    eprintln!(
        "heard: {heard:?}\nhanded to the teammate: {handed:?}\nsaid: {said:?}\nfrontAudio frames: {frames}"
    );
    let heard = heard.expect("the person's speech was transcribed");
    assert!(heard.to_lowercase().contains("mac"), "{heard}");
    assert!(!handed.is_empty(), "the front called send_to_teammate");
    assert!(frames > 0, "the front spoke");
    assert_eq!(longest, 4_800, "100 ms frames of 24 kHz PCM16");
    relayed.expect("the teammate's spoken line was said by the front");

    // The person talks over the front: it stops and yields the floor.
    speak_into(socket, &call, &mut seq, &speech).await;
    until(socket, |f| f["event"]["type"] == "frontStop").await;
    let ended = answer(
        socket,
        5,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
    assert_eq!(ended["ok"], true);
}
