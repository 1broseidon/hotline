//! A front call's sound: the person's microphone PCM in, the front's speech
//! PCM out, and the barge-in that keeps the two honest.
//!
//! The desk owns the provider socket. The phone sends small frames up and
//! plays small frames down over the call's own wire, each side numbered, so
//! a gap is visible and nothing is queued without a bound.

use super::front::{self, FRONT_LINE_PREFIX};
use super::front_prompt::{self, Behind, Line, Speaker as LineSpeaker, Start};
use super::front_tools::FrontTools;
use super::realtime::{
    Realtime, RealtimeConfig, RealtimeEvent, RealtimeInput, RealtimeSession, RealtimeTool,
};
use super::settings::FrontProvider;
use super::*;
use hotline_contract::Persona;
use hotline_store::log::StreamId;

/// The rate the front hears and speaks at, and the one a call is asked for.
pub(super) const FRONT_RATE: u32 = 24_000;
/// The microphone rates the desk takes, resampling the one that is not the
/// front's own.
const MIC_RATES: [u32; 2] = [16_000, FRONT_RATE];
/// One frame of the front's speech: 100 ms.
const FRAME_BYTES: usize = FRONT_RATE as usize / 10 * 2;
const CHUNK_LIMIT: usize = 32 * 1024;
/// A paid front is charged for its minutes as they pass, so a spent budget
/// ends the call within one of them.
const SPEND_EVERY: Duration = Duration::from_secs(60);
const LIVE_BUDGET: &str = "The Voice budget is spent. Raise it in Settings or choose Turn by turn.";
const LIVE_UNAVAILABLE: &str =
    "Live voice is unavailable. You can try again or choose Turn by turn.";

/// A front call's side of the sound. Held by the call, dropped with it.
pub(super) struct Front {
    /// Absent while the provider is connecting.
    input: Option<RealtimeInput>,
    mic_rate: u32,
    mic_seq: Option<u32>,
    out_seq: u32,
    interrupted: bool,
    pub(super) responding: bool,
}

impl Front {
    pub(super) fn new(asked_rate: Option<u32>) -> Self {
        Self {
            input: None,
            mic_rate: asked_rate
                .filter(|rate| MIC_RATES.contains(rate))
                .unwrap_or(FRONT_RATE),
            mic_seq: None,
            out_seq: 0,
            interrupted: false,
            responding: false,
        }
    }

    pub(super) fn description(&self) -> VoiceFront {
        VoiceFront {
            input_rate: self.mic_rate,
            output_rate: FRONT_RATE,
        }
    }
}

/// Linear resampling of mono PCM16. Each frame is resampled alone, so a
/// frame's last sample repeats rather than reaching into the next.
fn resample(pcm: &[u8], from: u32, to: u32) -> Vec<u8> {
    let samples: Vec<i16> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| i16::from_le_bytes(*pair))
        .collect();
    let count = samples.len() * to as usize / from as usize;
    let mut out = Vec::with_capacity(count * 2);
    for index in 0..count {
        let position = index as f64 * f64::from(from) / f64::from(to);
        let before = position as usize;
        let after = (before + 1).min(samples.len() - 1);
        let weight = position - before as f64;
        let sample =
            f64::from(samples[before]) * (1.0 - weight) + f64::from(samples[after]) * weight;
        out.extend((sample.round() as i16).to_le_bytes());
    }
    out
}

impl Calls {
    /// The realtime model a front call would speak through: an injected one,
    /// or the one the owner picked in Settings, resolved from the vault.
    pub(crate) fn realtime(&self) -> Option<Arc<dyn Realtime>> {
        match &self.injected {
            Some(services) => services.realtime.clone(),
            None => speech::resolve_front(&self.room.connected(), &self.settings()),
        }
    }

    /// A fixed notice still uses the selected live voice while it is available.
    pub(super) async fn front_notice(&self, id: &str, text: &str) -> Result<(), String> {
        let input = self.change(id, |call| {
            Ok(call.front.as_ref().and_then(|front| front.input.clone()))
        })?;
        if let Some(input) = input {
            input.say(text).await.map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Whether the login or key a front was opened with is still connected.
    /// Revoking it ends the front, as it ends a pipeline call's speech.
    fn front_connected(&self, front: &dyn Realtime) -> bool {
        self.injected.is_some()
            || self.login_connected(&front.id().provider_id, front.is_subscription())
    }

    /// Whether the call is spoken by a front, which says the call's lines
    /// itself, so the pipeline only shows them.
    pub(super) fn fronted(&self, id: &str) -> bool {
        self.change(id, |call| Ok(call.front.is_some()))
            .unwrap_or(false)
    }

    /// What the front is told before the call: on a call to a teammate, its
    /// name, the open chapter's handoff note and the latest lines of its DM
    /// said in words; on a desk call, only how to behave.
    fn front_instructions(&self, persona: Option<&Persona>, tools: &[front::Tool]) -> String {
        let settings = hotline_room::room::settings(&self.log);
        let person = settings
            .get("personName")
            .and_then(serde_json::Value::as_str);
        let Some(persona) = persona else {
            let roster: Vec<_> = hotline_room::room::roster(&self.log)
                .into_iter()
                .map(|p| serde_json::json!({"id":p.id,"name":p.name,"goal":p.goal}))
                .collect();
            return format!(
                "{}\nRoster (data): {}",
                front_prompt::instructions(
                    &Behind::Desk,
                    &Start {
                        person,
                        ..Default::default()
                    }
                ),
                super::dispatcher::untrusted(&serde_json::json!(roster))
            );
        };
        let tape = self.log.load(&StreamId::Tape(persona.id.clone()));
        let handoff = tape
            .iter()
            .rev()
            .filter(|event| event["kind"] == "chapter")
            .find_map(|event| event["note"].as_str());
        let recent: Vec<Line> = tape
            .iter()
            .filter_map(|event| {
                let speaker = match event["kind"].as_str()? {
                    "user" => LineSpeaker::Person,
                    "agent" => LineSpeaker::Teammate,
                    _ => return None,
                };
                Some(Line {
                    speaker,
                    text: event["text"].as_str()?,
                })
            })
            .collect();
        front_prompt::instructions(
            &Behind::Teammate {
                public_name: &persona.name,
                voice: &persona.goal,
            },
            &Start {
                person,
                handoff,
                recent: &recent,
                tools,
            },
        )
    }

    /// The teammate a call is to, or `None` on a desk call.
    fn front_persona(&self, id: &str) -> Option<Persona> {
        let target = self
            .change(id, |call| Ok(call.target.clone()))
            .ok()
            .flatten()?;
        hotline_room::room::roster(&self.log)
            .into_iter()
            .find(|persona| persona.id == target)
    }

    /// The voice a front call speaks in: the teammate's own when it picked
    /// one of this front's voices, else the one picked for the front, else
    /// the front's default.
    fn front_voice(&self, persona: Option<&Persona>) -> String {
        let front = self.settings().front;
        let provider = front
            .as_ref()
            .map_or(FrontProvider::Grok, |front| front.provider);
        let own = persona
            .and_then(|persona| persona.voice.as_ref())
            .filter(|voice| voice.provider_id == provider.key())
            .map(|voice| voice.voice.as_str());
        let picked = front.as_ref().and_then(|front| front.voice.as_deref());
        provider.voice_or_default(own.or(picked)).to_string()
    }

    /// The front calls would open, for `voice.status`.
    pub(super) fn front_model(&self) -> Option<hotline_contract::VoiceFrontModel> {
        let realtime = self.realtime()?;
        let provider = self
            .settings()
            .front
            .map_or(FrontProvider::Grok, |front| front.provider);
        Some(hotline_contract::VoiceFrontModel {
            provider_id: provider.key().into(),
            model_id: realtime.id().model_id,
            voice: self.front_voice(None),
            voices: provider.voices().iter().map(ToString::to_string).collect(),
        })
    }

    /// Whether this desk can run a voice front for a call.
    pub fn front_available(&self) -> bool {
        self.realtime().is_some()
    }

    /// Microphone audio for a front call's model. Never waits: a front that
    /// is behind, or still connecting, refuses the frame.
    pub fn front_audio(&self, id: &str, seq: u32, data: &str) -> Result<(), String> {
        if data.len() > CHUNK_LIMIT * 4 / 3 + 4 {
            return Err("A voice audio chunk is too large.".into());
        }
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| "Voice audio must be standard base64.".to_string())?;
        if bytes.is_empty() || bytes.len() > CHUNK_LIMIT || bytes.len() % 2 != 0 {
            return Err("Send at most 32 KiB of PCM16 per chunk.".into());
        }
        let (input, rate) = self.change(id, |call| {
            if call.state == VoiceState::Held {
                return Err("Resume the call before speaking.".into());
            }
            let front = call.front.as_mut().ok_or("This call has no voice front.")?;
            if front.mic_seq.is_some_and(|previous| seq <= previous) {
                return Err("Send an increasing sequence with front audio.".into());
            }
            front.mic_seq = Some(seq);
            let input = front
                .input
                .clone()
                .ok_or("The front is still connecting.")?;
            Ok((input, front.mic_rate))
        })?;
        let pcm = if rate == FRONT_RATE {
            bytes
        } else {
            resample(&bytes, rate, FRONT_RATE)
        };
        input
            .try_push_audio(pcm)
            .map_err(|_| "The front is behind. Drop this audio and send the next.".to_string())
    }

    /// The person spoke over the front, and `played_ms` of its reply had
    /// been heard: the front keeps that much of it.
    pub async fn front_barge(&self, id: &str, played_ms: u32) -> Result<(), String> {
        let input = self.change(id, |call| {
            let front = call.front.as_mut().ok_or("This call has no voice front.")?;
            let input = front
                .input
                .clone()
                .ok_or("The front is still connecting.")?;
            // A client that barges on every caller turn reports nothing played
            // once the reply has finished: there is nothing to cut.
            if played_ms == 0 && !front.responding {
                return Ok(None);
            }
            front.interrupted = front.responding;
            call.cut_speech(Some(played_ms));
            call.activity = Instant::now();
            if call.state == VoiceState::Speaking {
                call.state(call.resting(), None);
            }
            Ok(Some(input))
        })?;
        let Some(input) = input else {
            return Ok(());
        };
        input
            .truncate(played_ms)
            .await
            .map_err(|_| "The front is no longer on the call.".to_string())
    }

    /// A failed live call ends visibly. Only a new explicit choice starts Turn by turn.
    pub(super) fn fail_front(&self, id: &str, reason: String) {
        self.end_front(
            id,
            reason,
            LIVE_UNAVAILABLE,
            include_bytes!("assets/live-unavailable.wav"),
        );
    }

    fn end_front(&self, id: &str, reason: String, notice: &str, audio: &[u8]) {
        eprintln!(
            "[voice] call {id}: live failed, awaiting an explicit choice of turn by turn: {reason}"
        );
        if self
            .change(id, |call| {
                if call.live_error.is_some() {
                    return Err("Already reported.".into());
                }
                call.live_error = Some(reason.clone());
                Ok(())
            })
            .is_err()
        {
            return;
        }
        self.system_line(id, notice, audio, None);
        let _ = self.change(id, |call| {
            call.live_error = Some(reason);
            call.state(VoiceState::Ended, Some(VoiceEndReason::Error));
            Ok(())
        });
    }

    pub(super) fn front_selected(&self) -> bool {
        if hotline_room::room::settings(&self.log)
            .get("voice")
            .and_then(|voice| voice.get("front"))
            .is_some()
        {
            return self.settings().front.is_some();
        }
        self.injected.as_ref().map_or_else(
            || self.settings().front.is_some(),
            |services| services.realtime.is_some(),
        )
    }

    pub(super) fn required_front(&self) -> Result<Arc<dyn Realtime>, String> {
        let front = self.realtime().ok_or_else(|| {
            match self.settings().front.map(|front| front.provider) {
                Some(FrontProvider::GptLive) => "Live voice needs a ChatGPT sign-in in Settings.",
                Some(FrontProvider::OpenAi) => "Live voice needs an OpenAI API key in Settings.",
                _ => "Live voice needs an xAI sign-in or API key in Settings.",
            }
            .to_string()
        })?;
        self.front_ready(front.as_ref())?;
        Ok(front)
    }

    /// Carries one call's front: opens the provider session with the
    /// bridge's tools, then turns what the provider produces into the call's
    /// events, its tool calls into the bridge's answers, and what the bridge
    /// has for the person into lines for the front to say, until the call or
    /// the session ends. Its minutes are charged as they pass and once more
    /// when it stops.
    pub(super) async fn run_front(
        self: Arc<Self>,
        id: String,
        realtime: Arc<dyn Realtime>,
        cancel: CancellationToken,
    ) {
        // Subscribed before anything can be handed over, so no reply is missed.
        let persona = self.front_persona(&id);
        let speaks = if persona.is_some() {
            front::Speaks::AsTeammate
        } else {
            front::Speaks::ForDesk
        };
        let bridge = match front::Front::open(&self, &id, speaks) {
            Ok(bridge) => bridge,
            Err(error) => {
                self.fail_front(&id, error);
                return;
            }
        };
        // A call to a teammate holds the teammate's desk-side tools; a desk
        // call speaks for no one teammate and holds none.
        let desk_tools = match (&persona, self.live_room()) {
            (Some(persona), Some(room)) => {
                let Some(thread) = lock(&self.calls)
                    .iter()
                    .find(|call| call.id == id)
                    .and_then(|call| call.thread.clone())
                else {
                    self.fail_front(&id, "Live voice could not find the call’s thread.".into());
                    return;
                };
                let Ok(reach) = room.front_reach(persona, &thread) else {
                    self.fail_front(
                        &id,
                        "Live voice could not open the teammate’s tools.".into(),
                    );
                    return;
                };
                let opened = FrontTools::open(self.room.clone(), &thread, &persona.id, reach);
                Some(Arc::new(opened))
            }
            _ => None,
        };
        let desk_definitions = desk_tools
            .as_ref()
            .map(|tools| tools.definitions())
            .unwrap_or_default();
        let mut instructions = self.front_instructions(persona.as_ref(), &desk_definitions);
        if desk_definitions
            .iter()
            .any(|tool| tool.name == "message_teammate")
        {
            instructions.push_str("\nYou are answering the desk as team lead. Answer roster questions yourself with list_teammates. Keep the spoken answer short: names and what each teammate is working on, or idle. Do not recite models, tools or machine reach unless the person asks for those details. When the person names a teammate to do work, use message_teammate with intent handoff and their words unchanged. Say who has it after acceptance. This routing is the exception to work_on_it and to not mentioning handoffs: never send named work through work_on_it. If the name is ambiguous, ask which teammate. You cannot stop or reassign another teammate's work; that stays the person's choice.");
        }
        let config = RealtimeConfig {
            instructions: self.front_context(&id, &instructions, None),
            voice: self.front_voice(persona.as_ref()),
            tools: front::tools()
                .into_iter()
                .chain(desk_definitions)
                .map(|tool| RealtimeTool {
                    name: tool.name,
                    description: tool.description,
                    parameters: tool.parameters,
                })
                .collect(),
            input_rate: FRONT_RATE,
            output_rate: FRONT_RATE,
        };
        let mut bridge = bridge.with_desk_tools(desk_tools);
        let opened = tokio::select! {
            () = cancel.cancelled() => return,
            opened = realtime.open(config.clone()) => opened,
        };
        let session = match opened {
            Ok(session) => session,
            Err(error) => {
                eprintln!("[voice] the front could not open: {error}");
                self.fail_front(&id, format!("Live voice could not start: {error}"));
                return;
            }
        };
        let mut kept = Kept {
            base: instructions,
            ..Kept::default()
        };
        let mut session = session;
        let mut recovered = false;
        let ended = loop {
            eprintln!(
                "[voice] call {id}: live connected provider={} model={}",
                realtime.id().provider_id,
                realtime.id().model_id
            );
            let progress = (kept.heard, kept.replies);
            let since = Instant::now();
            let ended = self
                .carry_front(
                    &id,
                    realtime.as_ref(),
                    session,
                    &mut bridge,
                    &cancel,
                    &mut kept,
                )
                .await;
            let _ = self.change(&id, |call| {
                call.keep_front_speech();
                Ok(())
            });
            let rest = since.elapsed().as_secs_f64() % SPEND_EVERY.as_secs_f64();
            if self.front_spend(realtime.as_ref(), rest).is_err() {
                break FrontEnd::Budget;
            }
            if !matches!(ended, FrontEnd::Lost) {
                break ended;
            }
            if (kept.heard, kept.replies) != progress {
                recovered = false;
            }
            if !self.front_connected(realtime.as_ref()) || recovered {
                eprintln!(
                    "[voice] call {id}: reconnect unavailable: login revoked or replacement failed"
                );
                break FrontEnd::Lost;
            }
            if self.front_ready(realtime.as_ref()).is_err() {
                break FrontEnd::Budget;
            }
            eprintln!("[voice] call {id}: provider lost; reconnecting with call context");
            let _ = self.change(&id, |call| {
                call.reconnecting = true;
                let _ = call.events.send(call.snapshot());
                if let Some(front) = &mut call.front {
                    front.input = None;
                }
                let _ = call.events.send(VoiceEvent::FrontStop {});
                Ok(())
            });
            let mut replacement = config.clone();
            replacement.instructions = self.front_context(&id, &kept.base, Some(&kept.carry));
            if let Ok(Some(cut)) = self.change(&id, |call| Ok(call.speech_line.pending.clone())) {
                replacement.instructions.push_str("\n\n");
                replacement
                    .instructions
                    .push_str(&front_prompt::scrub(&cut));
            }
            let opened = tokio::select! {
                () = cancel.cancelled() => break FrontEnd::Call,
                opened = tokio::time::timeout(Duration::from_secs(30), realtime.open(replacement)) => opened,
            };
            match opened {
                Ok(Ok(fresh)) => {
                    session = fresh;
                    recovered = true;
                }
                failure => {
                    eprintln!(
                        "[voice] call {id}: reconnect failed: {}",
                        match failure {
                            Ok(Err(error)) => error.to_string(),
                            _ => "provider timed out".into(),
                        }
                    );
                    break FrontEnd::Lost;
                }
            }
        };
        match ended {
            FrontEnd::Call => {}
            FrontEnd::Lost => self.fail_front(
                &id,
                "Live voice disconnected and could not reconnect.".into(),
            ),
            FrontEnd::Budget => self.end_front(
                &id,
                LIVE_BUDGET.into(),
                LIVE_BUDGET,
                include_bytes!("assets/live-budget.wav"),
            ),
        }
    }

    async fn carry_front(
        &self,
        id: &str,
        realtime: &dyn Realtime,
        session: RealtimeSession,
        bridge: &mut front::Front,
        cancel: &CancellationToken,
        kept: &mut Kept,
    ) -> FrontEnd {
        let Kept {
            base,
            carry,
            heard,
            replies,
        } = kept;
        let RealtimeSession { input, mut events } = session;
        let connected = self.change(id, |call| {
            call.front
                .as_mut()
                .map(|front| front.input = Some(input.clone()))
                .ok_or_else(|| "No front.".to_string())
        });
        if connected.is_err() {
            return FrontEnd::Call;
        }
        let recovered = self
            .change(id, |call| {
                let recovered = call.reconnecting;
                call.reconnecting = false;
                let _ = call.events.send(call.snapshot());
                Ok(recovered)
            })
            .unwrap_or(false);
        if recovered {
            let _ = input.say("We're back on the same call.").await;
        }
        let mut saying = String::new();
        let mut awaiting_transcript = false;
        let mut pending_work = Vec::<(String, String)>::new();
        // Subscribe before reading, so a post racing the snapshot is refreshed.
        // Idle calls must not parse the whole home tape twice a second.
        let mut context_events = self
            .change(id, |call| {
                let streams = call
                    .thread
                    .as_ref()
                    .and_then(StreamId::of_thread)
                    .into_iter()
                    .chain(call.target.clone().map(StreamId::Tape));
                Ok(streams
                    .map(|stream| self.log.subscribe(&stream))
                    .collect::<Vec<_>>())
            })
            .unwrap_or_default();
        let mut work_state = self.change(id, |call| Ok((call.answering, call.utterance_pending)));
        let mut last_instructions = self.front_context(id, base, Some(carry));
        let mut refresh = tokio::time::interval(Duration::from_millis(500));

        let mut minutes =
            tokio::time::interval_at(tokio::time::Instant::now() + SPEND_EVERY, SPEND_EVERY);
        loop {
            let event = tokio::select! {
                () = cancel.cancelled() => return FrontEnd::Call,
                _ = refresh.tick() => {
                    if let Some(text) = self.next_card(id) {
                        let _ = input.say(&text).await;
                    }
                    let state = self.change(id, |call| Ok((call.answering, call.utterance_pending)));
                    let mut changed = state != work_state;
                    work_state = state;
                    for events in &mut context_events {
                        while let Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) = events.try_recv() {
                            changed = true;
                        }
                    }
                    if !changed { continue; }
                    let fresh = self.front_context(id, base, Some(carry));
                    if fresh != last_instructions {
                        if input.instructions(&fresh).await.is_err() { return FrontEnd::Lost; }
                        last_instructions = fresh;
                    }
                    continue;
                }
                _ = minutes.tick() => {
                    if !self.front_connected(realtime) {
                        eprintln!("[voice] call {id}: provider login disconnected");
                        return FrontEnd::Lost;
                    }
                    if self.front_spend(realtime, SPEND_EVERY.as_secs_f64()).is_err() {
                        return FrontEnd::Budget;
                    }
                    continue;
                }
                // What the bridge has for the person, said word for word. A
                // front that is gone is noticed by its events ending.
                note = bridge.next_note() => {
                    let Some(note) = note else { return FrontEnd::Call };
                    let _ = input.say(&note).await;
                    continue;
                }
                event = events.recv() => event,
            };
            let event = match event {
                Some(Ok(event)) => event,
                failure => {
                    match failure {
                        Some(Err(error)) => eprintln!("[voice] the front failed: {error}"),
                        _ => eprintln!("[voice] call {id}: provider events ended"),
                    }
                    if !saying.is_empty() {
                        carry.push(
                            "You",
                            &format!("[Speech ended with the connection] {saying}"),
                        );
                        *replies += 1;
                    }
                    return FrontEnd::Lost;
                }
            };
            if matches!(
                event,
                RealtimeEvent::ToolCall { .. } | RealtimeEvent::Heard(_)
            ) {
                let _ = self.change(id, |call| {
                    call.keep_front_speech();
                    Ok(())
                });
            }
            let renewed = matches!(event, RealtimeEvent::Renewed);
            let refresh_now = renewed
                || matches!(
                    event,
                    RealtimeEvent::Heard(_)
                        | RealtimeEvent::ResponseDone
                        | RealtimeEvent::ToolCall { .. }
                );
            let seen = match event {
                RealtimeEvent::Audio(pcm) => {
                    if saying.is_empty() {
                        let _ = self.change(id, |call| {
                            call.speech_line.update(
                                &format!("{FRONT_LINE_PREFIX}{id}-{replies}"),
                                "[Audio generated; transcript not yet available.]",
                            );
                            Ok(())
                        });
                    }
                    self.front_speaks(id, &pcm)
                }
                RealtimeEvent::SpeechStarted => {
                    awaiting_transcript = true;
                    self.change(id, |call| {
                        call.caller_speaking = true;
                        if call.front.as_ref().is_some_and(|front| front.responding) {
                            call.cut_speech(None);
                        }
                        if let Some(front) = &mut call.front {
                            front.interrupted = front.responding;
                        }
                        let _ = call.events.send(VoiceEvent::FrontStop {});
                        if call.state == VoiceState::Speaking {
                            call.state(call.resting(), None);
                        }
                        Ok(())
                    })
                }
                RealtimeEvent::Heard(text) => {
                    awaiting_transcript = false;
                    bridge.heard(&text);
                    let _ = self.change(id, |call| {
                        call.conversation.begin_turn();
                        Ok(())
                    });
                    // GPT-Live may end the caller's turn without a separate
                    // completion for the interrupted assistant turn.
                    let _ = self.change(id, |call| {
                        if let Some(front) = &mut call.front {
                            if front.interrupted {
                                carry.push(
                                    "You",
                                    &format!(
                                        "[Interrupted speech; not heard to the end] {}",
                                        std::mem::take(&mut saying)
                                    ),
                                );
                                *replies += 1;
                            }
                            front.interrupted = false;
                            front.responding = false;
                        }
                        Ok(())
                    });
                    let seq = *heard;
                    *heard += 1;
                    carry.push("Caller", &text);
                    self.front_heard(id, &text);
                    let _ = self.change(id, |call| {
                        call.caller_speaking = false;
                        call.cards.answered = false;
                        Ok(())
                    });
                    if let Some(answer) = self.answer_card_words(id, &text).await {
                        let _ = self.change(id, |call| {
                            if let Some(front) = &mut call.front {
                                front.responding = true;
                            }
                            Ok(())
                        });
                        let _ = input.say(&answer).await;
                    }
                    for (call_id, arguments) in pending_work.drain(..) {
                        work(bridge, &input, &call_id, &arguments).await;
                    }
                    self.change(id, |call| {
                        call.activity = Instant::now();
                        let _ = call.events.send(VoiceEvent::Heard { seq, text });
                        Ok(())
                    })
                }
                RealtimeEvent::Saying(piece) => {
                    saying.push_str(&piece);
                    self.change(id, |call| {
                        if let Some(front) = &mut call.front {
                            front.responding = true;
                        }
                        call.speech_line
                            .update(&format!("{FRONT_LINE_PREFIX}{id}-{replies}"), &saying);
                        Ok(())
                    })
                }
                RealtimeEvent::ResponseDone => {
                    let interrupted = self
                        .change(id, |call| {
                            call.keep_front_speech();
                            Ok(call.front.as_mut().is_some_and(|front| {
                                front.responding = false;
                                std::mem::take(&mut front.interrupted)
                            }))
                        })
                        .unwrap_or(true);
                    let text = std::mem::take(&mut saying);
                    let reply = format!("{FRONT_LINE_PREFIX}{replies}");
                    *replies += 1;
                    let kept_text = if interrupted {
                        format!("[Interrupted speech; not heard to the end] {text}")
                    } else {
                        text.clone()
                    };
                    carry.push("You", &kept_text);
                    self.change(id, |call| {
                        if !text.trim().is_empty() {
                            let _ = call.events.send(VoiceEvent::Said {
                                id: reply,
                                text: kept_text,
                            });
                        }
                        if call.state == VoiceState::Speaking {
                            call.state(call.resting(), None);
                        }
                        Ok(())
                    })
                }
                RealtimeEvent::ToolCall {
                    call_id,
                    name,
                    arguments,
                } if let Some(tools) = bridge.desk_tools().filter(|tools| tools.has(&name)) => {
                    // Run beside the call, so a slow tool does not hold the
                    // front's speech. The model speaks once it has the result.
                    let input = input.clone();
                    let cancel = cancel.clone();
                    let line = tools.begin(&name, &arguments);
                    tokio::spawn(async move {
                        let output = tools.call(&name, &arguments, line, &cancel).await;
                        let _ = input.tool_result(&call_id, &output, true).await;
                    });
                    Ok(())
                }
                RealtimeEvent::ToolCall {
                    call_id,
                    name,
                    arguments,
                } => {
                    if name == crate::realtime::WORK_ON_IT {
                        // A tool can beat asynchronous transcription. Admission
                        // needs caller words, not the model's proposed completion.
                        if front::stops_work(&arguments) {
                            for (pending_id, _) in pending_work.drain(..) {
                                let _ = input
                                    .tool_result(
                                        &pending_id,
                                        "Work was withdrawn before it started. Remain silent.",
                                        false,
                                    )
                                    .await;
                            }
                        }
                        if awaiting_transcript && !front::stops_work(&arguments) {
                            pending_work.push((call_id, arguments));
                        } else {
                            work(bridge, &input, &call_id, &arguments).await;
                        }
                    } else {
                        let (output, reply) = bridge.tool_call(&name, &arguments).await;
                        let _ = input.tool_result(&call_id, &output, reply).await;
                    }
                    Ok(())
                }
                RealtimeEvent::Refused { code } => {
                    eprintln!("[voice] the front refused a request: {code:?}");
                    Ok(())
                }
                RealtimeEvent::Reconnecting => self.change(id, |call| {
                    call.reconnecting = true;
                    let _ = call.events.send(VoiceEvent::FrontStop {});
                    let _ = call.events.send(call.snapshot());
                    Ok(())
                }),
                RealtimeEvent::Renewed => {
                    eprintln!("[voice] call {id}: provider renewed or reconnected with context");
                    let recovered = self.change(id, |call| {
                        let recovered = call.reconnecting;
                        call.reconnecting = false;
                        let _ = call.events.send(call.snapshot());
                        Ok(recovered)
                    });
                    if matches!(recovered, Ok(true)) {
                        let _ = input.say("We're back on the same call.").await;
                    }
                    recovered.map(|_| ())
                }
            };
            if refresh_now {
                let fresh = self.front_context(id, base, Some(carry));
                if renewed || fresh != last_instructions {
                    if input.instructions(&fresh).await.is_err() {
                        return FrontEnd::Lost;
                    }
                    last_instructions = fresh;
                }
            }
            if seen.is_err() {
                return FrontEnd::Call;
            }
        }
    }

    /// The front's speech as numbered frames, while the call is not held.
    fn front_speaks(&self, id: &str, pcm: &[u8]) -> Result<(), String> {
        self.change(id, |call| {
            if call.state == VoiceState::Held
                || call.front.as_ref().is_none_or(|front| front.interrupted)
            {
                return Ok(());
            }
            if let Some(front) = &mut call.front {
                front.responding = true;
            }
            call.activity = Instant::now();
            if call.state != VoiceState::Speaking {
                call.state(VoiceState::Speaking, None);
            }
            let front = call.front.as_mut().expect("checked above");
            for frame in pcm.chunks(FRAME_BYTES) {
                let seq = front.out_seq;
                front.out_seq += 1;
                let _ = call.events.send(VoiceEvent::FrontAudio {
                    seq,
                    data: STANDARD.encode(frame),
                });
            }
            Ok(())
        })
    }
}

/// What a front call keeps for after it, whichever way it ends.
#[derive(Default)]
struct Kept {
    base: String,
    /// Conversation used when the realtime connection is renewed.
    carry: realtime::socket::Carry,
    heard: u32,
    replies: u32,
}

/// Admit or stop work and answer the provider without waiting for a model turn.
async fn work(bridge: &mut front::Front, input: &RealtimeInput, call_id: &str, arguments: &str) {
    let (output, reply) = bridge
        .tool_call(crate::realtime::WORK_ON_IT, arguments)
        .await;
    let _ = input.tool_result(call_id, &output, reply).await;
}

/// Why a front stopped carrying its call.
enum FrontEnd {
    /// The call ended, or went on without needing the front.
    Call,
    /// The provider failed or closed, or its login was revoked. Reconnect
    /// if permitted, otherwise end the live call visibly.
    Lost,
    /// The Voice budget cannot pay for another minute: the call ends.
    Budget,
}
