//! A front call's sound: the person's microphone PCM in, the front's speech
//! PCM out, and the barge-in that keeps the two honest.
//!
//! The desk owns the provider socket. The phone sends small frames up and
//! plays small frames down over the call's own wire, each side numbered, so
//! a gap is visible and nothing is queued without a bound.

use super::front::{self, FRONT_LINE_PREFIX};
use super::front_prompt::{self, Behind, Line, Speaker as LineSpeaker, Start};
use super::realtime::{
    Realtime, RealtimeConfig, RealtimeEvent, RealtimeInput, RealtimeSession, RealtimeTool,
};
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

    fn front_voice(&self) -> String {
        self.settings()
            .front
            .map(|front| {
                front
                    .voice
                    .unwrap_or_else(|| front.provider.default_voice().into())
            })
            .unwrap_or_else(|| FrontProvider::Grok.default_voice().into())
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
    /// pipeline call.
    fn lose_front(&self, id: &str) {
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
        let config = RealtimeConfig {
            instructions: self.front_instructions(persona.as_ref()),
            voice: self.front_voice(),
            tools: front::tools()
                .into_iter()
                .map(|tool| RealtimeTool {
                    name: tool.name.into(),
                    description: tool.description.into(),
                    parameters: tool.parameters,
                })
                .collect(),
            input_rate: FRONT_RATE,
            output_rate: FRONT_RATE,
        };
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
        let ended = self
            .carry_front(&id, realtime.as_ref(), session, bridge, &cancel)
            .await;
        let rest = since.elapsed().as_secs_f64() % SPEND_EVERY.as_secs_f64();
        let _ = self.front_spend(realtime.as_ref(), rest);
        match ended {
            FrontEnd::Call => {}
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
    ) -> FrontEnd {
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
        let mut minutes =
            tokio::time::interval_at(tokio::time::Instant::now() + SPEND_EVERY, SPEND_EVERY);
        loop {
            let event = tokio::select! {
                _ = cancel.cancelled() => return FrontEnd::Call,
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
                    let _ = input.text(&note.instruction()).await;
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
                } => {
                    let (output, reply) = bridge.tool_call(&name, &arguments);
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
