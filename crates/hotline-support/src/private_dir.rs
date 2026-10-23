//! The directory a credential lives in: private to this user from the first
//! instant, and, for an agent account's folder, checked to still be a real
//! directory where Hotline made it before anything is read from or written to it.

use crate::{credentials, paths};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Creates a credential directory private to this user, and says so again on a
/// directory that was already there — a boundary is only a boundary if it is
/// re-established every time.
#[cfg(unix)]
pub fn make_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    // `mkdir` masks the mode it is given by the process umask, which can only
    // take permissions away, so the directory is never wider than 0700 — not
    // even for the instant between creating it and setting the mode again.
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    // chmod on the path follows a symlink. The fd does not, so a directory
    // swapped for a link cannot have its target's mode changed through us.
    open_directory(path)?.set_permissions(fs::Permissions::from_mode(0o700))
}

/// Opens a directory without following a symlink at the last component.
#[cfg(unix)]
pub fn open_directory(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

/// Creates a credential directory with a protected DACL from the first
/// instant, and repairs one that was already there. Files made in it inherit
/// that DACL, which is all the privacy a file has on this platform.
#[cfg(windows)]
pub fn make_private_directory(path: &Path) -> io::Result<()> {
    credentials::windows::private_directory(path)
}

/// One agent account's folder under `root`, as the real directory Hotline made,
/// and its canonical path, which is the one to use from here on.
///
/// Run before every spawn, status, login and setup write: the folder can be
/// swapped for a link between one use and the next, and a swapped folder
/// would send an account's sign-in somewhere else. A `root` that is
/// itself reached through a link, or a Windows 8.3 short name, is resolved
/// first, so that spelling alone never refuses the legitimate folder.
///
/// `InvalidInput` for an id that is not exactly the uuid Hotline writes,
/// `NotFound` for a folder or root that is not there, `NotADirectory` for a
/// file in its place, `PermissionDenied` for a link in its place or one
/// standing where `agent-accounts` should be.
pub fn checked_account_dir(root: &Path, id: &str) -> io::Result<PathBuf> {
    let not_an_id = || io::Error::new(io::ErrorKind::InvalidInput, "Not an account id.");
    let dir = paths::agent_account_dir(root, id).ok_or_else(not_an_id)?;
    let expected =
        paths::agent_account_dir(&dunce::canonicalize(root)?, id).ok_or_else(not_an_id)?;
    real_directory(&dir, &expected)
}

/// `path` as a real directory (no symlink, junction or other reparse point at
/// the last component) whose canonical path is exactly `expected`. `expected`
/// is not canonicalized here: a caller hands in where the folder must be, and
/// a link on the way to it, such as one at `agent-accounts`, resolves
/// somewhere else and is refused.
fn real_directory(path: &Path, expected: &Path) -> io::Result<PathBuf> {
    credentials::check_private_path(path)?;
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "The account folder is not a folder.",
        ));
    }
    let canonical = dunce::canonicalize(path)?;
    if canonical != expected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "The folder no longer resolves to where Hotline made it.",
        ));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests;
