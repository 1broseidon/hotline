//! Toad's core, as a library. Everything shell-agnostic lives here; the
//! Tauri shell is a thin adapter over it. The design and its phases are in
//! `docs/design.md`.
//!
//! What is here today is the material of Phase 0: the contract's types, the
//! data directory's layout, the log every stream is written to and the room
//! folded out of it, the search index, the workspace tools, and the sessions
//! that run a teammate's turns over a driver — Toad Agent on Rig being the
//! one driver this phase has. The wire is built on top of these.

pub mod contract;
pub mod desk;
pub mod driver;
pub mod log;
pub mod paths;
pub mod room;
pub mod session;
pub mod store;
pub mod tools;
pub mod vault;
pub mod wire;
