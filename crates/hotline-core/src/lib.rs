//! Hotline's core, as a library. Everything shell-agnostic lives here; the
//! Tauri shell is a thin adapter over it. The design and its phases are in
//! `docs/design.md`.
//!
//! What is here today is the material of Phase 0 and both halves of MCP from
//! Phase 2: the contract's types, the log, the room, the search index, the
//! workspace tools, Hotline Agent on Rig, Hotline as an MCP client for the servers
//! a teammate's policy grants, and Hotline as the MCP server of a teammate's own
//! tools — over its conversation, and over the teammates it shares the room
//! with. The wire is built on top of these.

pub mod computer;
pub mod contract;
pub mod credentials;
pub mod desk;
pub mod driver;
mod fence;
pub mod import;
pub mod log;
pub mod mcp;
pub mod models;
pub mod paths;
#[cfg(windows)]
mod process_windows;
mod providers;
pub mod push;
pub mod remote;
pub mod room;
pub mod session;
pub mod skills;
pub mod store;
pub mod tools;
pub mod vault;
pub mod wire;
