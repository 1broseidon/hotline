//! An agent account: a named sign-in the operator keeps for an agent, which a
//! teammate's agent process runs under instead of the agent's own default.
//!
//! An account is a folder under the data directory that the agent is pointed
//! at. Everything here is the same for every agent and parameterized by that
//! agent's row in [`support`]; a new agent joins by getting a row, a parser for
//! its status answer and the tests of both, and nothing else in this module
//! changes (spec §9.1.1).
//!
//! The pieces, in the order a spawn meets them:
//! - [`resolve`] turns a teammate's chosen account into its checked folder, or
//!   a refusal, before any process starts. A teammate with no account, or on an
//!   agent with no row, resolves to nothing, which is the agent's own default.
//! - [`env`] plans the process environment: the launch's, with every inherited
//!   credential removed and the folder set last, so the account's sign-in is
//!   the only one the agent can find. No account plans nothing, byte for byte
//!   what a launch did before accounts existed.
//! - [`status`] reads what the agent says about a folder's sign-in and trusts
//!   it only when it names exactly that folder.

pub(crate) mod env;
pub(crate) mod resolve;
pub(crate) mod setup;
pub(crate) mod status;
pub(crate) mod support;
