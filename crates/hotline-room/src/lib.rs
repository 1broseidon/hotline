//! The room as everything above the core sees it, and the plain pieces the
//! core's next crates need below them: the roster and settings folded out of
//! the room stream (`room`, with the `mcpServers` and `agentAccounts` settings'
//! reading beside it in `mcp_servers` and `agent_accounts`), linked desks'
//! records and live links (`linked`), the
//! seam the wire calls the live room through (`room_handle`), the tool ledger
//! every session publishes (`tool_ledger`), voice's settings, spend and
//! spoken replies (`voice`), and a handful of items that hold no state:
//! `ProviderAuth`, the secret and text helpers, the TLS default and where a
//! computer mounts a teammate's home.
//!
//! A crate of its own so that the wire, the MCP client and the voice engines
//! can stand on the room without standing on the core, and so that the room
//! cannot reach the sessions, the drivers or the wire. It stands on
//! `hotline-contract`, `hotline-support`, `hotline-store` and
//! `hotline-websearch`, and must never depend on `hotline-core` or on Tauri.
//! `hotline-core` re-exports each module under its old path (`room`,
//! `linked`, `session::ledger`, `voice::ledger` and the rest, and
//! `wire::RoomHandle`), so the core's callers did not move.

pub mod agent_accounts;
pub mod computer;
pub mod linked;
pub mod mcp_servers;
pub mod provider_auth;
pub mod room;
pub mod room_handle;
pub mod secret;
pub mod text;
pub mod tls;
pub mod tool_ledger;
pub mod voice;
