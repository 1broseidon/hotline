//! The utilities Hotline stands on that know nothing of its state: the
//! records secrets are kept behind, the data directory's layout, the lock that
//! keeps a second desk off it, the desk's own log file, images and their
//! thumbnails, the fence a quoted body is wrapped in, the private directory a
//! credential lives in, and the Windows job a command's processes are held in.
//!
//! A crate of its own so that the crates above it can stand on these without
//! the core: it must never depend on another Hotline crate, not even
//! `hotline-contract`. `hotline-core` re-exports each module under its old
//! name, so every `crate::paths` there still holds.

pub mod credentials;
pub mod fence;
pub mod images;
pub mod log_file;
pub mod paths;
pub mod private_dir;
#[cfg(windows)]
pub mod process_windows;
pub mod room_lock;
pub mod thumbs;
