//! One owner call per desk, with bounded work and complete sentence clips.

pub mod dispatcher;
pub mod ledger;
pub mod metering;
pub mod settings;
pub mod speech;

use crate::contract::{
    VoiceBudget, VoiceCall, VoiceEndReason, VoiceEvent, VoiceModel, VoiceState, VoiceStatus,
};
use crate::{log::Log, session::Room, vault::Vault, wire::RoomHandle};
use base64::{Engine, engine::general_purpose::STANDARD};
use dispatcher::{Context, Dispatcher, ProviderDispatcher};
use ledger::Kind;
use metering::{BUDGET_ERROR, Budget};
use settings::VoiceSettings;
use speech::{Clip, SpeechId, SpeechSet};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// This tape is indexed, but has no persona and never enters the roster.
pub const TAPE_ID: &str = "voice-dispatcher";
const IDLE: Duration = Duration::from_secs(600);
const MAX_AUDIO: usize = 2 * 1024 * 1024;
const BUDGET_LINE: &str = "The voice budget is unavailable or spent. Chat carries on by text.";

/// Provider seams for an embedded desk or a scripted client. Production desks
/// resolve both from the vault on each call and before each utterance.
#[derive(Clone)]
pub struct Services {
    pub speech: SpeechSet,
    pub dispatcher: Arc<dyn Dispatcher>,
}

struct Call {
    id: String,
    output: String,
    connection_bound: bool,
    state: VoiceState,
    reason: Option<VoiceEndReason>,
    seq: Option<u32>,
    activity: Instant,
    events: broadcast::Sender<VoiceEvent>,
    work: mpsc::Sender<Work>,
    cancel: CancellationToken,
    speech: CancellationToken,
    wake: Arc<Notify>,
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
        self.wake.notify_one();
        if state == VoiceState::Ended {
            self.cancel.cancel();
            self.speech.cancel();
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
    Utterance {
        seq: u32,
        clip: Clip,
        duration: u32,
        speech: CancellationToken,
    },
    Delivery(Delivery),
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
        VoiceSettings::from_room(&crate::room::settings(&self.log))
    }
    fn services(&self) -> Result<Services, String> {
        if let Some(services) = &self.injected {
            return Ok(services.clone());
        }
        Ok(Services {
            speech: speech::resolve(&self.vault, &self.settings())?,
            dispatcher: ProviderDispatcher::resolve(self.vault.clone(), &self.log)?,
        })
    }

    pub fn status(&self) -> VoiceStatus {
        let budget = self.ledger.balance();
        let services = self.services();
        let unavailable = services
            .as_ref()
            .err()
            .cloned()
            .or_else(|| self.ledger.check().err().map(|e| e.to_string()));
        VoiceStatus {
            available: unavailable.is_none(),
            unavailable,
            stt: services.as_ref().ok().map(|s| model(s.speech.stt.id())),
            tts: services.as_ref().ok().map(|s| model(s.speech.tts.id())),
            fallback_tts: services
                .as_ref()
                .ok()
                .and_then(|s| s.speech.fallback_tts.as_ref().map(|s| model(s.id()))),
            dispatcher: services.as_ref().ok().map(|s| s.dispatcher.id()),
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
        Uuid::parse_str(id).map_err(|_| "A voice call needs a UUID.".to_string())?;
        let mut calls = lock(&self.calls);
        if !calls.iter().any(|call| call.id == id) {
            let services = self.services()?;
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
                output: services.speech.tts.output_mime().into(),
                connection_bound: false,
                state: VoiceState::Listening,
                reason: None,
                seq: None,
                activity: Instant::now(),
                events: broadcast::channel(128).0,
                work: tx,
                cancel: cancel.clone(),
                speech: CancellationToken::new(),
                wake: Arc::new(Notify::new()),
            });
            let this = self.clone();
            let id = id.to_string();
            let context = Context {
                log: self.log.clone(),
                room,
                cancel: cancel.clone(),
            };
            tokio::spawn(async move {
                this.run(id, context, rx).await;
            });
        }
        Ok(VoiceCall {
            call_id: id.into(),
            input: vec!["audio/wav".into(), "audio/mp4".into()],
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
                call.speech.cancel();
                call.state(VoiceState::Held, None);
            } else if call.state == VoiceState::Held {
                call.state(VoiceState::Listening, None);
            }
            Ok(())
        })
    }

    pub fn interrupt(&self, id: &str) -> Result<(), String> {
        self.change(id, |call| {
            call.activity = Instant::now();
            call.speech.cancel();
            if call.state != VoiceState::Held {
                call.state(VoiceState::Listening, None);
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
        self.change(id, |call| {
            if call.state == VoiceState::Held {
                return Err("Resume the call before speaking.".into());
            }
            if call.seq.is_some_and(|previous| seq <= previous) {
                return Err("Utterance sequence numbers must rise.".into());
            }
            if call.state == VoiceState::Thinking {
                return Err("The previous utterance is still being handled.".into());
            }
            let speech = CancellationToken::new();
            call.work
                .try_send(Work::Utterance {
                    seq,
                    clip: Clip {
                        mime: mime.into(),
                        bytes,
                    },
                    duration,
                    speech: speech.clone(),
                })
                .map_err(|_| "The voice call is busy. Try again shortly.".to_string())?;
            call.speech.cancel();
            call.speech = speech;
            call.seq = Some(seq);
            call.activity = Instant::now();
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
    }

    fn emit(&self, id: &str, event: VoiceEvent) {
        let _ = self.change(id, |call| {
            let _ = call.events.send(event);
            Ok(())
        });
    }

    async fn ready(&self, id: &str, cancel: &CancellationToken) -> Result<(), String> {
        loop {
            let wake = self.change(id, |call| {
                Ok((call.state == VoiceState::Held).then(|| call.wake.clone()))
            })?;
            let Some(wake) = wake else {
                return Ok(());
            };
            tokio::select! { _ = cancel.cancelled() => return Err("The call ended.".into()), _ = wake.notified() => {} }
        }
    }

    async fn run(self: Arc<Self>, id: String, context: Context, mut work: mpsc::Receiver<Work>) {
        // A bundled spoken system line remains playable when neither a ledger
        // nor a provider can authorize another paid request.
        let closing = Clip {
            mime: "audio/wav".into(),
            bytes: include_bytes!("assets/budget.wav").to_vec(),
        };
        loop {
            let next = tokio::select! { biased; _ = context.cancel.cancelled() => None, work = work.recv() => work };
            let Some(next) = next else {
                break;
            };
            if self.ready(&id, &context.cancel).await.is_err() {
                if let Work::Delivery(d) = next {
                    self.push(&d, &d.text);
                }
                break;
            }
            let result = match &next {
                Work::Utterance {
                    seq,
                    clip,
                    duration,
                    speech,
                } => {
                    self.hear(&id, &context, *seq, clip.clone(), *duration, speech)
                        .await
                }
                Work::Delivery(delivery) => self.narrate(&id, &context, delivery).await,
            };
            let budget_failed = self.ledger.check().is_err()
                || result
                    .as_ref()
                    .err()
                    .is_some_and(|error| error == BUDGET_ERROR);
            if (result.is_err() || budget_failed) && !context.cancel.is_cancelled() {
                if budget_failed {
                    let line = Uuid::new_v4().to_string();
                    self.emit(
                        &id,
                        VoiceEvent::Said {
                            id: line.clone(),
                            text: BUDGET_LINE.into(),
                        },
                    );
                    self.clip(&id, &line, 0, true, &closing, None);
                    let _ = self.finish(&id, VoiceEndReason::Budget);
                } else {
                    self.emit(
                        &id,
                        VoiceEvent::Said {
                            id: Uuid::new_v4().to_string(),
                            text: "Voice couldn't finish that request. Please continue in text."
                                .into(),
                        },
                    );
                    let _ = self.finish(&id, VoiceEndReason::Error);
                }
            }
            if let Work::Delivery(delivery) = next
                && (result.is_err() || context.cancel.is_cancelled())
            {
                self.push(&delivery, &delivery.text);
            }
            let _ = self.change(&id, |call| {
                if call.state != VoiceState::Held && call.work.capacity() == 16 {
                    call.state(VoiceState::Listening, None);
                }
                Ok(())
            });
        }
        while let Ok(work) = work.try_recv() {
            if let Work::Delivery(d) = work {
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
        let services = self.services()?;
        self.pay(
            Kind::Stt,
            ledger::stt_usd(
                &services.speech.stt.id().provider_id,
                duration as f64 / 1000.0,
            ),
        )?;
        let text = tokio::select! {
            _ = context.cancel.cancelled() => return Ok(()),
            text = services.speech.stt.transcribe(clip) => text.map_err(|e| e.to_string())?,
        };
        if context.cancel.is_cancelled() {
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
        self.record("user", &text)?;
        if goodbye(&text) {
            self.say(id, "Goodbye.", speech_cancel, &context.cancel)
                .await?;
            self.finish(id, VoiceEndReason::Goodbye)?;
            return Ok(());
        }
        self.ledger.check().map_err(|e| e.to_string())?;
        let answer = tokio::select! {
            _ = context.cancel.cancelled() => return Ok(()),
            answer = tokio::time::timeout(Duration::from_secs(60), services.dispatcher.answer(context.clone(), &text, self.ledger.clone())) => answer.map_err(|_| "The voice dispatcher timed out.".to_string())??,
        };
        self.ready(id, &context.cancel).await?;
        if !speech_cancel.is_cancelled() {
            self.say(id, &answer, speech_cancel, &context.cancel)
                .await?;
        }
        Ok(())
    }

    async fn narrate(
        &self,
        id: &str,
        context: &Context,
        delivery: &Delivery,
    ) -> Result<(), String> {
        let services = self.services()?;
        self.ledger.check().map_err(|e| e.to_string())?;
        let text = tokio::select! {
            _ = context.cancel.cancelled() => return Err("The call ended.".into()),
            text = tokio::time::timeout(Duration::from_secs(30), services.dispatcher.narrate(&delivery.name, &delivery.text, self.ledger.clone())) => text.map_err(|_| "Voice narration timed out.".to_string())??,
        };
        let text = sentences(&text).join(" ");
        self.ready(id, &context.cancel).await?;
        self.emit(
            id,
            VoiceEvent::Delivery {
                persona_id: delivery.persona.clone(),
                event_id: delivery.event.clone(),
                text: text.clone(),
            },
        );
        let speech = self.change(id, |call| {
            call.speech = CancellationToken::new();
            Ok(call.speech.clone())
        })?;
        self.say(id, &text, &speech, &context.cancel).await
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

    async fn synthesize(&self, speech: &SpeechSet, text: &str) -> Result<Clip, String> {
        self.pay(
            Kind::Tts,
            ledger::tts_usd(&speech.tts.id().provider_id, text.chars().count()),
        )?;
        let clip = match speech.tts.speak(text).await {
            Ok(clip) => Ok(clip),
            Err(error) => match &speech.fallback_tts {
                Some(fallback) => {
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
        self.ready(id, ended).await?;
        if interrupted.is_cancelled() {
            return Ok(());
        }
        let text = sentences.join(" ");
        let line = self.record("agent", &text)?;
        self.emit(
            id,
            VoiceEvent::Said {
                id: line.clone(),
                text,
            },
        );
        let _ = self.change(id, |call| {
            if call.state != VoiceState::Held {
                call.state(VoiceState::Speaking, None);
            }
            Ok(())
        });
        for (index, sentence) in sentences.iter().enumerate() {
            self.ready(id, ended).await?;
            if interrupted.is_cancelled() {
                return Ok(());
            }
            let speech = self.services()?.speech;
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
                let _ = call.events.send(VoiceEvent::Clip {
                    id: line.into(),
                    index,
                    r#final: last,
                    mime_type: clip.mime.clone(),
                    data: STANDARD.encode(&clip.bytes),
                });
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
    ) -> bool {
        let delivery = Delivery {
            persona: persona.into(),
            event: event.into(),
            name: name.into(),
            text: text.chars().take(12_000).collect(),
        };
        let calls = lock(&self.calls);
        if let Some(call) = calls
            .iter()
            .rev()
            .find(|call| call.state != VoiceState::Ended)
        {
            return call.work.try_send(Work::Delivery(delivery)).is_ok();
        }
        drop(calls);
        // Text-only desks retain their ordinary push and incur no voice cost.
        if self.services().is_err() || self.ledger.check().is_err() {
            return false;
        }
        let Ok(permit) = self.narration.clone().try_acquire_owned() else {
            return false;
        };
        let this = self.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let summary = match this.services() {
                Ok(services) => tokio::time::timeout(
                    Duration::from_secs(30),
                    services.dispatcher.narrate(
                        &delivery.name,
                        &delivery.text,
                        this.ledger.clone(),
                    ),
                )
                .await
                .ok()
                .and_then(Result::ok),
                Err(_) => None,
            };
            this.push(&delivery, summary.as_deref().unwrap_or(&delivery.text));
        });
        true
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
            for call in lock(&self.calls)
                .iter()
                .filter(|call| call.state != VoiceState::Ended)
            {
                let _ = call.events.send(VoiceEvent::Card {
                    persona_id: persona.into(),
                    request_id: request.into(),
                    kind: kind.into(),
                });
            }
        }
    }
}

fn goodbye(text: &str) -> bool {
    static GOODBYE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)^\s*(?:goodbye|good bye|bye(?: bye)?|hang up|end (?:the )?call|that's all|that is all)[.!?]*\s*$").expect("fixed goodbye regex")
    });
    GOODBYE.is_match(text)
}

fn sentences(text: &str) -> Vec<String> {
    // Bound spoken output even if a provider ignores its output limit.
    let bounded: String = text.chars().take(400).collect();
    bounded
        .split_inclusive(['.', '!', '?'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .take(2)
        .map(str::to_string)
        .collect()
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
