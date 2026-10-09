//! One owner call per desk, with bounded work and complete sentence clips.

pub mod dispatcher;
pub mod ledger;
pub mod metering;
mod record;
pub mod settings;
pub mod speech;
pub mod spoken;

use crate::contract::{
    BudgetKind, SpeechModel, VoiceBudget, VoiceCall, VoiceEndReason, VoiceEvent, VoiceInputMode,
    VoiceModel, VoiceState, VoiceStatus,
};
use crate::{log::Log, session::Room, vault::Vault, wire::RoomHandle};
use base64::{Engine, engine::general_purpose::STANDARD};
use dispatcher::{Context, Dispatcher, ProviderDispatcher};
use ledger::Kind;
use metering::{BUDGET_ERROR, Budget};
use record::{Record, Speaker};
use settings::VoiceSettings;
use speech::local::{self, Installs};
use speech::{Clip, Speech, SpeechId, SpeechOutput, SpeechSet};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// The desk's own calls, which name no teammate, are kept on this tape and not
/// as call threads: they have no DM to hang off, and the dispatcher reads the
/// tape's tail as what was said on the desk lately, across calls. It is
/// indexed, but has no persona and never enters the roster.
pub const TAPE_ID: &str = "voice-dispatcher";
const RETRY_LINE: &str = "Sorry, say that again.";
const ERROR_LINE: &str = "Voice keeps failing. Please continue by text.";
const MAX_FAILURES: u8 = 3;
const MAX_AUDIO: usize = 2 * 1024 * 1024;
const BUDGET_LINE: &str = "The voice budget is unavailable or spent. Chat carries on by text.";
/// How often a call that is thinking says so again. A phone stops waiting on
/// a desk it has not heard from for 45 seconds, and a teammate's turn can work
/// for longer than that without a word. Tests wait a tenth of a second.
const THINKING_AGAIN: Duration = if cfg!(test) {
    Duration::from_millis(100)
} else {
    Duration::from_secs(15)
};

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

/**
 * The kinds of paid work a call can do, each spent against its own budget:
 * hearing and speaking against Voice when their provider charges, and the
 * call assistant against Chat when its model is billed per token. A call
 * heard, spoken and answered for free (a subscription, the desk's own
 * engine, a signed-in model) names none, so no budget can stop it. A call to
 * a teammate has no call assistant: its turns are the teammate's own, which
 * its session meters as any other.
 */
fn paid_kinds(speech: Option<&CallSpeech>, assistant: Option<&dyn Dispatcher>) -> Vec<Kind> {
    let mut kinds = Vec::new();
    if let Some(speech) = speech {
        if speech
            .stt
            .as_ref()
            .is_some_and(|stt| !ledger::is_free(&stt.id().provider_id))
        {
            kinds.push(Kind::Stt);
        }
        if !ledger::is_free(&speech.tts.id().provider_id) {
            kinds.push(Kind::Tts);
        }
    }
    if assistant.is_some_and(|assistant| !assistant.free()) {
        kinds.push(Kind::Dispatcher);
    }
    kinds
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
    /// The thread and turn a direct call's line came from.
    pub(crate) fn from(&self) -> Option<crate::contract::DeliveryFrom> {
        self.direct.then(|| {
            crate::contract::DeliveryFrom::new(
                &crate::thread::ThreadId::call(&self.call_id),
                Some(self.seq.to_string()),
            )
        })
    }

    pub(crate) fn event_id(&self, event: &str) -> String {
        format!(
            "voice:{}:{}:{}:{event}",
            self.call_id,
            self.seq,
            if self.direct { "agent" } else { "desk" }
        )
    }
    /// The call a line came from, when the line says so as a delivery does:
    /// the call thread, and the turn of it. Only a direct call is a thread, so
    /// a call said in this way is one.
    pub(crate) fn from_delivery(from: &crate::contract::DeliveryFrom) -> Option<Self> {
        if from.kind != crate::thread::ThreadKind::Call {
            return None;
        }
        Some(Self {
            call_id: from.thread.clone(),
            seq: from.request.as_deref()?.parse().ok()?,
            direct: true,
        })
    }

    /// Where the line a call's turn was written under came from. A desk call
    /// is no thread, so its lines are still known by the id they were written
    /// under; so are the lines of a direct call written before it was one.
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
    /// On a direct call, the latest turn handed to the teammate whose session
    /// has not finished it: the call thinks until it has, unless the person
    /// takes the floor.
    answering: Option<u32>,
    /// On a direct call, the teammate's replies being said as they stream,
    /// by event id, until each is whole.
    streams: HashMap<String, mpsc::UnboundedSender<String>>,
    /// Replies the call did not say to the end, cut off by a hold or with no
    /// room in its queue, which the phone is told of instead.
    cut: VecDeque<String>,
    /// A direct call's thread, which keeps all of what was said. A desk call
    /// is kept on the desk tape.
    record: Option<Record>,
}

impl Call {
    /// Where the call rests between things to say: thinking while an
    /// utterance or the teammate's turn is still being worked on, else
    /// listening.
    fn resting(&self) -> VoiceState {
        if self.utterance_pending || self.answering.is_some() {
            VoiceState::Thinking
        } else {
            VoiceState::Listening
        }
    }

    /// Stops saying the replies being streamed. A hold cuts them off, and
    /// what they would have said reaches the phone as a held call's replies do.
    fn stop_streams(&mut self, held: bool) {
        for (event, _) in std::mem::take(&mut self.streams) {
            if held {
                self.cut_off(event);
            }
        }
    }

    fn cut_off(&mut self, event: String) {
        if self.cut.len() >= 16 {
            self.cut.pop_front();
        }
        self.cut.push_back(event);
    }

    fn snapshot(&self) -> VoiceEvent {
        VoiceEvent::State {
            state: self.state,
            reason: self.reason,
        }
    }
    fn state(&mut self, state: VoiceState, reason: Option<VoiceEndReason>) {
        let ended = self.state == VoiceState::Ended;
        self.state = state;
        self.reason = reason;
        let _ = self.events.send(self.snapshot());
        if state == VoiceState::Ended {
            if !ended && let Some(record) = &self.record {
                record.ended(reason);
            }
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

/// A reply that arrives sentence by sentence: the one line it is shown and
/// kept as, and its words so far.
#[derive(Default)]
struct Reply {
    line: String,
    text: String,
}

impl Reply {
    /// Adds a sentence, and says the line and the reply so far.
    fn add(&mut self, sentence: &str) -> (String, String) {
        if self.line.is_empty() {
            self.line = Uuid::new_v4().to_string();
        } else {
            self.text.push(' ');
        }
        self.text.push_str(sentence);
        (self.line.clone(), self.text.clone())
    }
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
    /// A teammate's reply on its own call, said as it streams in.
    Answer {
        event: String,
        chunks: Mutex<Option<mpsc::UnboundedReceiver<String>>>,
        speech: CancellationToken,
    },
    /// The teammate's session finished the turn of this utterance.
    TurnOver(u32),
}

pub struct Calls {
    log: Log,
    vault: Arc<Vault>,
    room: Weak<Room>,
    injected: Option<Services>,
    ledger: Arc<Budget>,
    calls: Mutex<VecDeque<Call>>,
    narration: Arc<tokio::sync::Semaphore>,
    /// The desk's own speech models and their downloads.
    installs: Arc<Installs>,
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
    #[cfg(test)]
    pub(crate) fn new(
        log: Log,
        vault: Arc<Vault>,
        room: Weak<Room>,
        injected: Option<Services>,
    ) -> Arc<Self> {
        let budget = Arc::new(Budget::open(log.clone()));
        Self::with_models(log, vault, room, injected, local::catalogue(), budget)
    }

    /// With the speech models offered for download named, for a harness that serves its own.
    pub(crate) fn with_models(
        log: Log,
        vault: Arc<Vault>,
        room: Weak<Room>,
        injected: Option<Services>,
        models: Vec<local::Model>,
        ledger: Arc<Budget>,
    ) -> Arc<Self> {
        Arc::new(Self {
            ledger,
            installs: Installs::new(vault.root(), models),
            log,
            vault,
            room,
            injected,
            calls: Mutex::new(VecDeque::new()),
            narration: Arc::new(tokio::sync::Semaphore::new(1)),
        })
    }

    /// The desk's own speech models and where each stands.
    pub fn speech_models(&self) -> Vec<SpeechModel> {
        self.installs.status()
    }

    pub fn install_speech_model(&self, id: &str) -> Result<Vec<SpeechModel>, String> {
        self.installs.install(id)?;
        Ok(self.installs.status())
    }

    pub fn cancel_speech_model(&self, id: &str) -> Vec<SpeechModel> {
        self.installs.cancel(id);
        self.installs.status()
    }

    pub fn remove_speech_model(&self, id: &str) -> Result<Vec<SpeechModel>, String> {
        self.installs.remove(id)?;
        Ok(self.installs.status())
    }

    /// One clip heard by the desk's own model, outside any call: the model
    /// picked for hearing when it is one of the desk's, else the first
    /// installed. Free, so no budget is asked.
    pub async fn transcribe(&self, mime: &str, data: &str) -> Result<String, String> {
        // Two minutes of 16 kHz PCM16 and its WAV header, as base64.
        const MAX_BYTES: usize = local::MAX_SECONDS * 32_000 + 44;
        if data.len() > MAX_BYTES.div_ceil(3) * 4 {
            return Err(format!(
                "Send at most {} minutes of audio at a time.",
                local::MAX_SECONDS / 60
            ));
        }
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| "Audio must be standard base64.".to_string())?;
        let picked = self
            .settings()
            .stt
            .filter(|pick| pick.provider_id == local::PROVIDER_ID)
            .and_then(|pick| pick.model_id);
        let model = local::chosen(self.vault.root(), picked.as_deref())
            .or_else(|| local::chosen(self.vault.root(), None))
            .ok_or("Download a speech model in Settings › Providers for the desk to hear you.")?;
        local::Local::new(model)
            .transcribe(Clip {
                mime: mime.to_string(),
                bytes,
            })
            .await
            .map_err(|error| error.to_string())
    }

    /// How long each call still going has been quiet, in milliseconds: the
    /// room's sweep reads this, and ends a call that is quiet for long enough
    /// (`Policy::of(ThreadKind::Call).idle`).
    pub(crate) fn quiet(&self) -> Vec<(String, i64)> {
        lock(&self.calls)
            .iter()
            .filter(|call| call.state != VoiceState::Ended)
            .map(|call| {
                (
                    call.id.clone(),
                    i64::try_from(call.activity.elapsed().as_millis()).unwrap_or(i64::MAX),
                )
            })
            .collect()
    }

    /// Ends a call nobody has spoken on for long enough.
    pub(crate) fn end_quiet(&self, id: &str) {
        let _ = self.finish(id, VoiceEndReason::Idle);
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

    /// The desk's voice and call-assistant budget.
    pub(crate) fn budget(&self) -> Arc<Budget> {
        self.ledger.clone()
    }

    /// The call assistant a call uses: a desk call's routes what was said; a
    /// call to a teammate has none, since the teammate answers itself.
    fn assistant_for(&self, target: Option<&str>) -> Option<Arc<dyn Dispatcher>> {
        if target.is_some() {
            return None;
        }
        self.dispatcher().ok()
    }

    /// Whether the call's paid work may go on: each budget it spends against
    /// has something left.
    fn ready_for(&self, id: &str) -> Result<(), String> {
        let speech = self.speech_for(id)?;
        let target = self.change(id, |call| Ok(call.target.clone()))?;
        let assistant = self.assistant_for(target.as_deref());
        self.ledger
            .ready(&paid_kinds(Some(&speech), assistant.as_deref()))
            .map_err(|_| BUDGET_ERROR.to_string())
    }

    pub fn status(&self) -> VoiceStatus {
        self.status_for(VoiceInputMode::Audio)
    }

    pub fn status_for(&self, input_mode: VoiceInputMode) -> VoiceStatus {
        let speech = self.resolve_speech(input_mode, None);
        let dispatcher = self.dispatcher();
        let budget_error = |assistant: Option<&dyn Dispatcher>| {
            self.ledger
                .ready(&paid_kinds(speech.as_ref().ok(), assistant))
                .err()
                .map(|e| e.to_string())
        };
        // A call to a teammate pays for no call assistant.
        let direct_budget_error = budget_error(None);
        let budget_error = budget_error(dispatcher.as_deref().ok());
        let limits = self.ledger.limits(BudgetKind::Voice);
        let spent = self.ledger.spent();
        // A tally that cannot be read reports its limits spent.
        let (spent_day_usd, spent_month_usd) = match spent {
            Ok(spent) => (
                spent.day.budget(BudgetKind::Voice),
                spent.month.budget(BudgetKind::Voice),
            ),
            Err(_) => (
                limits.day_usd.unwrap_or(0.0),
                limits.month_usd.unwrap_or(0.0),
            ),
        };
        let direct_available = speech.is_ok() && direct_budget_error.is_none();
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
                day_usd: limits.day_usd,
                month_usd: limits.month_usd,
                spent_day_usd,
                spent_month_usd,
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
            let speech_services = self.resolve_speech(input_mode, target.as_deref())?;
            if target.is_none() {
                self.dispatcher()?;
            }
            // Only the budgets this call would pay into can refuse it.
            let assistant = self.assistant_for(target.as_deref());
            self.ledger
                .ready(&paid_kinds(Some(&speech_services), assistant.as_deref()))
                .map_err(|e| e.to_string())?;
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
                answering: None,
                streams: HashMap::new(),
                cut: VecDeque::new(),
                record: target
                    .as_deref()
                    .map(|persona_id| Record::open(self.room.clone(), id, persona_id)),
            });
            self.thinking_again(id, cancel.clone());
            let this = self.clone();
            let id = id.to_string();
            let context = Context::new(self.log.clone(), room, cancel.clone());
            // What the person says on this call carries the app they called from.
            let client = crate::wire::commands::prompt_client();
            tokio::spawn(async move {
                let run = this.run(id, context, rx);
                match client {
                    Some(client) => {
                        crate::wire::commands::PROMPT_CLIENT
                            .scope(client, run)
                            .await
                    }
                    None => run.await,
                }
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

    /// While the call thinks, it says so again every [`THINKING_AGAIN`], so a
    /// teammate working for minutes is not taken for a desk that went quiet.
    fn thinking_again(self: &Arc<Self>, id: &str, cancel: CancellationToken) {
        let this = Arc::downgrade(self);
        let id = id.to_string();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(THINKING_AGAIN);
            every.tick().await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = every.tick() => {}
                }
                let Some(this) = this.upgrade() else { return };
                let _ = this.change(&id, |call| {
                    if call.state == VoiceState::Thinking {
                        let _ = call.events.send(call.snapshot());
                    }
                    Ok(())
                });
            }
        });
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
                call.stop_streams(true);
                call.state(VoiceState::Held, None);
            } else if call.state == VoiceState::Held {
                call.state(call.resting(), None);
            }
            Ok(())
        })
    }

    /// Stops what the call is saying and gives the person the floor. A
    /// teammate's turn goes on, and what it says next is still said; what the
    /// person says now steers into that turn.
    pub fn interrupt(&self, id: &str) -> Result<(), String> {
        self.change(id, |call| {
            call.activity = Instant::now();
            call.input = None;
            call.speech.cancel();
            call.deliveries.cancel();
            call.deliveries = CancellationToken::new();
            call.stop_streams(false);
            call.answering = None;
            if call.state != VoiceState::Held {
                call.state(call.resting(), None);
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
            call.speech_services = self.resolve_speech(input_mode, call.target.as_deref())?;
            call.work
                .try_send(work(speech.clone()))
                .map_err(|_| "The voice call is busy. Try again shortly.".to_string())?;
            call.input = None;
            call.speech.cancel();
            call.speech = speech;
            call.deliveries.cancel();
            call.deliveries = CancellationToken::new();
            call.stop_streams(false);
            call.utterance_pending = true;
            call.first_clip_started = Some(Instant::now());
            call.seq = Some(seq);
            call.activity = Instant::now();
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
    }

    /// A direct call to `target` speaks in that teammate's own voice when it has one.
    fn resolve_speech(
        &self,
        input_mode: VoiceInputMode,
        target: Option<&str>,
    ) -> Result<CallSpeech, String> {
        if let Some(services) = &self.injected {
            return Ok(match input_mode {
                VoiceInputMode::Audio => CallSpeech::audio(services.speech.clone()),
                VoiceInputMode::Text => CallSpeech::output(SpeechOutput {
                    tts: services.speech.tts.clone(),
                    fallback_tts: services.speech.fallback_tts.clone(),
                }),
            });
        }
        let resolve = |settings: &VoiceSettings| match input_mode {
            VoiceInputMode::Audio => speech::resolve(&self.vault, settings).map(CallSpeech::audio),
            VoiceInputMode::Text => {
                speech::resolve_output(&self.vault, settings).map(CallSpeech::output)
            }
        };
        let settings = self.settings();
        let desk = resolve(&settings)?;
        let own = target
            .and_then(|target| {
                crate::room::roster(&self.log)
                    .into_iter()
                    .find(|persona| persona.id == target)
            })
            .and_then(|persona| persona.voice)
            .and_then(|voice| own_voice(&settings, &voice, &desk.tts.id()));
        // A voice the provider no longer takes leaves the desk's in place.
        Ok(own
            .and_then(|settings| resolve(&settings).ok())
            .unwrap_or(desk))
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
        if speech.id().provider_id == local::PROVIDER_ID {
            return local::chosen(self.vault.root(), Some(&speech.id().model_id)).is_some();
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
                let services =
                    self.resolve_speech(VoiceInputMode::Audio, call.target.as_deref())?;
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
                call.stop_streams(false);
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
        let direct = self
            .change(id, |call| Ok(call.target.is_some()))
            .unwrap_or(false);
        let line = if direct {
            self.keep(id, Speaker::Voice, text)
        } else {
            self.record("agent", text)
                .unwrap_or_else(|_| Uuid::new_v4().to_string())
        };
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
                Work::Answer {
                    event,
                    chunks,
                    speech,
                } => {
                    let chunks = lock(chunks).take().expect("one answer consumer");
                    self.answer(&id, event, chunks, speech, &context.cancel)
                        .await
                }
                Work::TurnOver(seq) => {
                    let _ = self.change(&id, |call| {
                        if call.answering.is_some_and(|answering| answering <= *seq) {
                            call.answering = None;
                        }
                        Ok(())
                    });
                    Ok(())
                }
            };
            // A refused reservation is what ends a call on its budget.
            let budget_failed = result.as_ref().err().is_some_and(|e| e == BUDGET_ERROR);
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
                            | Work::Answer { speech, .. }
                            | Work::Delivery(_, speech)
                            | Work::Notice(_, speech) => Some(speech),
                            Work::TurnOver(_) => None,
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
                    call.state(call.resting(), None);
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
        } else {
            self.keep(id, Speaker::Person, &text);
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
        self.ready_for(id)?;
        let origin = Origin {
            call_id: id.into(),
            seq,
            direct: target.is_some(),
        };
        if let Some(target) = target {
            return self
                .hand_off(id, &target, &text, origin, context, speech_cancel)
                .await;
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
        let reply = Mutex::new(Reply::default());
        let consume = self.say_reply(id, &mut answers, &reply, speech_cancel, &context.cancel);
        tokio::select! {
            _ = context.cancel.cancelled() => Ok(()),
            result = async {
                let (produced, spoken) = tokio::join!(produce, consume);
                let kept = self.keep_reply(id, std::mem::take(&mut *lock(&reply)));
                if [&produced, &spoken].iter().any(|result| result.as_ref().err().is_some_and(|e| e == BUDGET_ERROR)) {
                    Err(BUDGET_ERROR.to_string())
                } else {
                    produced.and(spoken).and(kept)
                }
            } => result,
        }
    }

    /// The person's words into the teammate's own session, in the open
    /// chapter of its conversation, as a turn said on this call: what a direct
    /// call does with every utterance. The session hears the contract after
    /// the words (see [`spoken`]); the conversation shows the words alone.
    /// Into a turn still running, the words steer it. The call thinks until
    /// the session has finished the turn ([`Self::turn_ended`]).
    async fn hand_off(
        &self,
        id: &str,
        target: &str,
        text: &str,
        origin: Origin,
        context: &Context,
        speech_cancel: &CancellationToken,
    ) -> Result<(), String> {
        let room = self.room.upgrade().ok_or("The desk has closed.")?;
        let seq = origin.seq;
        // Room owns session startup/reuse, revocable grants, steering and tape.
        // Once accepted, this work is independent of the call's speech token.
        tokio::select! {
            biased;
            _ = context.cancel.cancelled() => return Ok(()),
            _ = speech_cancel.cancelled() => return Ok(()),
            started = tokio::time::timeout(Duration::from_secs(60), room.start(target)) => {
                started.map_err(|_| "The teammate did not start in time.".to_string())??;
            }
        }
        if context.cancel.is_cancelled() || speech_cancel.is_cancelled() {
            return Ok(());
        }
        let prompt = crate::wire::commands::VOICE_COMMAND.scope(
            (),
            crate::wire::commands::CALL_ORIGIN.scope(origin, room.prompt(target, text, None, None)),
        );
        // Chapter/start gates can still wait before the instruction is
        // accepted. Cancellation stops that wait, not an accepted turn.
        tokio::select! {
            biased;
            _ = context.cancel.cancelled() => return Ok(()),
            _ = speech_cancel.cancelled() => return Ok(()),
            accepted = tokio::time::timeout(Duration::from_secs(60), prompt) => {
                accepted.map_err(|_| "The teammate did not accept the instruction in time.".to_string())??;
            }
        }
        self.change(id, |call| {
            call.answering = Some(seq);
            Ok(())
        })
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
        self.ready_for(id)?;
        let text = tokio::select! {
            _ = context.cancel.cancelled() => return Ok(()),
            _ = speech.cancelled() => { self.push(delivery, &delivery.text); return Ok(()); },
            text = self.summary(delivery) => text?,
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

    /// A teammate's reply on its own call, said as it streams in: its spoken
    /// part ([`spoken::Spoken`]) sentence by sentence as one line, kept on
    /// the call's thread once it is over. Speaking over it stops it at once,
    /// so the call is free for what the person says; the chat has the whole
    /// reply in any case.
    async fn answer(
        &self,
        id: &str,
        event: &str,
        mut chunks: mpsc::UnboundedReceiver<String>,
        speech: &CancellationToken,
        ended: &CancellationToken,
    ) -> Result<(), String> {
        if speech.is_cancelled() {
            return Ok(());
        }
        let (sentences, mut said) = mpsc::channel(8);
        let split = async move {
            let mut spoken = spoken::Spoken::default();
            while let Some(chunk) = chunks.recv().await {
                for sentence in spoken.push(&chunk) {
                    if sentences.send(sentence).await.is_err() {
                        return;
                    }
                }
            }
            for sentence in spoken.finish() {
                if sentences.send(sentence).await.is_err() {
                    return;
                }
            }
        };
        let reply = Mutex::new(Reply::default());
        let say = self.say_reply(id, &mut said, &reply, speech, ended);
        let result = tokio::select! {
            _ = speech.cancelled() => Ok(()),
            _ = ended.cancelled() => Ok(()),
            (_, said) = async { tokio::join!(split, say) } => said,
        };
        let _ = self.change(id, |call| {
            call.streams.remove(event);
            Ok(())
        });
        let kept = self.keep_reply(id, std::mem::take(&mut *lock(&reply)));
        result.and(kept)
    }

    /// A line said on a direct call, kept on the call's thread. The line's id
    /// is what the clients know it by; the write is made behind the call.
    fn keep(&self, id: &str, speaker: Speaker, text: &str) -> String {
        let line = Uuid::new_v4().to_string();
        self.keep_as(id, speaker, &line, text);
        line
    }

    fn keep_as(&self, id: &str, speaker: Speaker, line: &str, text: &str) {
        let _ = self.change(id, |call| {
            if let Some(record) = &call.record {
                record.said(speaker, line, text);
            }
            Ok(())
        });
    }

    /// A reply said sentence by sentence, kept once and whole: on a direct
    /// call as one line of the call's thread under the id the clients were
    /// shown, and on a desk call as one line of the dispatcher's tape.
    fn keep_reply(&self, id: &str, reply: Reply) -> Result<(), String> {
        if reply.text.is_empty() {
            return Ok(());
        }
        if self.change(id, |call| Ok(call.target.is_some()))? {
            self.keep_as(id, Speaker::Voice, &reply.line, &reply.text);
        } else {
            self.record("agent", &reply.text)?;
        }
        Ok(())
    }

    fn record(&self, kind: &str, text: &str) -> Result<String, String> {
        self.room
            .upgrade()
            .ok_or("The desk has closed.")?
            .voice_record(kind, text)
    }

    /// Speech is priced from what is sent (seconds of audio, characters of
    /// text) and no provider reports usage back, so the reservation is the
    /// charge and nothing is settled afterwards.
    fn pay(&self, kind: Kind, usd: f64) -> Result<(), String> {
        self.ledger
            .reserve(kind, usd)
            .map(drop)
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

    /// Speaks a line at once, kept as the call's own on a direct call's
    /// thread or the dispatcher's tape.
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
            self.keep(id, Speaker::Voice, &text)
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
        self.speaking(id, interrupted);
        let speech = self.speech_for(id)?;
        let streaming = self.change(id, |call| Ok(call.stream_audio))?;
        let mut index = 0;
        for (at, sentence) in sentences.iter().enumerate() {
            if interrupted.is_cancelled() || ended.is_cancelled() {
                return Ok(());
            }
            let last = at + 1 == sentences.len();
            self.speak_sentence(
                id,
                &line,
                &speech,
                streaming,
                sentence,
                &mut index,
                last,
                interrupted,
                ended,
            )
            .await?;
        }
        Ok(())
    }

    /// Says a reply as the dispatcher writes it, sentence by sentence, as one
    /// line: each sentence is spoken as soon as it arrives, the clients are
    /// shown the line growing under one id, and its audio carries on under
    /// that id until an empty final clip closes it. The words are gathered in
    /// `reply` for [`Self::keep_reply`], which keeps them once, whole, even
    /// when the person speaks over the reply. Speech that fails stops the
    /// speaking, not the words.
    async fn say_reply(
        &self,
        id: &str,
        answers: &mut mpsc::Receiver<String>,
        reply: &Mutex<Reply>,
        interrupted: &CancellationToken,
        ended: &CancellationToken,
    ) -> Result<(), String> {
        let mut failure = None;
        let mut speech = None;
        let mut index = 0;
        while let Some(sentence) = answers.recv().await {
            let sentence = sentence.trim();
            if sentence.is_empty() {
                continue;
            }
            let (line, text) = lock(reply).add(sentence);
            self.emit(
                id,
                VoiceEvent::Said {
                    id: line.clone(),
                    text,
                },
            );
            if failure.is_some() || interrupted.is_cancelled() || ended.is_cancelled() {
                continue;
            }
            if speech.is_none() {
                self.speaking(id, interrupted);
                let services = self.speech_for(id).and_then(|services| {
                    Ok((services, self.change(id, |call| Ok(call.stream_audio))?))
                });
                match services {
                    Ok(services) => speech = Some(services),
                    Err(error) => {
                        failure = Some(error);
                        continue;
                    }
                }
            }
            let (services, streaming) = speech.as_ref().expect("resolved above");
            if let Err(error) = self
                .speak_sentence(
                    id,
                    &line,
                    services,
                    *streaming,
                    sentence,
                    &mut index,
                    false,
                    interrupted,
                    ended,
                )
                .await
            {
                failure = Some(error);
            }
        }
        if failure.is_none() && index > 0 && !interrupted.is_cancelled() && !ended.is_cancelled() {
            let line = lock(reply).line.clone();
            self.clip(
                id,
                &line,
                index,
                true,
                &Clip {
                    mime: "audio/wav".into(),
                    bytes: Vec::new(),
                },
                Some(interrupted),
            );
        }
        failure.map_or(Ok(()), Err)
    }

    /// The call is speaking, unless it is held or was just spoken over.
    fn speaking(&self, id: &str, interrupted: &CancellationToken) {
        let _ = self.change(id, |call| {
            if call.state != VoiceState::Held && !interrupted.is_cancelled() {
                call.state(VoiceState::Speaking, None);
            }
            Ok(())
        });
    }

    /// One sentence of `line`, sent as its next clip or clips from `index`.
    #[allow(clippy::too_many_arguments)]
    async fn speak_sentence(
        &self,
        id: &str,
        line: &str,
        speech: &CallSpeech,
        streaming: bool,
        sentence: &str,
        index: &mut u32,
        last: bool,
        interrupted: &CancellationToken,
        ended: &CancellationToken,
    ) -> Result<(), String> {
        let sentence = &speakable(sentence);
        if streaming {
            return tokio::select! {
                _ = interrupted.cancelled() => Ok(()),
                _ = ended.cancelled() => Ok(()),
                result = self.synthesize_stream(id, line, speech, sentence, index, last, interrupted) => result,
            };
        }
        let clip = tokio::select! {
            _ = interrupted.cancelled() => return Ok(()),
            _ = ended.cancelled() => return Ok(()),
            clip = self.synthesize(speech, sentence) => clip?,
        };
        // A hold/interrupt can arrive at the same time as the provider.
        if interrupted.is_cancelled() || ended.is_cancelled() {
            return Ok(());
        }
        self.clip(id, line, *index, last, &clip, Some(interrupted));
        *index += 1;
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
        if let Some(origin) = origin.filter(|origin| origin.direct) {
            let mut calls = lock(&self.calls);
            let Some(call) = calling(&mut calls, persona, origin) else {
                return false;
            };
            // A reply said as it streamed is whole now.
            if call.streams.remove(event).is_some() {
                return true;
            }
            if call.cut.iter().any(|id| id == event) {
                return false;
            }
            if call.spoken_events.iter().any(|id| id == event) {
                return true;
            }
            if call.state == VoiceState::Held {
                return false;
            }
            return begin_answer(call, event).is_some_and(|said| said.send(text.into()).is_ok());
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
                && call.target.is_none()
                && origin.is_none_or(|origin| origin.call_id == call.id)
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
        if !from_voice
            || self.resolve_speech(VoiceInputMode::Text, None).is_err()
            || self.dispatcher().map_or(true, |assistant| {
                self.ledger
                    .ready(&paid_kinds(None, Some(assistant.as_ref())))
                    .is_err()
            })
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

    /// Words of a teammate's reply as it streams, on the turn of a call to
    /// it: said as they come, from the reply's first words. The reply's whole
    /// text follows as [`Self::delivery`], which ends the stream.
    pub(crate) fn reply_delta(&self, persona: &str, event: &str, chunk: &str, origin: &Origin) {
        if !origin.direct {
            return;
        }
        let mut calls = lock(&self.calls);
        let Some(call) = calling(&mut calls, persona, origin) else {
            return;
        };
        if let Some(stream) = call.streams.get(event) {
            let _ = stream.send(chunk.into());
            return;
        }
        // A reply already said, or cut off, is not begun again halfway.
        if call.state == VoiceState::Held
            || call.spoken_events.iter().any(|id| id == event)
            || call.cut.iter().any(|id| id == event)
        {
            return;
        }
        if let Some(stream) = begin_answer(call, event)
            && stream.send(chunk.into()).is_ok()
        {
            call.streams.insert(event.into(), stream);
        }
    }

    /// The teammate's session finished a turn said on a call to it, or left
    /// it open only for its subagents: once what it said is said, the call
    /// listens.
    pub(crate) fn turn_ended(&self, persona: &str, origin: &Origin) {
        if !origin.direct {
            return;
        }
        let mut calls = lock(&self.calls);
        let Some(call) = calling(&mut calls, persona, origin) else {
            return;
        };
        // The turn will say nothing more, even of a reply it never finished.
        call.streams.clear();
        if call.work.try_send(Work::TurnOver(origin.seq)).is_err()
            && call
                .answering
                .is_some_and(|answering| answering <= origin.seq)
        {
            call.answering = None;
            if call.state == VoiceState::Thinking {
                call.state(call.resting(), None);
            }
        }
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

/// The call to `persona` a line of its session belongs to: one still going
/// on which the line's turn, or a later one, was said.
fn calling<'a>(
    calls: &'a mut VecDeque<Call>,
    persona: &str,
    origin: &Origin,
) -> Option<&'a mut Call> {
    calls.iter_mut().rev().find(|call| {
        origin.direct
            && call.id == origin.call_id
            && call.state != VoiceState::Ended
            && call.target.as_deref() == Some(persona)
            && call.seq.is_some_and(|seq| origin.seq <= seq)
    })
}

/// Queues a teammate's reply to be said on its call, and hands back where its
/// words go. None when the call has too much queued to take it: the reply is
/// then left to the phone, and never begun later from its middle.
fn begin_answer(call: &mut Call, event: &str) -> Option<mpsc::UnboundedSender<String>> {
    let (words, chunks) = mpsc::unbounded_channel();
    let queued = call.work.try_send(Work::Answer {
        event: event.into(),
        chunks: Mutex::new(Some(chunks)),
        speech: call.deliveries.child_token(),
    });
    if queued.is_err() {
        call.cut_off(event.into());
        return None;
    }
    if call.spoken_events.len() >= 128 {
        call.spoken_events.pop_front();
    }
    call.spoken_events.push_back(event.into());
    Some(words)
}

/// Whether a teammate's reply on a desk call can be said as written.
fn speech_ready(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text.chars().count() <= 320
        && sentences(text).len() <= 2
        && !text.contains(['`', '#', '*', '\n', '[', ']', '|'])
        && speakable(text) == text
}

/// What is sent to speech: a link is said as its site, never spelled out
/// character by character. The line shown on screen keeps the full link.
fn speakable(text: &str) -> String {
    static LINK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)\b(?:https?://|www\.)[^\s<>()]+").expect("fixed link regex")
    });
    LINK.replace_all(text, |link: &regex::Captures| {
        let whole = &link[0];
        // Punctuation that ends the sentence is not part of the link.
        let link = whole.trim_end_matches(['.', ',', ';', ':', '!', '?']);
        let rest = link.split_once("://").map_or(link, |(_, rest)| rest);
        let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        let host = host.strip_prefix("www.").unwrap_or(host);
        format!("{host}{}", &whole[link.len()..])
    })
    .into_owned()
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

/// The desk's settings with a teammate's own voice, when the desk still
/// speaks with the provider and model that voice was picked from.
fn own_voice(
    settings: &VoiceSettings,
    voice: &crate::contract::PersonaVoice,
    speaking: &speech::SpeechId,
) -> Option<VoiceSettings> {
    (speaking.provider_id == voice.provider_id
        && speaking.model_id == voice.model_id
        && speaking.voice.as_deref() != Some(voice.voice.as_str()))
    .then(|| VoiceSettings {
        tts: Some(settings::Choice {
            provider_id: voice.provider_id.clone(),
            model_id: Some(voice.model_id.clone()),
            voice: Some(voice.voice.clone()),
            effort: None,
        }),
        ..settings.clone()
    })
}

fn sentences(text: &str) -> Vec<String> {
    take_sentences(&mut text.to_string(), true)
}

/// A boundary needs following whitespace, so file names and versions stay
/// intact. A line break is a boundary too: list items rarely end in a stop.
fn take_sentences(pending: &mut String, finished: bool) -> Vec<String> {
    let mut ends = Vec::new();
    let mut chars = pending.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if c == '\n'
            || matches!(c, '.' | '!' | '?')
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
            out.extend(spoken_lengths(sentence));
        }
        start = end;
    }
    pending.drain(..start);
    out
}

/// The longest piece sent to speech at once. A run-on sentence is cut at a
/// comma-like pause, else a space, so no one request nears the audio limit.
const SPOKEN_PIECE: usize = 300;

fn spoken_lengths(sentence: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = sentence;
    while rest.len() > SPOKEN_PIECE {
        let mut limit = SPOKEN_PIECE;
        while !rest.is_char_boundary(limit) {
            limit -= 1;
        }
        let head = &rest[..limit];
        let cut = [", ", "; ", ": ", " - "]
            .iter()
            .filter_map(|pause| head.rfind(pause).map(|at| at + 1))
            .max()
            .filter(|&at| at > SPOKEN_PIECE / 3)
            .or_else(|| head.rfind(' ').filter(|&at| at > 0))
            .unwrap_or(limit);
        let piece = rest[..cut].trim();
        if !piece.is_empty() {
            out.push(piece.to_string());
        }
        rest = rest[cut..].trim_start();
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
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
