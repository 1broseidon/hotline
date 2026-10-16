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
use super::record::Digest;
use super::settings::FrontProvider;
use super::*;
use crate::contract::Persona;
use crate::log::StreamId;

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
/// A longer call posts a summary to the DM this often, as well as at hang-up.
const SUMMARY_EVERY: Duration = Duration::from_secs(300);

/// A front call's side of the sound. Held by the call, dropped with it.
pub(super) struct Front {
    /// Absent while the provider is connecting.
    input: Option<RealtimeInput>,
    mic_rate: u32,
    mic_seq: Option<u32>,
    out_seq: u32,
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
            None => speech::resolve_front(&self.vault, &self.settings()),
        }
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
    fn front_instructions(&self, persona: Option<&Persona>) -> String {
        let Some(persona) = persona else {
            return front_prompt::instructions(&Behind::Desk, &Start::default());
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
                voice: "",
            },
            &Start {
                person: None,
                handoff,
                recent: &recent,
            },
        )
    }

    /// The teammate a call is to, or `None` on a desk call.
    fn front_persona(&self, id: &str) -> Option<Persona> {
        let target = self
            .change(id, |call| Ok(call.target.clone()))
            .ok()
            .flatten()?;
        crate::room::roster(&self.log)
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
    pub(super) fn front_model(&self) -> Option<crate::contract::VoiceFrontModel> {
        let realtime = self.realtime()?;
        let provider = self
            .settings()
            .front
            .map_or(FrontProvider::Grok, |front| front.provider);
        Some(crate::contract::VoiceFrontModel {
            provider_id: provider.key().into(),
            model_id: realtime.id().model_id,
            voice: self.front_voice(None),
            voices: provider
                .voices()
                .iter()
                .map(|voice| voice.to_string())
                .collect(),
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
            let front = call.front.as_ref().ok_or("This call has no voice front.")?;
            let input = front
                .input
                .clone()
                .ok_or("The front is still connecting.")?;
            call.activity = Instant::now();
            if call.state == VoiceState::Speaking {
                call.state(call.resting(), None);
            }
            Ok(input)
        })?;
        input
            .truncate(played_ms)
            .await
            .map_err(|_| "The front is no longer on the call.".to_string())
    }

    /// The front is gone for the rest of the call, which goes on as a
    /// pipeline call; a desk with no pipeline to fall back on ends it.
    pub(super) fn lose_front(&self, id: &str) {
        let pipeline = self.change(id, |call| {
            Ok(call
                .speech_services
                .front_only
                .then(|| (call.input_mode, call.target.clone())))
        });
        if let Ok(Some((input_mode, target))) = pipeline {
            match self.resolve_speech(input_mode, target.as_deref()) {
                Ok(services) => {
                    let _ = self.change(id, |call| {
                        call.speech_services = services;
                        Ok(())
                    });
                }
                Err(_) => {
                    let _ = self.finish(id, VoiceEndReason::Error);
                    return;
                }
            }
        }
        let _ = self.change(id, |call| {
            if call.front.take().is_some() {
                let _ = call.events.send(VoiceEvent::FrontEnded {});
                if call.state == VoiceState::Speaking {
                    call.state(call.resting(), None);
                }
            }
            Ok(())
        });
    }

    /// The call ended with the person's last words unanswered: they go to
    /// the teammate as a message from the call, marked as ended, so it
    /// answers in writing in the conversation rather than on a call that is
    /// gone, as a typed line would go (its session and open chapter).
    fn follow_up(&self, id: &str, target: &str, words: String, seq: u32) {
        let Some(room) = self.room.upgrade() else {
            return;
        };
        let origin = Origin {
            call_id: id.into(),
            seq,
            direct: true,
            front: true,
            ended: true,
        };
        let target = target.to_string();
        tokio::spawn(async move {
            match tokio::time::timeout(Duration::from_secs(60), room.start(&target)).await {
                Ok(Ok(_)) => {}
                _ => return,
            }
            let prompt = crate::wire::commands::CALL_ORIGIN
                .scope(origin, room.prompt(&target, &words, None, None));
            if let Err(error) = prompt.await {
                eprintln!("[voice] the call's last question did not reach the teammate: {error}");
            }
        });
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
        let Ok(bridge) = front::Front::open(&self, &id, speaks) else {
            return;
        };
        // A call to a teammate holds the teammate's desk-side tools; a desk
        // call speaks for no one teammate and holds none.
        let desk_tools = match (&persona, self.room.upgrade()) {
            (Some(persona), Some(room)) => {
                let reach = room.front_reach(persona);
                let opened = tokio::select! {
                    _ = cancel.cancelled() => return,
                    opened = FrontTools::open(Arc::downgrade(&room), &id, &persona.id, reach) => opened,
                };
                Some(Arc::new(opened))
            }
            _ => None,
        };
        let desk_definitions = desk_tools
            .as_ref()
            .map(|tools| tools.definitions())
            .unwrap_or_default();
        let config = RealtimeConfig {
            instructions: self.front_instructions(persona.as_ref()),
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
        let bridge = bridge.with_desk_tools(desk_tools);
        let opened = tokio::select! {
            _ = cancel.cancelled() => return,
            opened = realtime.open(config) => opened,
        };
        let session = match opened {
            Ok(session) => session,
            Err(error) => {
                eprintln!("[voice] the front could not open: {error}");
                self.lose_front(&id);
                return;
            }
        };
        let since = Instant::now();
        let mut kept = Kept {
            teammate: persona.as_ref().map(|persona| persona.id.clone()),
            ..Kept::default()
        };
        let ended = self
            .carry_front(&id, realtime.as_ref(), session, bridge, &cancel, &mut kept)
            .await;
        if let Some(persona) = &persona {
            self.post_summary(&persona.id, &id, &mut kept.digest, kept.unanswered.as_ref());
        }
        let rest = since.elapsed().as_secs_f64() % SPEND_EVERY.as_secs_f64();
        let _ = self.front_spend(realtime.as_ref(), rest);
        match ended {
            FrontEnd::Call => {
                if let (Some(persona), Some((words, seq))) = (persona, kept.unanswered) {
                    self.follow_up(&id, &persona.id, words, seq);
                }
            }
            FrontEnd::Lost => self.lose_front(&id),
            FrontEnd::Budget => {
                let _ = self.finish(&id, VoiceEndReason::Budget);
            }
        }
    }

    async fn carry_front(
        &self,
        id: &str,
        realtime: &dyn Realtime,
        session: RealtimeSession,
        mut bridge: front::Front,
        cancel: &CancellationToken,
        kept: &mut Kept,
    ) -> FrontEnd {
        let Kept {
            unanswered,
            digest,
            teammate,
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
        let mut saying = String::new();
        let mut replies = 0u32;
        let mut heard = 0u32;
        // `unanswered` is what the person last asked that nothing has
        // answered yet, neither handed to the teammate nor replied to in full
        // by the front, kept current so whichever way the call ends sees it.
        *unanswered = None;
        let mut minutes =
            tokio::time::interval_at(tokio::time::Instant::now() + SPEND_EVERY, SPEND_EVERY);
        let mut summaries =
            tokio::time::interval_at(tokio::time::Instant::now() + SUMMARY_EVERY, SUMMARY_EVERY);
        loop {
            let event = tokio::select! {
                _ = cancel.cancelled() => return FrontEnd::Call,
                _ = summaries.tick() => {
                    if let Some(teammate) = teammate.as_deref() {
                        // What is unanswered is still being answered; it is
                        // left open only when the call ends on it.
                        self.post_summary(teammate, id, digest, None);
                    }
                    continue;
                }
                _ = minutes.tick() => {
                    if !self.front_connected(realtime) {
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
                    let _ = input.say(note.text()).await;
                    continue;
                }
                event = events.recv() => event,
            };
            let event = match event {
                Some(Ok(event)) => event,
                Some(Err(error)) => {
                    eprintln!("[voice] the front failed: {error}");
                    return FrontEnd::Lost;
                }
                None => return FrontEnd::Lost,
            };
            let seen = match event {
                RealtimeEvent::Audio(pcm) => self.front_speaks(id, &pcm),
                RealtimeEvent::SpeechStarted => {
                    saying.clear();
                    self.change(id, |call| {
                        let _ = call.events.send(VoiceEvent::FrontStop {});
                        if call.state == VoiceState::Speaking {
                            call.state(call.resting(), None);
                        }
                        Ok(())
                    })
                }
                RealtimeEvent::Heard(text) => {
                    digest.asked(&text);
                    *unanswered =
                        worth_following_up(&text).then(|| (text.clone(), bridge.next_seq()));
                    let seq = heard;
                    heard += 1;
                    self.front_heard(id, &text);
                    self.change(id, |call| {
                        call.activity = Instant::now();
                        let _ = call.events.send(VoiceEvent::Heard { seq, text });
                        Ok(())
                    })
                }
                RealtimeEvent::Saying(piece) => {
                    saying.push_str(&piece);
                    Ok(())
                }
                RealtimeEvent::ResponseDone => {
                    let text = std::mem::take(&mut saying);
                    let reply = format!("{FRONT_LINE_PREFIX}{replies}");
                    replies += 1;
                    if !text.trim().is_empty() {
                        self.front_said(id, &text);
                        *unanswered = None;
                    }
                    self.change(id, |call| {
                        if !text.trim().is_empty() {
                            let _ = call.events.send(VoiceEvent::Said { id: reply, text });
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
                    digest.ran(&name);
                    // Run beside the call, so a slow tool does not hold the
                    // front's speech. The model speaks once it has the result.
                    let input = input.clone();
                    let cancel = cancel.clone();
                    tokio::spawn(async move {
                        let output = tokio::select! {
                            _ = cancel.cancelled() => return,
                            output = tools.call(&name, &arguments) => output,
                        };
                        let _ = input.tool_result(&call_id, &output, true).await;
                    });
                    Ok(())
                }
                RealtimeEvent::ToolCall {
                    call_id,
                    name,
                    arguments,
                } => {
                    let (output, reply) = bridge.tool_call(&name, &arguments);
                    if name == front::SEND_TO_TEAMMATE && !reply {
                        *unanswered = None;
                        digest.handed(&handed_words(&arguments));
                    }
                    let _ = input.tool_result(&call_id, &output, reply).await;
                    Ok(())
                }
                RealtimeEvent::Refused { code } => {
                    eprintln!("[voice] the front refused a request: {code:?}");
                    Ok(())
                }
                RealtimeEvent::Renewed => Ok(()),
            };
            if seen.is_err() {
                return FrontEnd::Call;
            }
        }
    }

    /// Posts what the call has done since its last post to the teammate's DM
    /// as one summary, or nothing when nothing happened. The turns of the
    /// call are not posted: its thread keeps them.
    fn post_summary(
        &self,
        persona_id: &str,
        id: &str,
        digest: &mut Digest,
        unanswered: Option<&(String, u32)>,
    ) {
        let open = unanswered.map(|(words, _)| words.as_str());
        let (Some(summary), Some(room)) = (digest.summary(open), self.room.upgrade()) else {
            return;
        };
        room.call_posted(persona_id, id, &summary);
    }

    /// The front's speech as numbered frames, while the call is not held.
    fn front_speaks(&self, id: &str, pcm: &[u8]) -> Result<(), String> {
        self.change(id, |call| {
            if call.state == VoiceState::Held || call.front.is_none() {
                return Ok(());
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
    /// What the person last asked that nothing has answered yet, neither
    /// handed to the teammate nor replied to in full by the front.
    unanswered: Option<(String, u32)>,
    /// What the call has done since its last summary to the DM.
    digest: Digest,
    /// The teammate a direct call is with, which gets the summaries.
    teammate: Option<String>,
}

/// The words a `send_to_teammate` call carried.
fn handed_words(arguments: &str) -> String {
    serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|arguments| arguments["words"].as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Whether words left unanswered when a call ended are worth answering in
/// writing: a question, or a request of a few words, and never a goodbye.
pub(super) fn worth_following_up(words: &str) -> bool {
    let words = words.trim();
    !super::goodbye(words) && (words.ends_with('?') || words.split_whitespace().count() >= 4)
}

/// Why a front stopped carrying its call.
enum FrontEnd {
    /// The call ended, or went on without needing the front.
    Call,
    /// The provider failed or closed, or its login was revoked: the call goes
    /// on as a pipeline call.
    Lost,
    /// The Voice budget cannot pay for another minute: the call ends.
    Budget,
}
