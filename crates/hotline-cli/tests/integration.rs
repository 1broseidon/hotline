//! One link for the harnesses that can safely share a process.
//! Explicit paths preserve the existing nested modules and fixture paths.

#[cfg(unix)]
mod common;

#[path = "pair.rs"]
mod pair;
#[path = "relay.rs"]
mod relay;
#[path = "serve.rs"]
mod serve;
#[path = "update.rs"]
mod update;
