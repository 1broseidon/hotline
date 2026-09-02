//! Toad's core, as a library. Everything shell-agnostic lives here; the
//! Tauri shell is a thin adapter over it. The design and its phases are in
//! `docs/design.md`.
//!
//! What is here today came over from the previous Toad's migration branch
//! and is the material of Phase 0: the contract's types, the data
//! directory's layout, the tape and thread streams, the search index, the
//! workspace tools, and Toad Agent's turn loop on Rig. The log, the room,
//! the session and the wire are built on top of these.

pub mod agent;
pub mod contract;
pub mod paths;
pub mod store;
pub mod tools;
pub mod transcript;
