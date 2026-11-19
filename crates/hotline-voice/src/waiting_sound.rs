//! Turn-by-turn presence while a teammate is working:
//! - One quiet tone if 8 seconds pass with no spoken audio on the turn
//!   (unexplained silence — e.g. slow STT, provider hiccup, or a tool-first
//!   turn that skipped its opening line). Never fires once the teammate has
//!   already spoken on the turn.
//! - One overrun note per turn, left for the teammate's running turn when 90
//!   seconds pass with nothing said since its last spoken clip (or, on a turn
//!   that never spoke, since the tone): the teammate reads it with its next
//!   tool results and says where the work stands. It never repeats, and it
//!   never interrupts the work to arrive.

use super::Calls;
use base64::{Engine, engine::general_purpose::STANDARD};
use hotline_contract::{VoiceEvent, VoiceState};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub(crate) const UNEXPLAINED_WAIT: Duration = Duration::from_secs(8);
pub(crate) const OVERRUN_SILENCE: Duration = Duration::from_secs(90);

impl Calls {
    pub(super) async fn waiting_sound(self: Arc<Self>, id: String, cancel: CancellationToken) {
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                _ = tick.tick() => {}
            }
            let nudge = self.change(&id, |call| {
                if call.front.is_some()
                    || call.speech.is_cancelled()
                    || call.state == VoiceState::Held
                    || call.caller_speaking
                    || !(call.utterance_pending || call.answering.is_some())
                {
                    return Ok(None);
                }
                let now = tokio::time::Instant::now();
                if !call.waiting_sound
                    && let Some(at) = call.feedback_at
                    && at.elapsed() >= UNEXPLAINED_WAIT
                {
                    call.waiting_sound = true;
                    call.feedback_at = Some(now);
                    let _ = call.events.send(VoiceEvent::Clip {
                        id: format!("waiting-{}", call.seq.unwrap_or_default()),
                        index: 0,
                        r#final: true,
                        mime_type: "audio/wav".into(),
                        data: STANDARD.encode(include_bytes!("assets/waiting.wav")),
                    });
                    return Ok(None);
                }
                if !call.overrun_nudged
                    && call.state == VoiceState::Thinking
                    && call.streams.is_empty()
                    && call.answering.is_some()
                    && let Some(quiet_since) = call.last_spoken_at.or(call.feedback_at)
                    && quiet_since.elapsed() >= OVERRUN_SILENCE
                    && let (Some(target), Some(thread)) = (call.target.clone(), call.thread.clone())
                {
                    call.overrun_nudged = true;
                    return Ok(Some((target, thread)));
                }
                Ok(None)
            });
            match nudge {
                Ok(Some((target, thread))) => {
                    if let Some(room) = self.live_room() {
                        room.nudge_overrun(&target, &thread);
                    }
                }
                Ok(None) => {}
                Err(_) => return,
            }
        }
    }
}
