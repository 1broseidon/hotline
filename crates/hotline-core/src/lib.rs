//! Hotline's core, as a library. Everything shell-agnostic lives here; the
//! Tauri shell is a thin adapter over it. The design is in `docs/design.md`.
//!
//! The contract's types, the log and the room, the sessions and both of their
//! drivers, the workspace tools, Hotline as an MCP client and as an MCP server,
//! the remote desks and relays, and the wire that carries all of it to a
//! window, a phone or another desk.

pub(crate) mod accounts;
pub use accounts::support::account_setup_items;
pub mod boot;
mod capabilities;
pub mod computer;
pub use hotline_contract as contract;
#[cfg(windows)]
use hotline_support::process_windows;
pub use hotline_support::{credentials, log_file, paths, private_dir, room_lock};
use hotline_support::{fence, images, thumbs};
pub mod desk;
pub mod driver;
pub mod imagegen;
pub mod import;
pub(crate) mod link_preview;
pub(crate) use hotline_room::linked;
pub use hotline_store::log;
pub mod mcp;
pub mod models;
pub(crate) mod pricing;
#[cfg(feature = "test-support")]
pub mod provider_fixture;
mod providers;
pub(crate) mod push;
pub mod remote;
pub use hotline_room::room;
use hotline_store::sent;
pub(crate) mod session;
pub(crate) mod skills;
pub(crate) mod spending;
pub(crate) use hotline_store as store;
#[cfg(test)]
mod test_support;
pub(crate) mod thread;
pub use hotline_tools as tools;
pub mod vault;
pub mod voice;
pub(crate) use hotline_websearch as websearch;
pub mod wire;
