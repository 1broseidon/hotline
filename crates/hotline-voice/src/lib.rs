//! Hotline's calls and their speech and realtime engines.
//!
//! The call runtime owns input, turns, fronts, synthesis and dispatching.
//! [`call_room::CallRoom`] is its only path back to sessions, real tool
//! handlers and the vault. The core implements that seam, including the
//! temporary operations that later voice slices remove. This crate depends
//! downward on the contract, support, store, room, tools and MCP client;
//! it must never depend on `hotline-core` or Tauri.

mod call;
pub mod call_room;
pub mod dispatcher;
mod duplex;
mod front;
mod front_prompt;
pub mod front_tools;
mod input;
pub mod realtime;
mod record;
mod replies;
pub mod speech;
mod synthesis;
mod turn;

pub use hotline_room::voice::{ledger, metering, settings, spoken};

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
pub use call::CallRequest;
use call_room::CallRoom;
use dispatcher::{Dispatcher, ProviderDispatcher};
use duplex::Front;
use hotline_contract::{
    BudgetKind, SpeechModel, VoiceBudget, VoiceEndReason, VoiceEvent, VoiceFront, VoiceInputMode,
    VoiceModel, VoiceState, VoiceStatus,
};
use hotline_store::log::Log;
use ledger::Kind;
use metering::{BUDGET_ERROR, Budget};
use record::Record;
use settings::VoiceSettings;
use speech::local::{self, Installs};
use speech::{Clip, Speech, SpeechId, SpeechOutput, SpeechSet};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use synthesis::{begin_answer, calling};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const RETRY_LINE: &str = "Sorry, say that again.";
const ERROR_LINE: &str = "Voice keeps failing. Please continue by text.";
const MAX_FAILURES: u8 = 3;
const MAX_AUDIO: usize = 2 * 1024 * 1024;
const FRONT_LINE: &str = "This call has a voice front. Send front audio.";
const BUDGET_LINE: &str = "The voice budget is unavailable or spent. Chat carries on by text.";
/// The capability of a desk whose call to a teammate takes what the person
/// says while the teammate's turn works: its `thinking` says `listening`, and
/// the words steer into the open turn.
pub const LISTEN_WHILE_THINKING: &str = "voiceListenWhileThinking";
/// A desk with a realtime provider can put a voice front on a call.
pub const VOICE_FRONT: &str = "voiceFront";
/// How often a call that is thinking says so again. A phone stops waiting on
/// a desk it has not heard from for 45 seconds, and a teammate's turn can work
/// for longer than that without a word. Tests wait a tenth of a second.
const THINKING_AGAIN: Duration = if cfg!(any(test, feature = "test-support")) {
    Duration::from_millis(100)
} else {
    Duration::from_secs(15)
};

/// How long a call waits for the call assistant to rewrite a teammate's reply
/// that wrote no spoken version before it says the reply's opening instead.
/// Tests wait less.
const REWRITE_WAIT: Duration = if cfg!(any(test, feature = "test-support")) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(6)
};

/// Provider seams for an embedded desk or a scripted client. Production desks
/// resolve both from the vault on each call and before each utterance. A
/// desk with no call assistant has `dispatcher` `None`.
#[derive(Clone)]
pub struct Services {
    pub speech: SpeechSet,
    pub dispatcher: Option<Arc<dyn Dispatcher>>,
    /// The realtime model a front call speaks through; none, no front.
    pub realtime: Option<Arc<dyn realtime::Realtime>>,
}

/// Text input carries no remote listener, even when an audio listener is configured.
#[derive(Clone)]
pub struct CallSpeech {
    pub stt: Option<Arc<dyn speech::Speech>>,
    tts: Arc<dyn speech::Speech>,
    fallback_tts: Option<Arc<dyn speech::Speech>>,
}

/// A front's identity as a call's speech: the
/// provider and model the login check and the budgets read, and nothing to
/// hear or say with, since the front does both itself.
struct FrontSpeech {
    id: SpeechId,
    subscription: bool,
}

#[async_trait]
impl Speech for FrontSpeech {
    fn id(&self) -> SpeechId {
        self.id.clone()
    }
    fn is_subscription(&self) -> bool {
        self.subscription
    }
    fn output_mime(&self) -> &'static str {
        "audio/pcm"
    }
    async fn transcribe(&self, _clip: Clip) -> Result<String, speech::SpeechError> {
        Err(speech::SpeechError::WrongJob)
    }
    async fn speak(&self, _text: &str) -> Result<Clip, speech::SpeechError> {
        Err(speech::SpeechError::WrongJob)
    }
}

/// Why a call is refused, and what `voice.status` says, while the owner has
/// Live calls Off.
pub const CALLS_OFF: &str = "Calls are off in Settings";

/**
 * The kinds of paid work a call can do, each spent against its own budget:
 * hearing and speaking against Voice when their provider charges, and the
 * call assistant against Chat when its model is billed per token. A call
 * heard, spoken and answered for free (a subscription, the desk's own
 * engine, a signed-in model) names none, so no budget can stop it. A call to
 * a teammate names no call assistant: its turns are the teammate's own, which
 * its session meters as any other, and the call assistant's rewrite of a
 * reply written only to be read is asked of the Chat budget when it is
 * wanted, and falls back to the reply's opening if refused.
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
    pub fn audio(speech: SpeechSet) -> Self {
        Self {
            stt: Some(speech.stt),
            tts: speech.tts,
            fallback_tts: speech.fallback_tts,
        }
    }
    pub fn output(speech: SpeechOutput) -> Self {
        Self {
            stt: None,
            tts: speech.tts,
            fallback_tts: speech.fallback_tts,
        }
    }
    /// A front call needs no Legacy speech services: the front
    /// hears and speaks itself, and its identity stands in for the speech
    /// the call would otherwise have resolved.
    fn front(front: &dyn realtime::Realtime) -> Self {
        Self {
            stt: None,
            tts: Arc::new(FrontSpeech {
                id: front.id(),
                subscription: front.is_subscription(),
            }),
            fallback_tts: None,
        }
    }
}

#[derive(Clone, Copy)]
enum TranscriptSource {
    Audio {
        duration: u32,
        bytes: usize,
    },
    Device,
    /// What a voice front relayed. Whether to hang up is the front's call, so
    /// a relayed "goodbye" is words for the teammate like any others.
    Front,
}

impl TranscriptSource {
    fn permits_goodbye(&self) -> bool {
        match self {
            Self::Audio { duration, bytes } => speech::plausible_goodbye(*duration, *bytes),
            Self::Device => true,
            Self::Front => false,
        }
    }
}

/// Internal provenance; never accepted from a session command's parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub call_id: String,
    pub seq: u32,
    pub direct: bool,
    /// Spoken results go through the live front instead of speech synthesis.
    pub front: bool,
}

impl Origin {
    /// The thread and turn a direct call's line came from.
    pub fn from(&self) -> Option<hotline_contract::DeliveryFrom> {
        self.direct.then(|| {
            hotline_contract::DeliveryFrom::new(
                &hotline_contract::ThreadId::call(&self.call_id),
                Some(self.seq.to_string()),
            )
        })
    }

    /// The call a line came from, when the line says so as a delivery does:
    /// the call thread, and the turn of it. Only a direct call is a thread, so
    /// a call said in this way is one.
    pub fn from_delivery(from: &hotline_contract::DeliveryFrom) -> Option<Self> {
        if from.kind != hotline_contract::ThreadKind::Call {
            return None;
        }
        Some(Self {
            call_id: from.thread.clone(),
            seq: from.request.as_deref()?.parse().ok()?,
            direct: true,
            front: false,
        })
    }

    /// Where the line a call's turn was written under came from. A desk call
    /// is no thread, so its lines are still known by the id they were written
    /// under; so are the lines of a direct call written before it was one.
    pub fn from_event_id(id: &str) -> Option<Self> {
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
            front: false,
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

#[doc(hidden)]
pub struct Call {
    desk: bool,
    thread: Option<hotline_contract::ThreadId>,
    id: String,
    output: String,
    pub target: Option<String>,
    stream_audio: bool,
    input_mode: VoiceInputMode,
    pub speech_services: CallSpeech,
    input: Option<Input>,
    spoken_events: VecDeque<String>,
    connection_bound: bool,
    pub state: VoiceState,
    reason: Option<VoiceEndReason>,
    pub seq: Option<u32>,
    activity: Instant,
    events: broadcast::Sender<VoiceEvent>,
    work: mpsc::Sender<Work>,
    pub cancel: CancellationToken,
    pub speech: CancellationToken,
    pub deliveries: CancellationToken,
    pub utterance_pending: bool,
    pub first_clip_started: Option<Instant>,
    /// On a direct call, the latest turn handed to the teammate whose session
    /// has not finished it: the call thinks until it has, unless the person
    /// takes the floor.
    pub answering: Option<u32>,
    /// On a direct call, the teammate's replies being said as they stream,
    /// by event id, until each is whole.
    pub streams: HashMap<String, mpsc::UnboundedSender<String>>,
    /// Replies the call did not say to the end, cut off by a hold or with no
    /// room in its queue, which the phone is told of instead.
    cut: VecDeque<String>,
    /// A direct call's thread, which keeps all of what was said. A desk call
    /// is kept on the desk tape.
    record: Option<Record>,
    speech_line: record::SpeechLine,
    interrupted_through: Option<u32>,
    /// What the person last said on a direct call, which a rewrite of the
    /// teammate's reply is handed for context.
    pub words: String,
    /// On a front call, its sound; gone once the front is.
    front: Option<Front>,
    live_error: Option<String>,
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

    /// Whether the call thinks only because the teammate's turn is still
    /// working, with the person's last words handed to it: what they say now
    /// is taken, and steers into that turn.
    fn listening_while_thinking(&self) -> bool {
        self.state == VoiceState::Thinking
            && self.target.is_some()
            && self.answering.is_some()
            && !self.utterance_pending
            && self.input.is_none()
    }

    fn snapshot(&self) -> VoiceEvent {
        VoiceEvent::State {
            state: self.state,
            live_error: self.live_error.clone(),
            reason: self.reason,
            listening: self.listening_while_thinking(),
        }
    }
    pub fn state(&mut self, state: VoiceState, reason: Option<VoiceEndReason>) {
        if state == VoiceState::Ended {
            self.keep_front_speech();
        }
        let ended = self.state == VoiceState::Ended;
        self.state = state;
        self.reason = reason;
        let _ = self.events.send(self.snapshot());
        if state == VoiceState::Ended {
            if !ended && let Some(record) = &self.record {
                record.ended(reason);
            }
            self.input = None;
            self.front = None;
            self.cancel.cancel();
            self.speech.cancel();
            self.deliveries.cancel();
        }
    }
}

#[derive(Clone)]
pub struct Delivery {
    pub persona: String,
    pub event: String,
    pub name: String,
    pub text: String,
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
        source: TranscriptSource,
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
    room: Arc<dyn CallRoom>,
    injected: Option<Services>,
    ledger: Arc<Budget>,
    calls: Mutex<VecDeque<Call>>,
    narration: Arc<tokio::sync::Semaphore>,
    /// The desk's own speech models and their downloads.
    installs: Arc<Installs>,
    /// How teammates' replies on calls to them were said, by model.
    replies: replies::Replies,
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
    fn live_room(&self) -> Option<Arc<dyn CallRoom>> {
        self.room.upgrade()
    }
    pub fn new(log: Log, room: Arc<dyn CallRoom>, injected: Option<Services>) -> Arc<Self> {
        let budget = Arc::new(Budget::open(log.clone()));
        Self::with_models(log, room, injected, local::catalogue(), budget)
    }

    /// With the speech models offered for download named, for a harness that serves its own.
    pub fn with_models(
        log: Log,
        room: Arc<dyn CallRoom>,
        injected: Option<Services>,
        models: Vec<local::Model>,
        ledger: Arc<Budget>,
    ) -> Arc<Self> {
        Arc::new(Self {
            ledger,
            installs: Installs::new(room.speech_root(), models),
            replies: replies::Replies::open(log.root()),
            log,
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
        let settings = self.settings();
        let picked = settings
            .stt
            .as_ref()
            .filter(|pick| pick.provider_id == local::PROVIDER_ID)
            .and_then(|pick| pick.model_id.as_deref());
        let model = local::chosen(self.room.speech_root(), picked)
            .or_else(|| local::chosen(self.room.speech_root(), None))
            .ok_or("Download a speech model in Settings › Providers for the desk to hear you.")?;
        let words = speech::listens_for(&self.room.connected(), &settings);
        local::Local::new(model)
            .listening_for(&words)
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
    pub fn quiet(&self) -> Vec<(String, i64)> {
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

    /// Whether the call `id` is still going: the person is on it.
    /// Only a live call placed to the desk carries the lead's routing grant.
    pub fn lead_call(&self, persona: &str, thread: &hotline_contract::ThreadId) -> bool {
        hotline_room::room::team_lead(&self.log).as_deref() == Some(persona)
            && lock(&self.calls).iter().any(|call| {
                call.desk
                    && call.target.as_deref() == Some(persona)
                    && call.thread.as_ref() == Some(thread)
                    && call.state != VoiceState::Ended
            })
    }

    pub fn thread_origin(&self, thread: &hotline_contract::ThreadId) -> Option<Origin> {
        let calls = lock(&self.calls);
        let call = calls
            .iter()
            .rev()
            .find(|call| call.thread.as_ref() == Some(thread) && call.state != VoiceState::Ended)?;
        Some(Origin {
            call_id: call.id.clone(),
            seq: call.seq.unwrap_or_default(),
            direct: true,
            front: call.front.is_some(),
        })
    }

    pub fn thread_is_up(&self, thread: &hotline_contract::ThreadId) -> bool {
        lock(&self.calls)
            .iter()
            .any(|call| call.thread.as_ref() == Some(thread) && call.state != VoiceState::Ended)
    }

    pub fn is_up(&self, id: &str) -> bool {
        lock(&self.calls)
            .iter()
            .any(|call| call.id == id && call.state != VoiceState::Ended)
    }

    /// Ends a call nobody has spoken on for long enough.
    pub fn end_quiet(&self, id: &str) {
        let _ = self.finish(id, VoiceEndReason::Idle);
    }

    fn settings(&self) -> VoiceSettings {
        VoiceSettings::from_log(&self.log)
    }
    fn dispatcher(&self) -> Result<Arc<dyn Dispatcher>, String> {
        if let Some(services) = &self.injected {
            return services
                .dispatcher
                .clone()
                .ok_or_else(|| "This desk has no call assistant.".to_string());
        }
        ProviderDispatcher::resolve(self.room.clone(), &self.log)
    }

    /// The desk's voice and call-assistant budget.
    pub fn budget(&self) -> Arc<Budget> {
        self.ledger.clone()
    }

    /// The call assistant a call pays for from the start: a desk call's
    /// routes what was said. A call to a teammate pays for none, since the
    /// teammate answers itself ([`paid_kinds`]).
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
        let mut status = self.pipeline_status(input_mode);
        if self.settings().calls_off {
            status.capabilities.clear();
            status.available = false;
            status.direct_available = false;
            status.unavailable = Some(CALLS_OFF.into());
            status.front = None;
            status.live_selected = false;
        }
        if status.live_selected && !self.settings().calls_off {
            let error = self.required_front().err();
            status.direct_available = error.is_none();
            status.available = error.is_none();
            status.unavailable = error;
        }
        status
    }

    fn pipeline_status(&self, input_mode: VoiceInputMode) -> VoiceStatus {
        let speech = self.resolve_speech(input_mode, None);
        // A front takes a call the pipeline could not: it hears and speaks itself.
        let front = self
            .realtime()
            .is_some_and(|front| self.front_ready(front.as_ref()).is_ok());
        let dispatcher = self.dispatcher();
        let budget_error = |assistant: Option<&dyn Dispatcher>| {
            self.ledger
                .ready(&paid_kinds(speech.as_ref().ok(), assistant))
                .err()
                .map(|e| e.to_string())
        };
        // A call to a teammate pays for no call assistant.
        let direct_budget_error = budget_error(None);
        let lead = hotline_room::room::team_lead(&self.log).is_some();
        let budget_error = budget_error(if lead {
            None
        } else {
            dispatcher.as_deref().ok()
        });
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
        let direct_available = (speech.is_ok() || front) && direct_budget_error.is_none();
        let unavailable = speech
            .as_ref()
            .err()
            .filter(|_| !front)
            .cloned()
            .or_else(|| {
                if lead {
                    None
                } else {
                    dispatcher.as_ref().err().cloned()
                }
            })
            .or(budget_error);
        let mut capabilities = vec![
            "voiceDirectCalls".into(),
            "voiceTextInput".into(),
            LISTEN_WHILE_THINKING.into(),
        ];
        if self.front_available() {
            capabilities.push(VOICE_FRONT.into());
        }
        VoiceStatus {
            live_selected: self.front_selected(),
            capabilities,
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
            replies: Some(self.replies.counts()).filter(|counts| !counts.is_empty()),
            front: self.front_model(),
        }
    }

    pub fn delivery(
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
            if call
                .interrupted_through
                .is_some_and(|seq| origin.seq <= seq)
            {
                return true;
            }
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
    /// it: its spoken version said as it comes, from the reply's first words
    /// ([`spoken::Spoken`]). The reply's whole text follows as
    /// [`Self::delivery`], which ends the stream.
    pub fn reply_delta(&self, persona: &str, event: &str, chunk: &str, origin: &Origin) {
        if !origin.direct {
            return;
        }
        let mut calls = lock(&self.calls);
        let Some(call) = calling(&mut calls, persona, origin) else {
            return;
        };
        if call
            .interrupted_through
            .is_some_and(|seq| origin.seq <= seq)
        {
            return;
        }
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

    /// The call to `persona` going on now, as the origin of a turn its
    /// session takes by itself: a reply it makes is said on the call, after
    /// the last line the person said there. None before the person has said
    /// anything, when there is nothing for the reply to follow.
    pub fn on_call(&self, persona: &str) -> Option<Origin> {
        let calls = lock(&self.calls);
        let call = calls.iter().rev().find(|call| {
            call.state != VoiceState::Ended
                && call.front.is_some()
                && call.target.as_deref() == Some(persona)
        })?;
        Some(Origin {
            call_id: call.id.clone(),
            seq: call.seq?,
            direct: true,
            front: call.front.is_some(),
        })
    }

    /// The teammate's session finished a turn said on a call to it, or left
    /// it open only for its subagents: once what it said is said, the call
    /// listens.
    pub fn turn_ended(&self, persona: &str, origin: &Origin) {
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

    pub fn handoff_failed(&self, persona: &str, name: &str) {
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
        if let Some(room) = self.live_room() {
            room.voice_push(&delivery.persona, &delivery.name, text);
        }
    }

    pub fn card(&self, persona: &str, event: &serde_json::Value) {
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

#[cfg(feature = "test-support")]
pub mod test_support;
