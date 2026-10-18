//! A call's life: starting it, holding it, interrupting it and ending it.

use super::dispatcher::Context;
use super::duplex::Front;
use super::record::Record;
use super::{CALLS_OFF, Call, CallSpeech, Calls, THINKING_AGAIN, lock, paid_kinds};
use crate::contract::{
    VoiceCall, VoiceCallMode, VoiceEndReason, VoiceEvent, VoiceInputMode, VoiceState,
};
use crate::wire::RoomHandle;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

impl Calls {
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
        self.start_call(
            id,
            target,
            stream_audio,
            input_mode,
            VoiceCallMode::Pipeline,
            None,
            room,
        )
    }

    /// Starts a call, or answers for one already started. A front is a
    /// request: the answer says whether the call got one, and a call that
    /// did not runs on the pipeline.
    #[allow(clippy::too_many_arguments)]
    pub fn start_call(
        self: &Arc<Self>,
        id: &str,
        target: Option<String>,
        stream_audio: bool,
        input_mode: VoiceInputMode,
        mode: VoiceCallMode,
        mic_rate: Option<u32>,
        room: Arc<dyn RoomHandle>,
    ) -> Result<VoiceCall, String> {
        if self.settings().calls_off {
            return Err(CALLS_OFF.into());
        }
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
                || call.input_mode != input_mode
                || (call.front.is_some() && mode != VoiceCallMode::Front))
        {
            return Err("A call cannot change its target or input/output mode.".into());
        }
        if !calls.iter().any(|call| call.id == id) {
            // A front the budget cannot pay for leaves the call on the pipeline.
            let realtime = self
                .realtime()
                .filter(|_| mode == VoiceCallMode::Front)
                .filter(|front| self.front_ready(front.as_ref()).is_ok());
            // Direct calls do not need a separate routing model. A front
            // hears and speaks itself, so with one the pipeline is only what
            // the call would fall back on, and a desk with none still takes
            // the call: a ChatGPT login alone runs GPT-Live.
            let speech_services = match self.resolve_speech(input_mode, target.as_deref()) {
                Ok(services) => services,
                Err(error) => match &realtime {
                    Some(front) => CallSpeech::front(front.as_ref()),
                    None => return Err(error),
                },
            };
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
                words: String::new(),
                front: realtime.is_some().then(|| Front::new(mic_rate)),
            });
            if let Some(realtime) = realtime {
                tokio::spawn(self.clone().run_front(id.into(), realtime, cancel.clone()));
            }
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
                            .await;
                    }
                    None => run.await,
                }
            });
        }
        Ok(VoiceCall {
            call_id: id.into(),
            front: calls
                .iter()
                .find(|call| call.id == id)
                .and_then(|call| call.front.as_ref().map(Front::description)),
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
    pub(super) fn thinking_again(self: &Arc<Self>, id: &str, cancel: CancellationToken) {
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

    pub(super) fn change<T>(
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
    pub(super) fn finish(&self, id: &str, reason: VoiceEndReason) -> Result<(), String> {
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
}
