//! One owner call per desk, with bounded work and complete sentence clips.

pub mod dispatcher;
pub mod ledger;
pub mod metering;
pub mod settings;
pub mod speech;

use crate::contract::{
    VoiceBudget, VoiceCall, VoiceEndReason, VoiceEvent, VoiceInputMode, VoiceModel, VoiceState,
    VoiceStatus,
};
use crate::{log::Log, session::Room, vault::Vault, wire::RoomHandle};
use base64::{Engine, engine::general_purpose::STANDARD};
use dispatcher::{Context, Dispatcher, ProviderDispatcher};
use ledger::Kind;
use metering::{BUDGET_ERROR, Budget};
use settings::VoiceSettings;
use speech::{Clip, SpeechId, SpeechOutput, SpeechSet};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// This tape is indexed, but has no persona and never enters the roster.
pub const TAPE_ID: &str = "voice-dispatcher";
const IDLE: Duration = Duration::from_secs(600);
const RETRY_LINE: &str = "Sorry, say that again.";
const ERROR_LINE: &str = "Voice keeps failing. Please continue by text.";
const MAX_FAILURES: u8 = 3;
const MAX_AUDIO: usize = 2 * 1024 * 1024;
const BUDGET_LINE: &str = "The voice budget is unavailable or spent. Chat carries on by text.";

/// Provider seams for an embedded desk or a scripted client. Production desks
/// resolve both from the vault on each call and before each utterance.
#[derive(Clone)]
pub struct Services {
    pub speech: SpeechSet,
    pub dispatcher: Arc<dyn Dispatcher>,
}

/// Text input carries no remote listener, even when an audio listener is configured.
#[derive(Clone)]
struct CallSpeech {
    stt: Option<Arc<dyn speech::Speech>>,
    tts: Arc<dyn speech::Speech>,
    fallback_tts: Option<Arc<dyn speech::Speech>>,
}

impl CallSpeech {
    fn audio(speech: SpeechSet) -> Self {
        Self {
            stt: Some(speech.stt),
            tts: speech.tts,
            fallback_tts: speech.fallback_tts,
        }
    }
    fn output(speech: SpeechOutput) -> Self {
        Self {
            stt: None,
            tts: speech.tts,
            fallback_tts: speech.fallback_tts,
        }
    }
}

enum TranscriptSource {
    Audio { duration: u32, bytes: usize },
    Device,
}

impl TranscriptSource {
    fn permits_goodbye(&self) -> bool {
        match self {
            Self::Audio { duration, bytes } => speech::plausible_goodbye(*duration, *bytes),
            Self::Device => true,
        }
    }
}

/// Internal provenance; never accepted from a session command's parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Origin {
    pub call_id: String,
    pub seq: u32,
    pub direct: bool,
}

impl Origin {
    pub(crate) fn event_id(&self, event: &str) -> String {
        format!(
            "voice:{}:{}:{}:{event}",
            self.call_id,
            self.seq,
            if self.direct { "agent" } else { "desk" }
        )
    }
    pub(crate) fn from_event_id(id: &str) -> Option<Self> {
        let mut parts = id.strip_prefix("voice:")?.split(':');
        let call_id = parts.next()?.to_string();
        Uuid::parse_str(&call_id).ok()?;
        let seq = parts.next()?.parse().ok()?;
        let direct = match parts.next()? {
            "agent" => true,
            "desk" => false,
            _ => return None,
        };
        Uuid::parse_str(parts.next()?).ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            call_id,
            seq,
            direct,
        })
    }
}

struct Input {
    seq: u32,
    index: u32,
    bytes: usize,
    sender: mpsc::Sender<Vec<u8>>,
    committed: Arc<AtomicU32>,
    stt: Arc<dyn speech::Speech>,
}

struct Call {
    id: String,
    output: String,
    target: Option<String>,
    stream_audio: bool,
    input_mode: VoiceInputMode,
    speech_services: CallSpeech,
    input: Option<Input>,
    spoken_events: VecDeque<String>,
    connection_bound: bool,
    state: VoiceState,
    reason: Option<VoiceEndReason>,
    seq: Option<u32>,
    activity: Instant,
    events: broadcast::Sender<VoiceEvent>,
    work: mpsc::Sender<Work>,
    cancel: CancellationToken,
    speech: CancellationToken,
    deliveries: CancellationToken,
    utterance_pending: bool,
    first_clip_started: Option<Instant>,
}

impl Call {
    fn snapshot(&self) -> VoiceEvent {
        VoiceEvent::State {
            state: self.state,
            reason: self.reason,
        }
    }
    fn state(&mut self, state: VoiceState, reason: Option<VoiceEndReason>) {
        self.state = state;
        self.reason = reason;
        let _ = self.events.send(self.snapshot());
        if state == VoiceState::Ended {
            self.input = None;
            self.cancel.cancel();
            self.speech.cancel();
            self.deliveries.cancel();
        }
    }
}

#[derive(Clone)]
struct Delivery {
    persona: String,
    event: String,
    name: String,
    text: String,
}

enum Work {
    Text {
        seq: u32,
        text: String,
        speech: CancellationToken,
    },
    Live {
        seq: u32,
        input: Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
        committed: Arc<AtomicU32>,
        stt: Arc<dyn speech::Speech>,
        speech: CancellationToken,
    },
    Utterance {
        seq: u32,
        clip: Clip,
        duration: u32,
        speech: CancellationToken,
    },
    Delivery(Delivery, CancellationToken),
    Notice(Delivery, CancellationToken),
}

pub struct Calls {
    log: Log,
    vault: Arc<Vault>,
    room: Weak<Room>,
    injected: Option<Services>,
    ledger: Arc<Budget>,
    calls: Mutex<VecDeque<Call>>,
    narration: Arc<tokio::sync::Semaphore>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn model(id: SpeechId) -> VoiceModel {
    VoiceModel {
        provider_id: id.provider_id,
        model_id: id.model_id,
        voice: id.voice,
    }
}

impl Calls {
    pub(crate) fn new(
        log: Log,
        vault: Arc<Vault>,
        room: Weak<Room>,
        injected: Option<Services>,
    ) -> Arc<Self> {
        let calls = Arc::new(Self {
            ledger: Arc::new(Budget::open(log.clone())),
            log,
            vault,
            room,
            injected,
            calls: Mutex::new(VecDeque::new()),
            narration: Arc::new(tokio::sync::Semaphore::new(1)),
        });
        let weak = Arc::downgrade(&calls);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let Some(calls) = weak.upgrade() else { break };
                calls.expire();
            }
        });
        calls
    }

    fn expire(&self) {
        for call in lock(&self.calls).iter_mut() {
            if call.state != VoiceState::Ended && call.activity.elapsed() >= IDLE {
                call.state(VoiceState::Ended, Some(VoiceEndReason::Idle));
            }
        }
    }

    fn settings(&self) -> VoiceSettings {
        VoiceSettings::from_log(&self.log)
    }
    fn dispatcher(&self) -> Result<Arc<dyn Dispatcher>, String> {
        if let Some(services) = &self.injected {
            return Ok(services.dispatcher.clone());
        }
        ProviderDispatcher::resolve(self.vault.clone(), &self.log)
    }

    /// What voice has spent so far against its caps, from its own tally.
    pub(crate) fn balance(&self) -> ledger::Budget {
        self.ledger.balance()
    }

    pub fn status(&self) -> VoiceStatus {
        self.status_for(VoiceInputMode::Audio)
    }

    pub fn status_for(&self, input_mode: VoiceInputMode) -> VoiceStatus {
        let budget = self.ledger.balance();
        let speech = self.resolve_speech(input_mode);
        let dispatcher = self.dispatcher();
        let budget_error = self.ledger.check().err().map(|e| e.to_string());
        let direct_available = speech.is_ok() && budget_error.is_none();
        let unavailable = speech
            .as_ref()
            .err()
            .cloned()
            .or_else(|| dispatcher.as_ref().err().cloned())
            .or(budget_error);
        VoiceStatus {
            capabilities: vec!["voiceDirectCalls".into(), "voiceTextInput".into()],
            available: unavailable.is_none(),
            direct_available,
            unavailable,
            stt: speech
                .as_ref()
                .ok()
                .and_then(|s| s.stt.as_ref().map(|s| model(s.id()))),
            tts: speech.as_ref().ok().map(|s| model(s.tts.id())),
            fallback_tts: speech
                .as_ref()
                .ok()
                .and_then(|s| s.fallback_tts.as_ref().map(|s| model(s.id()))),
            dispatcher: dispatcher.as_ref().ok().map(|d| d.id()),
            budget: VoiceBudget {
                day_usd: budget.day_usd,
                month_usd: budget.month_usd,
                spent_day_usd: budget.spent_day_usd,
                spent_month_usd: budget.spent_month_usd,
            },
        }
    }

    pub fn start(
        self: &Arc<Self>,
        id: &str,
        room: Arc<dyn RoomHandle>,
    ) -> Result<VoiceCall, String> {
        self.start_target(id, None, false, room)
    }

    pub fn start_target(
        self: &Arc<Self>,
        id: &str,
        target: Option<String>,
        stream_audio: bool,
        room: Arc<dyn RoomHandle>,
    ) -> Result<VoiceCall, String> {
        self.start_with_input(id, target, stream_audio, VoiceInputMode::Audio, room)
    }

    pub fn start_with_input(
        self: &Arc<Self>,
        id: &str,
        target: Option<String>,
        stream_audio: bool,
        input_mode: VoiceInputMode,
        room: Arc<dyn RoomHandle>,
    ) -> Result<VoiceCall, String> {
        if let Some(target) = &target
            && !crate::room::roster(&self.log)
                .iter()
                .any(|persona| &persona.id == target)
        {
            return Err("That teammate is no longer in the room.".into());
        }
        Uuid::parse_str(id).map_err(|_| "A voice call needs a UUID.".to_string())?;
        let mut calls = lock(&self.calls);
        if let Some(call) = calls.iter().find(|call| call.id == id)
            && (call.target != target
                || call.stream_audio != stream_audio
                || call.input_mode != input_mode)
        {
            return Err("A call cannot change its target or input/output mode.".into());
        }
        if !calls.iter().any(|call| call.id == id) {
            // Direct calls do not need a separate routing model.
            let speech_services = self.resolve_speech(input_mode)?;
            if target.is_none() {
                self.dispatcher()?;
            }
            self.ledger.check().map_err(|e| e.to_string())?;
            for call in calls
                .iter_mut()
                .filter(|call| call.state != VoiceState::Ended)
            {
                call.state(VoiceState::Ended, Some(VoiceEndReason::Replaced));
            }
            while calls.len() >= 32 {
                calls.pop_front();
            }
            let (tx, rx) = mpsc::channel(16);
            let cancel = CancellationToken::new();
            calls.push_back(Call {
                id: id.into(),
                output: speech_services.tts.output_mime().into(),
                target: target.clone(),
                stream_audio,
                input_mode,
                speech_services,
                input: None,
                spoken_events: VecDeque::new(),
                connection_bound: false,
                state: VoiceState::Listening,
                reason: None,
                seq: None,
                activity: Instant::now(),
                events: broadcast::channel(128).0,
                work: tx,
                cancel: cancel.clone(),
                speech: CancellationToken::new(),
                deliveries: CancellationToken::new(),
                utterance_pending: false,
                first_clip_started: None,
            });
            let this = self.clone();
            let id = id.to_string();
            let context = Context::new(self.log.clone(), room, cancel.clone());
            tokio::spawn(async move {
                this.run(id, context, rx).await;
            });
        }
        Ok(VoiceCall {
            call_id: id.into(),
            persona_id: target,
            input_mode,
            input: {
                let mut input = if input_mode == VoiceInputMode::Text {
                    vec!["text/plain".into()]
                } else {
                    vec!["audio/wav".into(), "audio/mp4".into()]
                };
                if stream_audio
                    && calls.iter().find(|c| c.id == id).is_some_and(|c| {
                        c.speech_services
                            .stt
                            .as_ref()
                            .is_some_and(|s| s.supports_live_input())
                    })
                {
                    input.push("audio/pcm".into());
                }
                input
            },
            output: calls
                .iter()
                .find(|call| call.id == id)
                .expect("inserted call")
                .output
                .clone(),
        })
    }

    /// A revoked or disconnected owner may not leave a paid dispatcher running.
    pub(crate) fn bind_connection(self: &Arc<Self>, id: String, cancel: CancellationToken) {
        let this = Arc::downgrade(self);
        let call_cancel = lock(&self.calls)
            .iter_mut()
            .find(|c| c.id == id && !c.connection_bound)
            .map(|c| {
                c.connection_bound = true;
                c.cancel.clone()
            });
        if let Some(call_cancel) = call_cancel {
            tokio::spawn(async move {
                tokio::select! {
                    _ = cancel.cancelled() => if let Some(this) = this.upgrade() { let _ = this.end(&id); },
                    _ = call_cancel.cancelled() => {},
                }
            });
        }
    }

    pub fn subscribe(
        &self,
        id: &str,
    ) -> Result<(VoiceEvent, broadcast::Receiver<VoiceEvent>), String> {
        let calls = lock(&self.calls);
        let call = calls
            .iter()
            .find(|call| call.id == id)
            .ok_or("That voice call is no longer available.")?;
        Ok((call.snapshot(), call.events.subscribe()))
    }

    fn change<T>(
        &self,
        id: &str,
        change: impl FnOnce(&mut Call) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut calls = lock(&self.calls);
        let call = calls
            .iter_mut()
            .find(|call| call.id == id)
            .ok_or("That voice call is no longer available.")?;
        if call.state == VoiceState::Ended {
            return Err("That voice call has ended.".into());
        }
        change(call)
    }

    pub fn end(&self, id: &str) -> Result<(), String> {
        // Ending the same call twice is harmless.
        if lock(&self.calls)
            .iter()
            .any(|c| c.id == id && c.state == VoiceState::Ended)
        {
            return Ok(());
        }
        self.finish(id, VoiceEndReason::Client)
    }
    fn finish(&self, id: &str, reason: VoiceEndReason) -> Result<(), String> {
        self.change(id, |call| {
            call.state(VoiceState::Ended, Some(reason));
            Ok(())
        })
    }

    pub fn hold(&self, id: &str, hold: bool) -> Result<(), String> {
        self.change(id, |call| {
            call.activity = Instant::now();
            if hold {
                call.input = None;
                call.speech.cancel();
                call.deliveries.cancel();
                call.deliveries = CancellationToken::new();
                call.state(VoiceState::Held, None);
            } else if call.state == VoiceState::Held {
                call.state(
                    if call.utterance_pending {
                        VoiceState::Thinking
                    } else {
                        VoiceState::Listening
                    },
                    None,
                );
            }
            Ok(())
        })
    }

    pub fn interrupt(&self, id: &str) -> Result<(), String> {
        self.change(id, |call| {
            call.activity = Instant::now();
            call.input = None;
            call.speech.cancel();
            call.deliveries.cancel();
            call.deliveries = CancellationToken::new();
            if call.state != VoiceState::Held {
                call.state(
                    if call.utterance_pending {
                        VoiceState::Thinking
                    } else {
                        VoiceState::Listening
                    },
                    None,
                );
            }
            Ok(())
        })
    }

    pub fn utterance(
        &self,
        id: &str,
        seq: u32,
        mime: &str,
        data: &str,
        duration: u32,
    ) -> Result<(), String> {
        if !(1..=20_000).contains(&duration) || !matches!(mime, "audio/wav" | "audio/mp4") {
            return Err("Send a WAV or AAC/MP4 utterance lasting at most 20 seconds.".into());
        }
        if data.len() > MAX_AUDIO * 4 / 3 + 4 {
            return Err("That voice clip is too large.".into());
        }
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| "Voice audio must be standard base64.".to_string())?;
        if bytes.is_empty() || bytes.len() > MAX_AUDIO {
            return Err("That voice clip is empty or too large.".into());
        }
        let duration = validate_audio(mime, &bytes, duration)?;
        self.queue_utterance(id, seq, VoiceInputMode::Audio, |speech| Work::Utterance {
            seq,
            clip: Clip {
                mime: mime.into(),
                bytes,
            },
            duration,
            speech,
        })
    }

    /// Only a finalized device transcript enters the paid response pipeline.
    pub fn text(&self, id: &str, seq: u32, text: &str) -> Result<(), String> {
        if text.len() > 32_000 || text.chars().count() > 8_000 {
            return Err("A voice transcript may contain at most 8,000 characters.".into());
        }
        let text = text.trim();
        if text.is_empty() {
            return Err("A finalized voice transcript cannot be empty.".into());
        }
        self.queue_utterance(id, seq, VoiceInputMode::Text, |speech| Work::Text {
            seq,
            text: text.into(),
            speech,
        })
    }

    fn queue_utterance(
        &self,
        id: &str,
        seq: u32,
        input_mode: VoiceInputMode,
        work: impl FnOnce(CancellationToken) -> Work,
    ) -> Result<(), String> {
        self.change(id, |call| {
            if call.input_mode != input_mode {
                return Err(match call.input_mode {
                    VoiceInputMode::Text => {
                        "This call accepts finalized text, not microphone audio."
                    }
                    VoiceInputMode::Audio => {
                        "This call accepts microphone audio, not device transcripts."
                    }
                }
                .into());
            }
            if call.state == VoiceState::Held {
                return Err("Resume the call before speaking.".into());
            }
            if call.seq.is_some_and(|previous| seq <= previous) {
                return Err("Utterance sequence numbers must rise.".into());
            }
            if call.utterance_pending {
                return Err("The previous utterance is still being handled.".into());
            }
            let speech = CancellationToken::new();
            call.speech_services = self.resolve_speech(input_mode)?;
            call.work
                .try_send(work(speech.clone()))
                .map_err(|_| "The voice call is busy. Try again shortly.".to_string())?;
            call.input = None;
            call.speech.cancel();
            call.speech = speech;
            call.deliveries.cancel();
            call.deliveries = CancellationToken::new();
            call.utterance_pending = true;
            call.first_clip_started = Some(Instant::now());
            call.seq = Some(seq);
            call.activity = Instant::now();
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
    }

    fn resolve_speech(&self, input_mode: VoiceInputMode) -> Result<CallSpeech, String> {
        if let Some(services) = &self.injected {
            return Ok(match input_mode {
                VoiceInputMode::Audio => CallSpeech::audio(services.speech.clone()),
                VoiceInputMode::Text => CallSpeech::output(SpeechOutput {
                    tts: services.speech.tts.clone(),
                    fallback_tts: services.speech.fallback_tts.clone(),
                }),
            });
        }
        match input_mode {
            VoiceInputMode::Audio => {
                speech::resolve(&self.vault, &self.settings()).map(CallSpeech::audio)
            }
            VoiceInputMode::Text => {
                speech::resolve_output(&self.vault, &self.settings()).map(CallSpeech::output)
            }
        }
    }

    fn speech_for(&self, id: &str) -> Result<CallSpeech, String> {
        self.change(id, |call| {
            // Connection revocation ends the call; disconnected providers may not
            // continue to use a key retained by its current sentence adapters.
            if !self.connected(call.speech_services.tts.as_ref()) {
                return Err("The voice provider is no longer connected.".into());
            }
            Ok(call.speech_services.clone())
        })
    }

    fn connected(&self, speech: &dyn speech::Speech) -> bool {
        if self.injected.is_some() {
            return true;
        }
        let auth = self.vault.provider_auth();
        if speech.is_subscription() {
            matches!(
                auth.get("xai"),
                Some(crate::session::ProviderAuth::StoredLogin { .. })
            )
        } else {
            auth.contains_key(&speech.id().provider_id)
        }
    }

    pub fn audio(
        &self,
        id: &str,
        seq: u32,
        index: u32,
        data: &str,
        last: bool,
    ) -> Result<(), String> {
        if data.len() > 32_768 * 4 / 3 + 4 {
            return Err("A voice audio chunk is too large.".into());
        }
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| "Voice audio must be standard base64.".to_string())?;
        if bytes.len() > 32_768 || bytes.len() % 2 != 0 || (bytes.is_empty() && !last) {
            return Err("Send at most 32 KiB of PCM16 per chunk.".into());
        }
        self.change(id, |call| {
            if call.state == VoiceState::Held {
                return Err("Resume the call before speaking.".into());
            }
            if call.input_mode != VoiceInputMode::Audio
                || !call.stream_audio
                || !call
                    .speech_services
                    .stt
                    .as_ref()
                    .is_some_and(|s| s.supports_live_input())
            {
                return Err("This call does not accept live PCM.".into());
            }
            if index == 0 {
                if bytes.is_empty()
                    || call.utterance_pending
                    || call.seq.is_some_and(|previous| seq <= previous)
                {
                    return Err("Start a new, increasing voice sequence with audio.".into());
                }
                let services = self.resolve_speech(VoiceInputMode::Audio)?;
                let stt = services
                    .stt
                    .as_ref()
                    .filter(|s| s.supports_live_input())
                    .cloned()
                    .ok_or("The voice provider no longer accepts live PCM.")?;
                if !self.connected(stt.as_ref()) {
                    return Err("The voice provider no longer accepts live PCM.".into());
                }
                let work_slot = call
                    .work
                    .try_reserve()
                    .map_err(|_| "The voice call is busy. Try again shortly.".to_string())?;
                self.pay(
                    Kind::Stt,
                    ledger::stt_live_usd(&stt.id().provider_id, bytes.len() as f64 / 32_000.0),
                )?;
                let (sender, input) = mpsc::channel(32);
                sender
                    .try_send(bytes.clone())
                    .map_err(|_| "The voice input is busy.".to_string())?;
                let committed = Arc::new(AtomicU32::new(0));
                let speech = CancellationToken::new();
                work_slot.send(Work::Live {
                    seq,
                    input: Mutex::new(Some(input)),
                    committed: committed.clone(),
                    stt: stt.clone(),
                    speech: speech.clone(),
                });
                call.speech.cancel();
                call.speech = speech;
                call.deliveries.cancel();
                call.deliveries = CancellationToken::new();
                call.speech_services = services;
                call.seq = Some(seq);
                call.utterance_pending = true;
                call.input = Some(Input {
                    seq,
                    index: 1,
                    bytes: bytes.len(),
                    sender,
                    committed,
                    stt,
                });
            } else {
                let input = call
                    .input
                    .as_mut()
                    .ok_or("There is no live voice input to continue.")?;
                if input.seq != seq || input.index != index {
                    return Err("Voice audio chunks must arrive in order.".into());
                }
                if input.bytes + bytes.len() > 640_000 {
                    return Err("A voice turn may last at most 20 seconds.".into());
                }
                if !bytes.is_empty() {
                    let slot = input
                        .sender
                        .try_reserve()
                        .map_err(|_| "The live voice input is busy. Call again.".to_string())?;
                    // No provider work is accepted before its budget reservation.
                    self.pay(
                        Kind::Stt,
                        ledger::stt_live_usd(
                            &input.stt.id().provider_id,
                            bytes.len() as f64 / 32_000.0,
                        ),
                    )?;
                    slot.send(bytes.clone());
                }
                input.bytes += bytes.len();
                input.index += 1;
            }
            call.activity = Instant::now();
            if last {
                let input = call.input.take().expect("accepted input");
                input.committed.store(input.bytes as u32, Ordering::SeqCst);
                drop(input.sender);
                call.first_clip_started = Some(Instant::now());
                call.state(VoiceState::Thinking, None);
            }
            Ok(())
        })
    }

    fn emit(&self, id: &str, event: VoiceEvent) {
        let _ = self.change(id, |call| {
            let _ = call.events.send(event);
            Ok(())
        });
    }

    fn system_line(
        &self,
        id: &str,
        text: &str,
        audio: &[u8],
        interrupted: Option<&CancellationToken>,
    ) {
        let line = self
            .record("agent", text)
            .unwrap_or_else(|_| Uuid::new_v4().to_string());
        self.emit(
            id,
            VoiceEvent::Said {
                id: line.clone(),
                text: text.into(),
            },
        );
        self.clip(
            id,
            &line,
            0,
            true,
            &Clip {
                mime: "audio/wav".into(),
                bytes: audio.to_vec(),
            },
            interrupted,
        );
    }

    async fn run(self: Arc<Self>, id: String, context: Context, mut work: mpsc::Receiver<Work>) {
        let mut failures = 0u8;
        loop {
            let next = tokio::select! { biased; _ = context.cancel.cancelled() => None, work = work.recv() => work };
            let Some(next) = next else {
                break;
            };
            let result = match &next {
                Work::Text { seq, text, speech } => {
                    self.respond(
                        &id,
                        &context.for_utterance(),
                        *seq,
                        text.clone(),
                        TranscriptSource::Device,
                        speech,
                    )
                    .await
                }
                Work::Live {
                    seq,
                    input,
                    committed,
                    stt,
                    speech,
                } => {
                    let receiver = lock(input).take().expect("one live consumer");
                    let transcribed = tokio::select! {
                        _ = context.cancel.cancelled() => Ok(String::new()),
                        _ = speech.cancelled() => Ok(String::new()),
                        result = tokio::time::timeout(Duration::from_secs(50), stt.transcribe_live(receiver, 16_000)) =>
                            result.map_err(|_| "The live voice input timed out.".to_string()).and_then(|r| r.map_err(|e| e.to_string())),
                    };
                    let bytes = committed.load(Ordering::SeqCst);
                    match transcribed {
                        Ok(text) if bytes != 0 && !speech.is_cancelled() => {
                            self.respond(
                                &id,
                                &context.for_utterance(),
                                *seq,
                                text,
                                TranscriptSource::Audio {
                                    duration: bytes / 32,
                                    bytes: bytes as usize,
                                },
                                speech,
                            )
                            .await
                        }
                        Ok(_) => Ok(()),
                        Err(error) => Err(error),
                    }
                }
                Work::Utterance {
                    seq,
                    clip,
                    duration,
                    speech,
                } => {
                    self.hear(
                        &id,
                        &context.for_utterance(),
                        *seq,
                        clip.clone(),
                        *duration,
                        speech,
                    )
                    .await
                }
                Work::Delivery(delivery, speech) | Work::Notice(delivery, speech) => {
                    let held = self
                        .change(&id, |c| Ok(c.state == VoiceState::Held))
                        .unwrap_or(true);
                    if held || speech.is_cancelled() {
                        self.push(delivery, &delivery.text);
                        Ok(())
                    } else if matches!(next, Work::Notice(..)) {
                        self.say(&id, &delivery.text, speech, &context.cancel).await
                    } else {
                        self.narrate(&id, &context, delivery, speech).await
                    }
                }
            };
            let budget_failed = self.ledger.check().is_err()
                || result.as_ref().err().is_some_and(|e| e == BUDGET_ERROR);
            if !context.cancel.is_cancelled() {
                if budget_failed {
                    self.system_line(&id, BUDGET_LINE, include_bytes!("assets/budget.wav"), None);
                    let _ = self.finish(&id, VoiceEndReason::Budget);
                } else if result.is_err() {
                    failures += 1;
                    if failures >= MAX_FAILURES {
                        self.system_line(&id, ERROR_LINE, include_bytes!("assets/error.wav"), None);
                        let _ = self.finish(&id, VoiceEndReason::Error);
                    } else {
                        let speech = match &next {
                            Work::Utterance { speech, .. }
                            | Work::Text { speech, .. }
                            | Work::Live { speech, .. }
                            | Work::Delivery(_, speech)
                            | Work::Notice(_, speech) => Some(speech),
                        };
                        self.system_line(
                            &id,
                            RETRY_LINE,
                            include_bytes!("assets/retry.wav"),
                            speech,
                        );
                    }
                } else {
                    failures = 0;
                }
            }
            if let Work::Delivery(delivery, _) | Work::Notice(delivery, _) = &next
                && (result.is_err() || context.cancel.is_cancelled())
            {
                self.push(delivery, &delivery.text);
            }
            let _ = self.change(&id, |call| {
                if matches!(
                    next,
                    Work::Utterance { .. } | Work::Live { .. } | Work::Text { .. }
                ) {
                    call.input = None;
                    call.utterance_pending = false;
                    if call.target.is_none() {
                        call.first_clip_started = None;
                    }
                }
                if call.state != VoiceState::Held {
                    call.state(
                        if call.utterance_pending {
                            VoiceState::Thinking
                        } else {
                            VoiceState::Listening
                        },
                        None,
                    );
                }
                Ok(())
            });
        }
        while let Ok(work) = work.try_recv() {
            if let Work::Delivery(d, _) | Work::Notice(d, _) = work {
                self.push(&d, &d.text);
            }
        }
    }

    async fn hear(
        &self,
        id: &str,
        context: &Context,
        seq: u32,
        clip: Clip,
        duration: u32,
        speech_cancel: &CancellationToken,
    ) -> Result<(), String> {
        let services = self.speech_for(id)?;
        let stt = services
            .stt
            .as_ref()
            .ok_or("This call does not have a remote transcription provider.")?;
        let bytes = clip.bytes.len();
        let billable_ms = speech::billable_ms(&clip.mime, bytes, duration);
        self.pay(
            Kind::Stt,
            ledger::stt_usd(&stt.id().provider_id, billable_ms as f64 / 1000.0),
        )?;
        let text = tokio::select! {
            _ = context.cancel.cancelled() => return Ok(()),
            _ = speech_cancel.cancelled() => return Ok(()),
            text = stt.transcribe(clip) => text.map_err(|e| e.to_string())?,
        };
        if context.cancel.is_cancelled() {
            return Ok(());
        }
        self.respond(
            id,
            context,
            seq,
            text,
            TranscriptSource::Audio { duration, bytes },
            speech_cancel,
        )
        .await
    }

    async fn respond(
        &self,
        id: &str,
        context: &Context,
        seq: u32,
        text: String,
        source: TranscriptSource,
        speech_cancel: &CancellationToken,
    ) -> Result<(), String> {
        if context.cancel.is_cancelled() || speech_cancel.is_cancelled() {
            return Ok(());
        }
        let text = text.trim().chars().take(8_000).collect::<String>();
        self.emit(
            id,
            VoiceEvent::Heard {
                seq,
                text: text.clone(),
            },
        );
        if text.is_empty() {
            return Ok(());
        }
        let target = self.change(id, |call| Ok(call.target.clone()))?;
        if target.is_none() {
            self.record("user", &text)?;
        }
        if goodbye(&text) && source.permits_goodbye() {
            if self
                .say(id, "Goodbye.", speech_cancel, &context.cancel)
                .await
                .is_err()
            {
                self.system_line(
                    id,
                    "Goodbye.",
                    include_bytes!("assets/goodbye.wav"),
                    Some(speech_cancel),
                );
            }
            let _ = self.finish(id, VoiceEndReason::Goodbye);
            return Ok(());
        }
        self.ledger.check().map_err(|_| BUDGET_ERROR.to_string())?;
        let origin = Origin {
            call_id: id.into(),
            seq,
            direct: target.is_some(),
        };
        if let Some(target) = target {
            let room = self.room.upgrade().ok_or("The desk has closed.")?;
            // Room owns session startup/reuse, revocable grants, steering and tape.
            // Once accepted, this work is independent of the call's speech token.
            tokio::select! {
                biased;
                _ = context.cancel.cancelled() => return Ok(()),
                _ = speech_cancel.cancelled() => return Ok(()),
                started = tokio::time::timeout(Duration::from_secs(60), room.start(&target)) => {
                    started.map_err(|_| "The teammate did not start in time.".to_string())??;
                }
            }
            if context.cancel.is_cancelled() || speech_cancel.is_cancelled() {
                return Ok(());
            }
            let prompt = crate::wire::commands::VOICE_COMMAND.scope(
                (),
                crate::wire::commands::CALL_ORIGIN
                    .scope(origin, room.prompt(&target, &text, None, None)),
            );
            // Chapter/start gates can still wait before the instruction is
            // accepted. Cancellation stops that wait, not an accepted turn.
            return tokio::select! {
                biased;
                _ = context.cancel.cancelled() => Ok(()),
                _ = speech_cancel.cancelled() => Ok(()),
                accepted = tokio::time::timeout(Duration::from_secs(60), prompt) => {
                    accepted.map_err(|_| "The teammate did not accept the instruction in time.".to_string())?
                }
            };
        }
        let dispatcher = self.dispatcher()?;
        let context = context.with_origin(origin);
        let (output, mut answers) = mpsc::channel(8);
        let produce = async {
            tokio::time::timeout(
                Duration::from_secs(60),
                dispatcher.answer_stream(context.clone(), &text, self.ledger.clone(), output),
            )
            .await
            .map_err(|_| "The voice dispatcher timed out.".to_string())?
        };
        let consume = async {
            let mut failure = None;
            while let Some(sentence) = answers.recv().await {
                if failure.is_some() {
                    // Speech may fail; keep recording the rest of the answer as text.
                    let silent = CancellationToken::new();
                    silent.cancel();
                    let _ = self.say(id, &sentence, &silent, &context.cancel).await;
                } else if let Err(error) = self
                    .say(id, &sentence, speech_cancel, &context.cancel)
                    .await
                {
                    failure = Some(error);
                }
            }
            failure.map_or(Ok(()), Err)
        };
        tokio::select! {
            _ = context.cancel.cancelled() => Ok(()),
            result = async {
                let (produced, spoken) = tokio::join!(produce, consume);
                if [&produced, &spoken].iter().any(|result| result.as_ref().err().is_some_and(|e| e == BUDGET_ERROR)) {
                    Err(BUDGET_ERROR.to_string())
                } else {
                    produced.and(spoken)
                }
            } => result,
        }
    }

    async fn summary(&self, delivery: &Delivery) -> Result<String, String> {
        if speech_ready(&delivery.text) {
            return Ok(delivery.text.trim().to_string());
        }
        let fallback = || {
            sentences(&delivery.text)
                .into_iter()
                .next()
                .unwrap_or_default()
        };
        if delivery.text.len() > 32_000 {
            return Ok(fallback());
        }
        let Ok(dispatcher) = self.dispatcher() else {
            return Ok(fallback());
        };
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            dispatcher.narrate(&delivery.name, &delivery.text, self.ledger.clone()),
        )
        .await;
        match result {
            Ok(Err(error)) if error == BUDGET_ERROR => Err(error),
            Ok(Ok(text)) if !text.trim().is_empty() => Ok(sentences(&text).join(" ")),
            _ => Ok(fallback()),
        }
    }

    async fn narrate(
        &self,
        id: &str,
        context: &Context,
        delivery: &Delivery,
        speech: &CancellationToken,
    ) -> Result<(), String> {
        self.ledger.check().map_err(|_| BUDGET_ERROR.to_string())?;
        let text = tokio::select! {
            _ = context.cancel.cancelled() => return Ok(()),
            _ = speech.cancelled() => { self.push(delivery, &delivery.text); return Ok(()); },
            text = async {
                if self.change(id, |call| Ok(call.target.is_some()))? {
                    Ok(sentences(&delivery.text).into_iter().take(2).collect::<Vec<_>>().join(" "))
                } else { self.summary(delivery).await }
            } => text?,
        };
        if text.is_empty() {
            return Ok(());
        }
        // Hold/interrupt may win while the narrator is completing. Decide publication
        // under the same lock as those commands, just as for audio clips.
        let published = self.change(id, |call| {
            if call.state == VoiceState::Held || speech.is_cancelled() {
                return Ok(false);
            }
            let _ = call.events.send(VoiceEvent::Delivery {
                persona_id: delivery.persona.clone(),
                event_id: delivery.event.clone(),
                text: text.clone(),
            });
            Ok(true)
        })?;
        if !published {
            self.push(delivery, &delivery.text);
            return Ok(());
        }
        self.say(id, &text, speech, &context.cancel).await
    }

    fn record(&self, kind: &str, text: &str) -> Result<String, String> {
        self.room
            .upgrade()
            .ok_or("The desk has closed.")?
            .voice_record(kind, text)
    }

    fn pay(&self, kind: Kind, usd: f64) -> Result<(), String> {
        self.ledger
            .reserve(kind, usd)
            .map_err(|_| BUDGET_ERROR.to_string())
    }

    async fn synthesize(&self, speech: &CallSpeech, text: &str) -> Result<Clip, String> {
        self.pay(
            Kind::Tts,
            ledger::tts_usd(&speech.tts.id().provider_id, text.chars().count()),
        )?;
        let clip = match speech.tts.speak(text).await {
            Ok(clip) => Ok(clip),
            Err(error) => match &speech.fallback_tts {
                Some(_) if speech.tts.is_subscription() => Err(error.to_string()),
                Some(fallback) => {
                    if !self.connected(fallback.as_ref()) {
                        return Err("The fallback voice provider is no longer connected.".into());
                    }
                    self.pay(
                        Kind::Tts,
                        ledger::tts_usd(&fallback.id().provider_id, text.chars().count()),
                    )?;
                    fallback.speak(text).await.map_err(|e| e.to_string())
                }
                None => Err(error.to_string()),
            },
        }?;
        if !matches!(clip.mime.as_str(), "audio/wav" | "audio/mpeg")
            || clip.bytes.is_empty()
            || clip.bytes.len() > MAX_AUDIO
        {
            return Err("The speech provider returned an unusable sentence clip.".into());
        }
        Ok(clip)
    }

    async fn say(
        &self,
        id: &str,
        text: &str,
        interrupted: &CancellationToken,
        ended: &CancellationToken,
    ) -> Result<(), String> {
        let sentences = sentences(text);
        if sentences.is_empty() {
            return Err("The dispatcher returned no words.".into());
        }
        let text = sentences.join(" ");
        let line = if self.change(id, |call| Ok(call.target.is_some()))? {
            Uuid::new_v4().to_string()
        } else {
            self.record("agent", &text)?
        };
        self.emit(
            id,
            VoiceEvent::Said {
                id: line.clone(),
                text,
            },
        );
        let _ = self.change(id, |call| {
            if call.state != VoiceState::Held && !interrupted.is_cancelled() {
                call.state(VoiceState::Speaking, None);
            }
            Ok(())
        });
        let speech = self.speech_for(id)?;
        let streaming = self.change(id, |call| Ok(call.stream_audio))?;
        let mut chunk_index = 0;
        for (index, sentence) in sentences.iter().enumerate() {
            if interrupted.is_cancelled() || ended.is_cancelled() {
                return Ok(());
            }
            if streaming {
                tokio::select! {
                    _ = interrupted.cancelled() => return Ok(()),
                    _ = ended.cancelled() => return Ok(()),
                    result = self.synthesize_stream(id, &line, &speech, sentence, &mut chunk_index, index + 1 == sentences.len(), interrupted) => result?,
                }
                continue;
            }
            let clip = tokio::select! {
                _ = interrupted.cancelled() => return Ok(()),
                _ = ended.cancelled() => return Ok(()),
                clip = self.synthesize(&speech, sentence) => clip?,
            };
            // A hold/interrupt can arrive at the same time as the provider.
            if interrupted.is_cancelled() || ended.is_cancelled() {
                return Ok(());
            }
            self.clip(
                id,
                &line,
                index as u32,
                index + 1 == sentences.len(),
                &clip,
                Some(interrupted),
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn synthesize_stream(
        &self,
        id: &str,
        line: &str,
        services: &CallSpeech,
        text: &str,
        index: &mut u32,
        final_sentence: bool,
        interrupted: &CancellationToken,
    ) -> Result<(), String> {
        if interrupted.is_cancelled() {
            return Ok(());
        }
        self.pay(
            Kind::Tts,
            ledger::tts_usd(&services.tts.id().provider_id, text.chars().count()),
        )?;
        let (output, chunks) = mpsc::channel(4);
        let produce = services.tts.speak_chunks(text, output);
        let mut published = false;
        let consume = async {
            let mut chunks = chunks;
            let mut bytes = 0;
            let mut finished = false;
            while let Some(chunk) = chunks.recv().await {
                if finished {
                    return Err("The speech provider sent audio after its final chunk.".to_string());
                }
                let clip = chunk.clip;
                bytes += clip.bytes.len();
                if !matches!(clip.mime.as_str(), "audio/wav" | "audio/mpeg")
                    || clip.bytes.is_empty()
                    || bytes > MAX_AUDIO
                {
                    return Err("The speech provider returned unusable audio.".to_string());
                }
                self.clip(
                    id,
                    line,
                    *index,
                    final_sentence && chunk.final_chunk,
                    &clip,
                    Some(interrupted),
                );
                *index += 1;
                published = true;
                finished = chunk.final_chunk;
            }
            if finished {
                Ok(())
            } else {
                Err("The speech provider ended before its final audio chunk.".to_string())
            }
        };
        let attempted = match tokio::time::timeout(Duration::from_secs(30), async {
            tokio::join!(produce, consume)
        })
        .await
        {
            Ok((produced, consumed)) => consumed.and(produced.map_err(|e| e.to_string())),
            Err(_) => Err("The speech provider timed out.".to_string()),
        };
        if attempted.is_ok() || interrupted.is_cancelled() {
            return Ok(());
        }
        // A replacement after audible output would repeat words. Only an attempt
        // that produced no usable chunks may use the configured paid fallback.
        if !published
            && !services.tts.is_subscription()
            && let Some(fallback) = &services.fallback_tts
        {
            if interrupted.is_cancelled() {
                return Ok(());
            }
            if !self.connected(fallback.as_ref()) {
                return Err("The fallback voice provider is no longer connected.".into());
            }
            self.pay(
                Kind::Tts,
                ledger::tts_usd(&fallback.id().provider_id, text.chars().count()),
            )?;
            if interrupted.is_cancelled() {
                return Ok(());
            }
            let clip = tokio::time::timeout(Duration::from_secs(30), fallback.speak(text))
                .await
                .map_err(|_| "The fallback speech provider timed out.".to_string())?
                .map_err(|e| e.to_string())?;
            if !matches!(clip.mime.as_str(), "audio/wav" | "audio/mpeg")
                || clip.bytes.is_empty()
                || clip.bytes.len() > MAX_AUDIO
            {
                return Err("The speech provider returned unusable audio.".into());
            }
            self.clip(id, line, *index, final_sentence, &clip, Some(interrupted));
            *index += 1;
            return Ok(());
        }
        attempted
    }

    fn clip(
        &self,
        id: &str,
        line: &str,
        index: u32,
        last: bool,
        clip: &Clip,
        interrupted: Option<&CancellationToken>,
    ) {
        let _ = self.change(id, |call| {
            if call.state != VoiceState::Held
                && !interrupted.is_some_and(CancellationToken::is_cancelled)
            {
                let sent = call.events.send(VoiceEvent::Clip {
                    id: line.into(),
                    index,
                    r#final: last,
                    mime_type: clip.mime.clone(),
                    data: STANDARD.encode(&clip.bytes),
                });
                if sent.is_ok()
                    && let Some(started) = call.first_clip_started.take()
                {
                    eprintln!(
                        "[voice] utterance to first clip call={} seq={}: {}ms",
                        call.id,
                        call.seq.unwrap_or_default(),
                        started.elapsed().as_millis()
                    );
                }
            }
            Ok(())
        });
    }

    pub(crate) fn delivery(
        self: &Arc<Self>,
        persona: &str,
        event: &str,
        name: &str,
        text: &str,
        from_voice: bool,
        origin: Option<&Origin>,
    ) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        let delivery = Delivery {
            persona: persona.into(),
            event: event.into(),
            name: name.into(),
            text: text.to_string(),
        };
        let mut calls = lock(&self.calls);
        if let Some(call) = calls.iter_mut().rev().find(|call| {
            call.state != VoiceState::Ended
                && origin.is_none_or(|origin| origin.call_id == call.id)
                && call.target.as_ref().is_none_or(|target| {
                    target == persona
                        && origin
                            .is_some_and(|origin| origin.direct && call.seq == Some(origin.seq))
                })
        }) {
            if call.spoken_events.iter().any(|id| id == event) {
                return true;
            }
            if call.state == VoiceState::Held {
                return false;
            }
            let accepted = call
                .work
                .try_send(Work::Delivery(delivery, call.deliveries.child_token()))
                .is_ok();
            if accepted {
                if call.spoken_events.len() >= 128 {
                    call.spoken_events.pop_front();
                }
                call.spoken_events.push_back(event.into());
            }
            return accepted;
        }
        drop(calls);
        // Text-only desks retain their ordinary push and incur no voice cost.
        if origin.is_some_and(|origin| origin.direct)
            || !from_voice
            || self.resolve_speech(VoiceInputMode::Text).is_err()
            || self.dispatcher().is_err()
            || self.ledger.check().is_err()
        {
            return false;
        }
        let Ok(permit) = self.narration.clone().try_acquire_owned() else {
            return false;
        };
        let this = self.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let summary = this
                .summary(&delivery)
                .await
                .ok()
                .filter(|text| !text.is_empty());
            this.push(&delivery, summary.as_deref().unwrap_or(&delivery.text));
        });
        true
    }

    pub(crate) fn handoff_failed(&self, persona: &str, name: &str) {
        let text = format!(
            "The task for {name} could not be delivered. Please check their conversation before retrying."
        );
        let notice = Delivery {
            persona: persona.into(),
            name: name.into(),
            text,
            // System notices never emit a teammate delivery event.
            event: String::new(),
        };
        let calls = lock(&self.calls);
        if let Some(call) = calls.iter().rev().find(|c| {
            c.state != VoiceState::Ended && c.state != VoiceState::Held && c.target.is_none()
        }) && call
            .work
            .try_send(Work::Notice(notice.clone(), call.deliveries.child_token()))
            .is_ok()
        {
            return;
        }
        drop(calls);
        self.push(&notice, &notice.text);
    }

    fn push(&self, delivery: &Delivery, text: &str) {
        if let Some(room) = self.room.upgrade() {
            room.voice_push(&delivery.persona, &delivery.name, text);
        }
    }

    pub(crate) fn card(&self, persona: &str, event: &serde_json::Value) {
        let kind = event["kind"].as_str().unwrap_or_default();
        let request = match kind {
            "permission" if event["decision"].is_null() => event["requestId"].as_str(),
            "human_action" if event["status"] == "pending" => event["actionId"].as_str(),
            "passkey_ask" if event["status"] == "pending" => event["askId"].as_str(),
            _ => None,
        };
        if let Some(request) = request {
            for call in lock(&self.calls).iter().filter(|call| {
                call.state != VoiceState::Ended
                    && call.target.as_ref().is_none_or(|target| target == persona)
            }) {
                let _ = call.events.send(VoiceEvent::Card {
                    persona_id: persona.into(),
                    request_id: request.into(),
                    kind: kind.into(),
                });
            }
        }
    }
}

fn speech_ready(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text.chars().count() <= 320
        && sentences(text).len() <= 2
        && !text.contains(['`', '#', '*', '\n', '[', ']', '|'])
}

fn goodbye(text: &str) -> bool {
    static PUNCTUATION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"[\p{P}&&[^,-]]").expect("fixed punctuation regex")
    });
    static GOODBYE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?x)^
            (?:(?:ok(?:ay)?|alright|all\ right|thanks|thank\ you)[\ ,]+)*
            (?:goodbye|good\ bye|bye(?:[\ -]bye)?|see\ (?:you|ya)\ later
               |hang\ up|end\ (?:the\ )?call|thats\ all|that\ is\ all)
            (?:[\ ,]+(?:hotline|desk))?$",
        )
        .expect("fixed goodbye regex")
    });
    let lower = text.to_lowercase();
    let spoken = PUNCTUATION.replace_all(&lower, "");
    let spoken = spoken.split_whitespace().collect::<Vec<_>>().join(" ");
    GOODBYE.is_match(&spoken)
}

fn sentences(text: &str) -> Vec<String> {
    take_sentences(&mut text.to_string(), true)
}

/// A boundary needs following whitespace, so file names and versions stay intact.
fn take_sentences(pending: &mut String, finished: bool) -> Vec<String> {
    let mut ends = Vec::new();
    let mut chars = pending.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?')
            && chars.peek().is_some_and(|(_, next)| next.is_whitespace())
        {
            ends.push(at + c.len_utf8());
        }
    }
    if finished {
        ends.push(pending.len());
    }
    let mut start = 0;
    let mut out = Vec::new();
    for end in ends {
        let sentence = pending[start..end].trim();
        if !sentence.is_empty() {
            out.push(sentence.to_string());
        }
        start = end;
    }
    pending.drain(..start);
    out
}

fn validate_audio(mime: &str, bytes: &[u8], duration: u32) -> Result<u32, String> {
    if mime == "audio/mp4" {
        if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
            return Err("Send an AAC clip in an MP4 container.".into());
        }
        let actual = mp4_duration(bytes, 0)?.ok_or("The MP4 clip has no duration.")?;
        if actual == 0 || actual > 20_000 || (actual as i64 - duration as i64).abs() > 250 {
            return Err("Send at most 20 seconds of audio with its actual duration.".into());
        }
        return Ok(actual);
    }
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("Send a PCM WAV clip.".into());
    }
    let mut offset = 12;
    let mut format = false;
    let mut saw_format = false;
    let mut samples = None;
    while offset + 8 <= bytes.len() {
        let size = u32::from_le_bytes(
            bytes[offset + 4..offset + 8]
                .try_into()
                .expect("four bytes"),
        ) as usize;
        let end = (offset + 8)
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or("The WAV clip is truncated.")?;
        if &bytes[offset..offset + 4] == b"fmt " {
            if saw_format {
                return Err("The WAV clip repeats its format.".into());
            }
            saw_format = true;
            let f = &bytes[offset + 8..end];
            format = f.len() >= 16
                && f[..2] == 1u16.to_le_bytes()
                && f[2..4] == 1u16.to_le_bytes()
                && f[4..8] == 16_000u32.to_le_bytes()
                && f[14..16] == 16u16.to_le_bytes();
        }
        if &bytes[offset..offset + 4] == b"data" {
            if samples.is_some() {
                return Err("The WAV clip repeats its samples.".into());
            }
            samples = Some(size);
        }
        offset = end + (size % 2);
    }
    let size = samples.ok_or("The WAV clip has no samples.")?;
    if !format
        || size == 0
        || size > 640_000
        || size % 2 != 0
        || (size as i64 * 1000 / 32_000 - duration as i64).abs() > 250
    {
        return Err(
            "Send 16 kHz mono PCM16 WAV, at most 20 seconds, with its actual duration.".into(),
        );
    }
    Ok((size as u32).div_ceil(32))
}

fn mp4_duration(bytes: &[u8], depth: u8) -> Result<Option<u32>, String> {
    if depth > 5 {
        return Err("The MP4 clip is too deeply nested.".into());
    }
    let mut offset = 0usize;
    let mut duration = None;
    let malformed = || "The MP4 clip is malformed.".to_string();
    while offset < bytes.len() {
        let header = bytes.get(offset..offset + 8).ok_or_else(malformed)?;
        let mut size = u32::from_be_bytes(header[..4].try_into().expect("four bytes")) as usize;
        let mut header_size = 8;
        if size == 1 {
            size = u64::from_be_bytes(
                bytes
                    .get(offset + 8..offset + 16)
                    .ok_or_else(malformed)?
                    .try_into()
                    .expect("eight bytes"),
            )
            .try_into()
            .map_err(|_| malformed())?;
            header_size = 16;
        } else if size == 0 {
            size = bytes.len() - offset;
        }
        if size < header_size {
            return Err(malformed());
        }
        let end = offset
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(malformed)?;
        let body = &bytes[offset + header_size..end];
        let found = match &header[4..8] {
            b"moov" | b"trak" | b"mdia" => mp4_duration(body, depth + 1)?,
            b"mdhd" => {
                let version = *body.first().ok_or_else(malformed)?;
                let (scale_at, duration_at, width) = match version {
                    0 => (12, 16, 4),
                    1 => (20, 24, 8),
                    _ => return Err(malformed()),
                };
                let scale = u32::from_be_bytes(
                    body.get(scale_at..scale_at + 4)
                        .ok_or_else(malformed)?
                        .try_into()
                        .expect("four bytes"),
                ) as u64;
                let raw = body
                    .get(duration_at..duration_at + width)
                    .ok_or_else(malformed)?;
                let count = if width == 4 {
                    u32::from_be_bytes(raw.try_into().expect("four bytes")) as u64
                } else {
                    u64::from_be_bytes(raw.try_into().expect("eight bytes"))
                };
                if scale == 0 {
                    return Err(malformed());
                }
                Some(
                    count
                        .checked_mul(1000)
                        .ok_or_else(malformed)?
                        .div_ceil(scale)
                        .try_into()
                        .map_err(|_| malformed())?,
                )
            }
            _ => None,
        };
        if let Some(found) = found {
            duration = Some(duration.map_or(found, |old: u32| old.max(found)));
        }
        offset = end;
    }
    Ok(duration)
}

#[cfg(test)]
pub(crate) mod tests;
