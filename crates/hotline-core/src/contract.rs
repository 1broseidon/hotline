//! The room's contract types: what the window is handed when it asks about a
//! teammate, and what the room pushes at it unasked.
//!
//! These are defined once, in the files below, one per concern, and re-exported
//! here so every `crate::contract::X` path holds. The TypeScript is generated
//! from them:
//! `cargo test -p hotline-core` runs the tests `#[ts(export)]` writes and leaves
//! `ui/src/generated/contract.ts` behind, which the window
//! re-exports rather than spelling the same shapes a second time. The
//! workspace's `.cargo/config.toml` is what points ts-rs at that directory
//! and tells it a 64-bit integer is a JavaScript `number`; run `cargo test`
//! and check the regenerated file in with the change that moved it.
//!
//! Every attribute here serves one invariant: the JSON serde emits must be
//! what the Bun main emits for the same value, field for field. That is why
//! an optional field carries `skip_serializing_if` — absent means ABSENT, and
//! a `null` where Bun wrote nothing is a different value to the window — and
//! why the store's tests (`store/tests.rs`) feed its own output through
//! these types and back out again.

mod command;
mod computer;
mod credentials;
mod payloads;
mod persona;
mod replies;
mod session;
#[cfg(test)]
mod tests;
mod transcript;
mod views;

pub use command::*;
pub use computer::*;
pub use credentials::*;
pub use payloads::*;
pub use persona::*;
pub use replies::*;
pub use session::*;
pub use transcript::*;
pub use views::*;
