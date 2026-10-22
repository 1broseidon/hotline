//! The room's contract types: what the window is handed when it asks about a
//! teammate, and what the room pushes at it unasked.
//!
//! A crate of its own so that it depends on no other Hotline crate: every
//! other crate may name these types, and none of them may reach back. These
//! are defined once, in the files below, one per concern, and re-exported
//! here; `hotline-core` re-exports this crate as `contract`, so every
//! `hotline_core::contract::X` path holds. The TypeScript is generated from
//! them: `cargo test -p hotline-contract` runs the tests `#[ts(export)]`
//! writes and leaves `ui/src/generated/contract.ts` behind, which the window
//! re-exports rather than spelling the same shapes a second time. Every
//! exported type lives here, Remote's, linked desks' and the importer's
//! included, because ts-rs starts the file afresh in each test process: a
//! type exported from another crate would erase these, or be erased. The
//! workspace's `.cargo/config.toml` is what points ts-rs at that directory
//! and tells it a 64-bit integer is a JavaScript `number`; run the tests
//! unfiltered and check the regenerated file in with the change that moved it.
//!
//! Every attribute here serves one invariant: the JSON serde emits must be
//! what the Bun main emits for the same value, field for field. That is why
//! an optional field carries `skip_serializing_if` — absent means ABSENT, and
//! a `null` where Bun wrote nothing is a different value to the window — and
//! why the core's store tests (`store/tests.rs`) feed its own output through
//! these types and back out again.

mod capture;
mod command;
mod computer;
mod credentials;
mod payloads;
mod persona;
mod remote;
mod replies;
mod session;
#[cfg(test)]
mod tests;
mod transcript;
mod views;

pub use capture::*;
pub use command::*;
pub use computer::*;
pub use credentials::*;
pub use payloads::*;
pub use persona::*;
pub use remote::*;
pub use replies::*;
pub use session::*;
pub use transcript::*;
pub use views::*;
