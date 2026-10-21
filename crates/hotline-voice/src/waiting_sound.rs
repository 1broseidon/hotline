//! One quiet cue for a Legacy turn that has not begun speaking, on every client.

use crate::{Calls, VoiceEvent, VoiceState};
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
                    if call.front.is_none()
                        && !call.waiting_sound
                        && !call.speech.is_cancelled()
                        && call.state != VoiceState::Held
                        && (call.utterance_pending || call.answering.is_some())
                        && call
                            .first_clip_started
                            .is_some_and(|at| at.elapsed() >= Duration::from_millis(1_800))
                    {
                        call.waiting_sound = true;
                        let _ = call.events.send(VoiceEvent::Clip {
                            id: format!("waiting-{}", call.seq.unwrap_or_default()),
                            index: 0,
                            r#final: true,
                            mime_type: "audio/wav".into(),
                            data: STANDARD.encode(include_bytes!("assets/waiting.wav")),
                        });
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
