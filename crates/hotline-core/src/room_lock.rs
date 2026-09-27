//! One writer per room.
//!
//! Two desks on one data directory would both fire its schedules, both answer
//! its phones and both write its streams. Every process that writes a room —
//! the desktop app, `hotline serve`, `hotline-import` — takes this lock before
//! it opens anything, and holds it for the life of the process. The lock is an
//! exclusive lock on `desk.lock` in the data directory: the system lets go of
//! it however the process ends, so a crash never leaves it behind, and the
//! file itself is never removed, so a desk that is still leaving keeps its
//! room until it has left.
//!
//! There is no build or platform that skips it. A file system that cannot
//! lock is an error, not an unguarded room.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;

pub const LOCK_FILE: &str = "desk.lock";

/// Held for as long as this value lives.
#[derive(Debug)]
pub struct RoomLock {
    _file: File,
}

impl RoomLock {
    /// Takes the room, or `None` when another process holds it.
    pub fn try_take(root: &Path) -> io::Result<Option<RoomLock>> {
        fs::create_dir_all(root)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(LOCK_FILE))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(RoomLock { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(io::Error::new(
                error.kind(),
                format!(
                    "{} could not be locked, and Hotline does not open a room it cannot keep to itself: {error}",
                    root.display()
                ),
            )),
        }
    }

    /// Takes the room, or says which room is already open elsewhere.
    pub fn take(root: &Path) -> io::Result<RoomLock> {
        Self::try_take(root)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "Another Hotline desk has {} open. Stop it first; two desks on one room would both run its work.",
                    root.display()
                ),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_room_has_one_writer_until_it_lets_go() {
        let root = tempfile::tempdir().unwrap();
        let first = RoomLock::take(root.path()).unwrap();
        assert!(RoomLock::try_take(root.path()).unwrap().is_none());
        let refused = RoomLock::take(root.path()).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::WouldBlock);
        assert!(refused.to_string().contains("Another Hotline desk"));
        drop(first);
        assert!(RoomLock::try_take(root.path()).unwrap().is_some());
        // The file stays: a leaving desk's lock is never unlinked from under it.
        assert!(root.path().join(LOCK_FILE).exists());
    }

    #[test]
    fn another_room_is_another_lock() {
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        let _one = RoomLock::take(one.path()).unwrap();
        assert!(RoomLock::take(two.path()).is_ok());
    }
}
