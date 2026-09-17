//! Beside the tape: the search index over every tape, and two views of one
//! tape — its chapters and its last line. None of them is a record; every one
//! is rebuilt from the log.

pub mod chapters;
pub mod previews;
pub mod search;
