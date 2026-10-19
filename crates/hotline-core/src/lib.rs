//! Hotline's core, as a library. Everything shell-agnostic lives here; the
//! Tauri shell is a thin adapter over it. The design is in `docs/design.md`.
//!
//! The contract's types, the log and the room, the sessions and both of their
//! drivers, the workspace tools, Hotline as an MCP client and as an MCP server,
//! the remote desks and relays, and the wire that carries all of it to a
//! window, a phone or another desk.

pub mod boot;
mod capabilities;
pub mod computer;
pub use hotline_contract as contract;
pub mod credentials;
pub mod desk;
pub mod driver;
mod fence;
pub mod imagegen;
mod images;
pub mod import;
pub(crate) mod link_preview;
pub(crate) mod linked;
pub mod log;
pub mod log_file;
pub mod mcp;
pub mod models;
pub mod paths;
pub(crate) mod pricing;
#[cfg(windows)]
mod process_windows;
mod providers;
pub(crate) mod push;
pub mod remote;
pub mod room;
pub mod room_lock;
mod sent;
pub(crate) mod session;
pub(crate) mod skills;
pub(crate) mod spending;
pub(crate) mod store;
#[cfg(test)]
mod test_support;
pub(crate) mod thread;
mod thumbs;
pub mod tools;
pub mod vault;
pub mod voice;
pub(crate) mod websearch;
pub mod wire;
