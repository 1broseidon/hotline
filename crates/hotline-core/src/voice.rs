//! The room's side of calls; the call runtime lives in `hotline-voice`.
pub mod connected;
pub mod room;
pub use hotline_voice::{CallRequest, Calls, LISTEN_WHILE_THINKING, Origin, Services, VOICE_FRONT};
pub use hotline_voice::{dispatcher, ledger, metering, settings, spoken};
#[cfg(test)]
pub(crate) use room::tests;
