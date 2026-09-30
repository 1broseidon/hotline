//! A desk owns one voice call. The wire carries complete sentence clips.
//!
//! This first integration milestone uses canned speech; the provider pipeline
//! replaces `utterance` while retaining the call and subscription contract.

use crate::contract::{
    VoiceBudget, VoiceCall, VoiceEndReason, VoiceEvent, VoiceModel, VoiceState, VoiceStatus,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use uuid::Uuid;

const IDLE: Duration = Duration::from_secs(600);
const MAX_AUDIO: usize = 2 * 1024 * 1024;

struct Call {
    id: String,
    state: VoiceState,
    reason: Option<VoiceEndReason>,
    seq: Option<u32>,
    activity: Instant,
    events: broadcast::Sender<VoiceEvent>,
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
    }
}

pub struct Calls {
    log: crate::log::Log,
    calls: Mutex<VecDeque<Call>>,
}

impl Calls {
    pub fn new(log: crate::log::Log) -> Arc<Self> {
        let calls = Arc::new(Self {
            log,
            calls: Mutex::new(VecDeque::new()),
        });
        let weak = Arc::downgrade(&calls);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let Some(calls) = weak.upgrade() else { break };
                let mut calls = calls
                    .calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                for call in calls.iter_mut() {
                    if call.state != VoiceState::Ended && call.activity.elapsed() >= IDLE {
                        call.state(VoiceState::Ended, Some(VoiceEndReason::Idle));
                    }
                }
            }
        });
        calls
    }

    pub fn status(&self) -> VoiceStatus {
        let available = crate::room::settings(&self.log)
            .get("voice")
            .and_then(|voice| voice.get("stub"))
            .and_then(serde_json::Value::as_bool)
            == Some(true);
        let model = VoiceModel {
            provider_id: "stub".into(),
            model_id: "integration-stub".into(),
            voice: None,
        };
        VoiceStatus {
            available,
            unavailable: (!available).then(|| "The voice integration stub is disabled.".into()),
            stt: Some(model.clone()),
            tts: Some(model.clone()),
            fallback_tts: None,
            dispatcher: Some(model),
            budget: VoiceBudget {
                day_usd: 2.0,
                month_usd: 20.0,
                spent_day_usd: 0.0,
                spent_month_usd: 0.0,
            },
        }
    }

    pub fn start(&self, id: &str) -> Result<VoiceCall, String> {
        if !self.status().available {
            return Err(
                "Enable settings.voice.stub on a scratch desk for the integration milestone."
                    .into(),
            );
        }
        Uuid::parse_str(id).map_err(|_| "A voice call needs a UUID.".to_string())?;
        let mut calls = self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !calls.iter().any(|call| call.id == id) {
            for call in calls
                .iter_mut()
                .filter(|call| call.state != VoiceState::Ended)
            {
                call.state(VoiceState::Ended, Some(VoiceEndReason::Replaced));
            }
            while calls.len() >= 32 {
                calls.pop_front();
            }
            calls.push_back(Call {
                id: id.into(),
                state: VoiceState::Listening,
                reason: None,
                seq: None,
                activity: Instant::now(),
                events: broadcast::channel(128).0,
            });
        }
        Ok(VoiceCall {
            call_id: id.into(),
            input: vec!["audio/wav".into(), "audio/mp4".into()],
            output: "audio/wav".into(),
        })
    }

    pub fn subscribe(
        &self,
        id: &str,
    ) -> Result<(VoiceEvent, broadcast::Receiver<VoiceEvent>), String> {
        let calls = self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let call = calls
            .iter()
            .find(|call| call.id == id)
            .ok_or("That voice call is no longer available.")?;
        Ok((call.snapshot(), call.events.subscribe()))
    }

    fn change(
        &self,
        id: &str,
        change: impl FnOnce(&mut Call) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut calls = self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let call = calls
            .iter_mut()
            .find(|call| call.id == id)
            .ok_or("That voice call is no longer available.")?;
        if call.state == VoiceState::Ended {
            return Err("That voice call has ended.".into());
        }
        call.activity = Instant::now();
        change(call)
    }

    pub fn end(&self, id: &str) -> Result<(), String> {
        self.change(id, |call| {
            call.state(VoiceState::Ended, Some(VoiceEndReason::Client));
            Ok(())
        })
    }

    pub fn hold(&self, id: &str, hold: bool) -> Result<(), String> {
        self.change(id, |call| {
            call.state(
                if hold {
                    VoiceState::Held
                } else {
                    VoiceState::Listening
                },
                None,
            );
            Ok(())
        })
    }

    pub fn interrupt(&self, id: &str) -> Result<(), String> {
        self.change(id, |call| {
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
        self.change(id, |call| {
            if call.state == VoiceState::Held {
                return Err("Resume the call before speaking.".into());
            }
            if call.seq.is_some_and(|previous| seq <= previous) {
                return Err("Utterance sequence numbers must rise.".into());
            }
            call.seq = Some(seq);
            call.state(VoiceState::Thinking, None);
            let _ = call.events.send(VoiceEvent::Heard {
                seq,
                text: "ask Mack to check the failing PR".into(),
            });
            let id = Uuid::new_v4().to_string();
            let _ = call.events.send(VoiceEvent::Said {
                id: id.clone(),
                text: "The voice connection is working. This is the integration stub.".into(),
            });
            call.state(VoiceState::Speaking, None);
            let _ = call.events.send(VoiceEvent::Clip {
                id,
                index: 0,
                r#final: true,
                mime_type: "audio/wav".into(),
                data: STANDARD.encode(tone()),
            });
            call.state(VoiceState::Listening, None);
            Ok(())
        })
    }
}

fn tone() -> Vec<u8> {
    let samples = 3200_u32;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&16000_u32.to_le_bytes());
    wav.extend_from_slice(&32000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples * 2).to_le_bytes());
    for n in 0..samples {
        let sample = ((n as f32 * std::f32::consts::TAU * 440.0 / 16000.0).sin() * 2000.0) as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}
