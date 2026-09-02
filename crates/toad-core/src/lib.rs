//! Toad's core, as a library. Everything shell-agnostic lives here; the
//! Tauri shell is a thin adapter over it. The design and its phases are in
//! `docs/design.md`.
//!
//! What is here today is the material of Phase 0: the contract's types, the
//! data directory's layout, the log every stream is written to and the room
//! folded out of it, the search index, the workspace tools, and Toad Agent's
//! turn loop on Rig — the last four moved over from the previous Toad's
//! migration branch. The session and the wire are built on top of these.

pub mod agent;
pub mod contract;
pub mod log;
pub mod paths;
pub mod room;
pub mod store;
pub mod tools;
pub mod vault;
