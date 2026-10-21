//! A call's life: starting it, holding it, interrupting it and ending it.

use super::dispatcher::Context;
use super::duplex::Front;
use super::record::Record;
use super::{CALLS_OFF, Call, CallSpeech, Calls, THINKING_AGAIN, lock, paid_kinds};
use hotline_contract::{
    VoiceCall, VoiceCallMode, VoiceEndReason, VoiceEvent, VoiceInputMode, VoiceState,
};
use hotline_room::room_handle::RoomHandle;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// What a call is started with: whom it is to, how it hears and speaks, and
/// whether it asks for a front, with the rate the microphone sends that front.
pub struct CallRequest {
    pub target: Option<String>,
    pub thread: Option<hotline_contract::ThreadId>,
    pub stream_audio: bool,
    pub input_mode: VoiceInputMode,
    pub mode: VoiceCallMode,
    pub mic_rate: Option<u32>,
}

pub(super) struct Connection {
    owner: String,
    replaced: CancellationToken,
    disconnected: CancellationToken,
    pub(super) dropped: bool,
    deadline: Option<tokio::time::Instant>,
    was_held: bool,
}

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
        let request = CallRequest {
            target,
            thread: None,
            stream_audio,
            input_mode,
            mode: VoiceCallMode::Selected,
            mic_rate: None,
        };
        self.start_call(id, request, room)
    }

    /// Starts a call in the selected mode; live failures never select Legacy.
    pub fn start_call(
        self: &Arc<Self>,
        id: &str,
        request: CallRequest,
        room: Arc<dyn RoomHandle>,
    ) -> Result<VoiceCall, String> {
        let result = self.start_requested(id, request, room);
        if let Err(reason) = &result {
            eprintln!("[voice] call {id}: start refused: {reason}");
        }
        result
    }

    fn start_requested(
        self: &Arc<Self>,
        id: &str,
        request: CallRequest,
        room: Arc<dyn RoomHandle>,
    ) -> Result<VoiceCall, String> {
        let CallRequest {
            target,
            thread,
            stream_audio,
            input_mode,
            mode,
            mic_rate,
        } = request;
        let live = mode == VoiceCallMode::Front
            || (mode == VoiceCallMode::Selected && self.front_selected());
        eprintln!("[voice] call {id}: start requested={mode:?} live={live}");
        if self.settings().calls_off {
            return Err(CALLS_OFF.into());
        }
        if let Some(target) = &target
            && !hotline_room::room::roster(&self.log)
                .iter()
                .any(|persona| &persona.id == target)
        {
            return Err("That teammate is no longer in the room.".into());
        }
        if thread.is_some() && target.is_none() {
            return Err("Choose the thread's teammate before calling.".into());
        }
        let existing_thread = thread.is_some();
        let thread = target
            .as_ref()
            .map(|_| thread.unwrap_or_else(|| hotline_contract::ThreadId::call(id)));
        if existing_thread && let (Some(target), Some(thread)) = (&target, &thread) {
            self.live_room()
                .ok_or("The desk has closed.")?
                .validate_call_thread(target, thread)?;
        }
        Uuid::parse_str(id).map_err(|_| "A voice call needs a UUID.".to_string())?;
        let mut calls = lock(&self.calls);
        if let Some(call) = calls.iter().find(|call| call.id == id)
            && (call.target != target
                || call.thread != thread
                || call.stream_audio != stream_audio
                || call.input_mode != input_mode
                || (call.front.is_some() != live))
        {
            return Err("A call cannot change its target or input/output mode.".into());
        }
        if !calls.iter().any(|call| call.id == id) {
            let realtime = if live {
                Some(self.required_front()?)
            } else {
                eprintln!("[voice] call {id}: Legacy selected explicitly or in Settings");
                None
            };
            let speech_services = match &realtime {
                Some(front) => CallSpeech::front(front.as_ref()),
                None => self.resolve_speech(input_mode, target.as_deref())?,
            };
            if target.is_none() && !live {
                self.dispatcher()?;
            }
            // Only the budgets this call would pay into can refuse it.
            if !live {
                let assistant = self.assistant_for(target.as_deref());
                self.ledger
                    .ready(&paid_kinds(Some(&speech_services), assistant.as_deref()))
                    .map_err(|e| e.to_string())?;
            }
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
                thread: thread.clone(),
                stream_audio,
                input_mode,
                speech_services,
                input: None,
                spoken_events: VecDeque::new(),
                connection: None,
                reconnecting: false,
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
                waiting_sound: false,
                answering: None,
                streams: HashMap::new(),
                cut: VecDeque::new(),
                speech_line: super::record::SpeechLine::default(),
                interrupted_through: None,
                record: target.as_deref().map(|persona_id| {
                    Record::open(
                        self.room.clone(),
                        thread.as_ref().expect("a direct call has a thread"),
                        persona_id,
                    )
                }),
                words: String::new(),
                front: realtime.is_some().then(|| Front::new(mic_rate)),
                live_error: None,
            });
            if let Some(realtime) = realtime {
                tokio::spawn(self.clone().run_front(id.into(), realtime, cancel.clone()));
            } else {
                tokio::spawn(self.clone().waiting_sound(id.into(), cancel.clone()));
            }
            self.thinking_again(id, cancel.clone());
            let this = self.clone();
            let id = id.to_string();
            let context = Context::new(self.log.clone(), room, self.clone(), cancel);
            // What the person says on this call carries the app they called from.
            let client = crate::call_room::prompt_client();
            tokio::spawn(async move {
                let run = this.run(id, context, rx);
                match client {
                    Some(client) => {
                        crate::call_room::PROMPT_CLIENT.scope(client, run).await;
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
                    () = cancel.cancelled() => return,
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

    /// Transport loss has a grace period; losing the pairing does not.
    pub fn bind_connection(
        self: &Arc<Self>,
        id: String,
        owner: &str,
        disconnected: CancellationToken,
        revoked: CancellationToken,
    ) -> Result<(), String> {
        let generation = CancellationToken::new();
        let Some((cancel, resumed)) = self.change(&id, |call| {
            if let Some(connection) = &call.connection {
                if connection.owner != owner {
                    return Err("Only the device that started this call can resume it.".into());
                }
                if connection
                    .deadline
                    .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
                {
                    call.state(VoiceState::Ended, Some(VoiceEndReason::Client));
                    return Ok(None);
                }
                // A second subscription observes an active call; it does not
                // transfer its lifetime to another window's socket.
                if !connection.disconnected.is_cancelled() {
                    return Ok(None);
                }
                connection.replaced.cancel();
            }
            let resumed = call
                .connection
                .as_ref()
                .is_some_and(|c| c.dropped || c.disconnected.is_cancelled());
            let held = call
                .connection
                .as_ref()
                .map_or(call.state == VoiceState::Held, |c| {
                    if c.dropped {
                        c.was_held
                    } else {
                        call.state == VoiceState::Held
                    }
                });
            call.connection = Some(Connection {
                owner: owner.into(),
                replaced: generation.clone(),
                disconnected: disconnected.clone(),
                dropped: false,
                deadline: None,
                was_held: held,
            });
            if resumed && !held {
                call.state(call.resting(), None);
            }
            Ok(Some((call.cancel.clone(), resumed && !held)))
        })?
        else {
            return Ok(());
        };
        if resumed {
            let _ = self.change(&id, |call| {
                let notice = super::Delivery {
                    persona: call.target.clone().unwrap_or_default(),
                    name: String::new(),
                    event: String::new(),
                    text: "We're back on the same call.".into(),
                };
                let _ = call
                    .work
                    .try_send(super::Work::Notice(notice, call.deliveries.child_token()));
                Ok(())
            });
        }
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::select! {
                biased;
                () = revoked.cancelled() => {
                    if let Some(this) = this.upgrade() { let _ = this.end(&id); }
                    return;
                }
                () = generation.cancelled() => return,
                () = cancel.cancelled() => return,
                () = disconnected.cancelled() => {},
            }
            if let Some(this) = this.upgrade() {
                let _ = this.change(&id, |call| {
                    // Rebinding can race with this task waking from the old socket.
                    if generation.is_cancelled() {
                        return Ok(());
                    }
                    if let Some(connection) = &mut call.connection {
                        connection.dropped = true;
                        connection.deadline =
                            Some(tokio::time::Instant::now() + std::time::Duration::from_secs(20));
                        connection.was_held = call.state == VoiceState::Held;
                    }
                    call.input = None;
                    call.speech.cancel();
                    call.deliveries.cancel();
                    call.deliveries = CancellationToken::new();
                    call.cut_speech(None);
                    call.stop_streams(true);
                    call.state(VoiceState::Held, None);
                    Ok(())
                });
            }
            tokio::select! {
                biased;
                () = generation.cancelled() => {},
                () = cancel.cancelled() => {},
                () = revoked.cancelled() => {
                    if let Some(this) = this.upgrade() { let _ = this.end(&id); }
                }
                () = tokio::time::sleep(std::time::Duration::from_secs(20)) => {
                    if let Some(this) = this.upgrade() {
                        let _ = this.change(&id, |call| {
                            if !generation.is_cancelled() {
                                call.state(VoiceState::Ended, Some(VoiceEndReason::Client));
                            }
                            Ok(())
                        });
                    }
                }
            }
        });
        Ok(())
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
    /// teammate's work goes on, but further speech from its old turn is
    /// muted. The person's next turn receives the playback context.
    pub fn interrupt(&self, id: &str) -> Result<(), String> {
        self.change(id, |call| {
            call.activity = Instant::now();
            call.input = None;
            call.speech.cancel();
            call.deliveries.cancel();
            call.deliveries = CancellationToken::new();
            call.cut_speech(None);
            call.interrupted_through = call.seq;
            call.stop_streams(true);
            call.answering = None;
            if call.state != VoiceState::Held {
                call.state(call.resting(), None);
            }
            Ok(())
        })
    }
}
