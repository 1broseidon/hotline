//! Talking to the desk: hearing a clip, speaking a sentence, and paying for
//! both. Every provider here is one the owner has already connected; voice
//! never asks for a key of its own.

pub mod ledger;
pub mod settings;
pub mod speech;
