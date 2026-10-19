//! What Hotline keeps on disk and what it rebuilds from it: the log of
//! append-only streams and the tapes and threads in it, the files teammates
//! send the person, the line a thread leaves on its parent, and, beside the
//! tape, the search index over every tape and two views of one tape — its
//! chapters and its last line. The index and the views are not records; every
//! one is rebuilt from the log.
//!
//! A crate of its own so that the core's build does not recompile the store,
//! and the store cannot reach the room, the sessions or the wire: it stands on
//! `hotline-contract` and `hotline-support` and must never depend on another
//! Hotline crate. `hotline-core` re-exports `log` and `sent` under their old
//! names and this crate as `store`, so every `crate::log` and
//! `crate::store::search` there still holds.

pub mod chapters;
pub mod link;
pub mod log;
pub mod previews;
pub mod search;
pub mod sent;

#[cfg(test)]
mod tests;
