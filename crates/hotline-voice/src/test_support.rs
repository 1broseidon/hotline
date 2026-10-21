//! Inspection and setup used by the real-room call regression tests.
use super::*;
pub use crate::dispatcher::meter::Meter;
pub use crate::input::validate_audio;
pub use crate::synthesis::{goodbye, own_voice, speech_ready};

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    super::lock(mutex)
}
pub fn model(id: SpeechId) -> VoiceModel {
    super::model(id)
}

impl Calls {
    pub fn test_change<T>(
        &self,
        id: &str,
        change: impl FnOnce(&mut Call) -> Result<T, String>,
    ) -> Result<T, String> {
        self.change(id, change)
    }
    pub fn test_activity(&self, id: &str, activity: Instant) {
        self.change(id, |call| {
            call.activity = activity;
            Ok(())
        })
        .unwrap();
    }
    pub fn test_writer(&self, persona: &str) -> String {
        self.writer(persona)
    }
    pub async fn test_synthesize(&self, speech: &CallSpeech, text: &str) -> Result<Clip, String> {
        self.synthesize(speech, text).await
    }
}

pub use crate::replies::Replies;
pub const BUDGET_LINE: &str = super::BUDGET_LINE;
pub const ERROR_LINE: &str = super::ERROR_LINE;
pub const RETRY_LINE: &str = super::RETRY_LINE;
pub const REWRITE_WAIT: Duration = super::REWRITE_WAIT;
pub const THINKING_AGAIN: Duration = super::THINKING_AGAIN;
pub const TOOL_OVERHEAD_BYTES: usize = crate::dispatcher::meter::TOOL_OVERHEAD_BYTES;

impl Calls {
    pub fn test_ready_for(&self, id: &str) -> Result<(), String> {
        self.ready_for(id)
    }
    pub fn test_lose_front(&self, id: &str) {
        self.fail_front(
            id,
            "Live voice disconnected and could not reconnect.".into(),
        );
    }
    pub async fn test_say(
        &self,
        id: &str,
        text: &str,
        interrupted: &CancellationToken,
        ended: &CancellationToken,
    ) -> Result<(), String> {
        self.say(id, text, interrupted, ended).await
    }
    pub fn test_clip(
        &self,
        id: &str,
        line: &str,
        index: u32,
        last: bool,
        clip: &Clip,
        interrupted: Option<&CancellationToken>,
    ) {
        self.clip(id, line, index, last, clip, interrupted);
    }
}
pub use crate::dispatcher::{speed_family, untrusted};
