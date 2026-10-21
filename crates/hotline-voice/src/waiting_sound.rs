//! Bounded feedback while a Legacy teammate works without audible speech.

use crate::{Calls, VoiceEvent, VoiceState, Work};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

impl Calls {
    pub(super) async fn waiting_sound(self: Arc<Self>, id: String, cancel: CancellationToken) {
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                _ = tick.tick() => {}
            }
            if self
                .change(&id, |call| {
                    if call.front.is_some()
                        || call.speech.is_cancelled()
                        || call.state == VoiceState::Held
                        || call.caller_speaking
                        || !(call.utterance_pending || call.answering.is_some())
                    {
                        return Ok(());
                    }
                    let Some(at) = call.feedback_at else {
                        return Ok(());
                    };
                    let now = tokio::time::Instant::now();
                    if !call.waiting_sound && at.elapsed() >= Duration::from_millis(1_800) {
                        call.waiting_sound = true;
                        call.feedback_at = Some(now);
                        let _ = call.events.send(VoiceEvent::Clip {
                            id: format!("waiting-{}", call.seq.unwrap_or_default()),
                            index: 0,
                            r#final: true,
                            mime_type: "audio/wav".into(),
                            data: STANDARD.encode(include_bytes!("assets/waiting.wav")),
                        });
                    } else if at.elapsed() >= Duration::from_secs(20)
                        && call.state == VoiceState::Thinking
                        && call.streams.is_empty()
                        && let Some(seq) = call.answering
                        && call
                            .work
                            .try_send(Work::Progress {
                                seq,
                                at: now,
                                speech: call.speech.child_token(),
                            })
                            .is_ok()
                    {
                        call.feedback_at = Some(now);
                    }
                    Ok(())
                })
                .is_err()
            {
                return;
            }
        }
    }
}
