use crate::contract::VoiceCallMode;
use crate::log::StreamId;
use crate::test_support::until;
use crate::wire::RoomHandle;
use async_trait::async_trait;
use hotline_voice::dispatcher::Context;
use hotline_voice::dispatcher::Dispatcher;
use hotline_voice::ledger::Kind;
use hotline_voice::speech::{Clip, SpeechOutput, SpeechSet};
use hotline_voice::spoken::{sentences, take_sentences};
use hotline_voice::test_support::validate_audio;
use hotline_voice::test_support::*;
use hotline_voice::test_support::{goodbye, own_voice, speech_ready};
use hotline_voice::*;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub(crate) struct Fake {
    pub transcript: Mutex<String>,
    pub answers: AtomicUsize,
    pub transcriptions: AtomicUsize,
    pub live_chunks: AtomicUsize,
    pub spoken: Mutex<Vec<String>>,
    pub delay: Mutex<Duration>,
    pub fail: bool,
    pub subscription: bool,
    pub output_mime: &'static str,
    pub fail_transcription: AtomicBool,
    pub answer_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    pub narration: Mutex<Option<Result<String, String>>>,
    pub narrations: AtomicUsize,
    /// Whether the fake call assistant's model costs nothing.
    pub free_assistant: bool,
    /// What the call assistant's rewrite of a reply comes back with, after
    /// `rewrite_delay`; none is scripted unless a test says so.
    pub rewrite: Mutex<Result<String, String>>,
    pub rewrite_delay: Mutex<Duration>,
    /// The person's words and the written reply each rewrite was handed.
    pub rewrites: Mutex<Vec<(String, String)>>,
    pub pointer_instructions: Mutex<Vec<String>>,
}
impl Default for Fake {
    fn default() -> Self {
        Self {
            transcript: Mutex::new("hello".into()),
            answers: AtomicUsize::new(0),
            transcriptions: AtomicUsize::new(0),
            live_chunks: AtomicUsize::new(0),
            spoken: Mutex::new(Vec::new()),
            delay: Mutex::new(Duration::ZERO),
            fail: false,
            subscription: false,
            output_mime: "audio/wav",
            fail_transcription: AtomicBool::new(false),
            answer_gate: Mutex::new(None),
            narration: Mutex::new(None),
            narrations: AtomicUsize::new(0),
            free_assistant: false,
            rewrite: Mutex::new(Err("No rewrite is scripted.".into())),
            rewrite_delay: Mutex::new(Duration::ZERO),
            rewrites: Mutex::new(Vec::new()),
            pointer_instructions: Mutex::new(Vec::new()),
        }
    }
}
#[async_trait]
impl speech::Speech for Fake {
    fn output_mime(&self) -> &'static str {
        self.output_mime
    }
    fn id(&self) -> SpeechId {
        SpeechId {
            provider_id: if self.subscription {
                "xai-subscription"
            } else {
                "fixture"
            }
            .into(),
            model_id: "fake".into(),
            voice: None,
        }
    }
    fn is_subscription(&self) -> bool {
        self.subscription
    }
    async fn transcribe(&self, _: Clip) -> Result<String, speech::SpeechError> {
        self.transcriptions.fetch_add(1, Ordering::SeqCst);
        if self.fail_transcription.load(Ordering::SeqCst) {
            return Err(speech::SpeechError::Unreachable {
                provider_id: "fixture".into(),
            });
        }
        Ok(lock(&self.transcript).clone())
    }
    fn supports_live_input(&self) -> bool {
        true
    }
    async fn transcribe_live(
        &self,
        mut input: mpsc::Receiver<Vec<u8>>,
        sample_rate: u32,
    ) -> Result<String, speech::SpeechError> {
        assert_eq!(sample_rate, 16_000);
        self.transcriptions.fetch_add(1, Ordering::SeqCst);
        while let Some(bytes) = input.recv().await {
            assert_ne!(bytes, [] as [u8; 0]);
            self.live_chunks.fetch_add(1, Ordering::SeqCst);
        }
        Ok(lock(&self.transcript).clone())
    }
    async fn speak(&self, text: &str) -> Result<Clip, speech::SpeechError> {
        lock(&self.spoken).push(text.into());
        let delay = *lock(&self.delay);
        tokio::time::sleep(delay).await;
        if self.fail {
            return Err(speech::SpeechError::Unreachable {
                provider_id: "fixture".into(),
            });
        }
        Ok(Clip {
            mime: "audio/wav".into(),
            bytes: wav(),
        })
    }
}
#[async_trait]
impl Dispatcher for Fake {
    fn free(&self) -> bool {
        self.free_assistant
    }
    fn id(&self) -> VoiceModel {
        model(speech::Speech::id(self))
    }
    async fn answer(&self, _: Context, _: &str, _: Arc<Budget>) -> Result<String, String> {
        self.answers.fetch_add(1, Ordering::SeqCst);
        let gate = lock(&self.answer_gate).clone();
        if let Some(gate) = gate {
            gate.acquire().await.unwrap().forget();
        }
        Ok("The first sentence. The second sentence.".into())
    }
    async fn narrate(
        &self,
        name: &str,
        text: &str,
        _: &str,
        _: Arc<Budget>,
    ) -> Result<String, String> {
        self.narrations.fetch_add(1, Ordering::SeqCst);
        let gate = lock(&self.answer_gate).clone();
        if let Some(gate) = gate {
            gate.acquire().await.unwrap().forget();
        }
        let result = lock(&self.narration).clone();
        if let Some(result) = result {
            return result;
        }
        Ok(format!("{name} says: {text}"))
    }
    async fn rewrite(
        &self,
        words: &str,
        written: &str,
        instruction: &str,
        _: Arc<Budget>,
    ) -> Result<String, String> {
        if instruction.contains("Only supply the brief contextual pointer") {
            lock(&self.pointer_instructions).push(instruction.into());
            return Ok("The result is available in our chat.".into());
        }
        lock(&self.rewrites).push((words.into(), written.into()));
        let delay = *lock(&self.rewrite_delay);
        tokio::time::sleep(delay).await;
        lock(&self.rewrite).clone()
    }
}
pub(crate) fn services() -> Services {
    with_fake(Arc::new(Fake::default()))
}
pub(crate) fn with_fake(fake: Arc<Fake>) -> Services {
    Services {
        speech: SpeechSet {
            stt: fake.clone(),
            tts: fake.clone(),
            fallback_tts: None,
        },
        dispatcher: Some(fake),
        realtime: None,
    }
}
fn desk(services: Services) -> (tempfile::TempDir, Arc<crate::desk::Desk>, Arc<Calls>) {
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_voice_services(
            root.path(),
            Arc::new(crate::credentials::MemoryStore::default()),
            Some(services),
        )
        .unwrap(),
    );
    let calls = desk.parts().voice.unwrap();
    (root, desk, calls)
}
fn wav() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend(32036u32.to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(16000u32.to_le_bytes());
    bytes.extend(32000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(32000u32.to_le_bytes());
    bytes.resize(32044, 0);
    bytes
}
async fn event(
    rx: &mut broadcast::Receiver<VoiceEvent>,
    predicate: impl Fn(&VoiceEvent) -> bool,
) -> VoiceEvent {
    let mut seen = Vec::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let event = rx.recv().await.unwrap();
            if predicate(&event) {
                return event;
            }
            seen.push(event);
        }
    })
    .await
    .unwrap_or_else(|error| panic!("{error}; preceding voice events: {seen:?}"))
}
fn utterance(calls: &Calls, id: &str, seq: u32) -> Result<(), String> {
    calls.utterance(id, seq, "audio/wav", &STANDARD.encode(wav()), 1000)
}

fn spending(desk: &crate::desk::Desk, value: Value) {
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event("setting", json!({"id": "spending", "value": value})),
        )
        .unwrap();
}

/// The teammate Mack, made through the room's own command.
async fn mack(desk: &Arc<crate::desk::Desk>) -> String {
    use crate::contract::Command;
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let create: Command = serde_json::from_value(json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Keep the build green","cwd":desk.log.root().to_str().unwrap()}}})).unwrap();
    crate::wire::commands::run(create, &desk.log, &handle, &desk.parts())
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A direct call that is a thread from the start: the teammate Mack, and a
/// call to it that names Mack as its target when it begins.
async fn direct_call() -> (
    tempfile::TempDir,
    Arc<crate::desk::Desk>,
    Arc<Calls>,
    String,
    String,
    broadcast::Receiver<VoiceEvent>,
    Arc<Fake>,
) {
    direct_call_with(true).await
}

/// The same, on a desk that has a call assistant or none.
async fn direct_call_with(
    assistant: bool,
) -> (
    tempfile::TempDir,
    Arc<crate::desk::Desk>,
    Arc<Calls>,
    String,
    String,
    broadcast::Receiver<VoiceEvent>,
    Arc<Fake>,
) {
    let fake = Arc::new(Fake::default());
    *lock(&fake.transcript) = "Can you check the build?".into();
    let mut services = with_fake(fake.clone());
    if !assistant {
        services.dispatcher = None;
    }
    let (root, desk, calls) = desk(services);
    let persona = mack(&desk).await;
    let id = Uuid::new_v4().to_string();
    calls
        .start_target(&id, Some(persona.clone()), false, desk.clone())
        .unwrap();
    let (_, rx) = calls.subscribe(&id).unwrap();
    (root, desk, calls, id, persona, rx, fake)
}

/// The call has heard turn `seq`, as if the person had said it: its replies
/// are this call's.
fn on_turn(calls: &Calls, id: &str, seq: u32) -> Origin {
    calls.test_begin_turn(id);
    calls
        .test_change(id, |call| {
            call.seq = Some(seq);
            Ok(())
        })
        .unwrap();
    Origin {
        call_id: id.into(),
        seq,
        direct: true,
        front: false,
    }
}

/// Every `said` and `clip` of a reply until its final clip.
async fn one_reply(
    rx: &mut broadcast::Receiver<VoiceEvent>,
) -> (Vec<(String, String)>, Vec<(String, u32, bool)>) {
    let mut said = Vec::new();
    let mut clips = Vec::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match rx.recv().await.unwrap() {
                VoiceEvent::Said { id, text } => said.push((id, text)),
                VoiceEvent::Clip {
                    id,
                    index,
                    r#final,
                    data,
                    ..
                } => {
                    clips.push((id, index, data.is_empty()));
                    if r#final {
                        return;
                    }
                }
                VoiceEvent::Delivery { .. } => panic!("a teammate's own reply is no delivery"),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    (said, clips)
}

fn call_link(desk: &crate::desk::Desk, persona: &str) -> Option<Value> {
    desk.log
        .load(&StreamId::Tape(persona.into()))
        .into_iter()
        .find(|event| event["kind"] == "link" && event["threadKind"] == "call")
}

mod budgets;
mod cards;
mod clips;
mod desk_calls;
mod direct;
mod failures;
mod front_calls;
mod handoffs;
mod streaming;
mod text;

use base64::{Engine, engine::general_purpose::STANDARD};
use hotline_contract::{VoiceEndReason, VoiceEvent, VoiceInputMode, VoiceModel, VoiceState};
use hotline_voice::metering::{BUDGET_ERROR, Budget};
use hotline_voice::speech::SpeechId;

use hotline_voice::settings::VoiceSettings;
