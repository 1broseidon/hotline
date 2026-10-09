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
