//! Hotline's voice engines: what hears a person and speaks to them, and the
//! realtime models that front a call.
//!
//! `speech` is the turn-by-turn seam, [`speech::Speech`]: speech to text and
//! text to speech over the providers the owner connected (the OpenAI shape,
//! Google, xAI) and the desk's own models (`speech::local`, sherpa-onnx), and
//! which of them a call uses. `realtime` is the live seam,
//! [`realtime::Realtime`]: one spoken conversation with a speech-to-speech
//! model, Grok Voice or GPT Realtime over a socket and GPT-Live over WebRTC.
//!
//! A crate of its own so the engines can be built and tested without the
//! core, and cannot reach the calls, the sessions or the vault. The calls
//! that use them (the dispatcher, the turns, the front) stay in the core's
//! `voice`, which reports to the live room. The vault stays in the core too:
//! what it keeps that voice reads reaches here as [`speech::Connected`],
//! values the core reads and hands in, with the two logins whose tokens the
//! core's providers refresh (Grok's and ChatGPT's) as functions in it. It
//! stands on `hotline-contract`, `hotline-support` and `hotline-room` (voice's
//! settings, `ProviderAuth`, the TLS default), and must never depend on
//! `hotline-core` or on Tauri. The core re-exports both modules at
//! `crate::voice::speech` and `crate::voice::realtime`.

pub mod realtime;
pub mod speech;
