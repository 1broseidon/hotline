//! Where the desktop app's own lines go when nobody is reading its output.
//!
//! An app started from the Finder, the Start menu or a desktop launcher has
//! its stdout and stderr on `/dev/null`, or on Windows no console at all, so
//! the `[startup]` lines, the reason an agent would not start and a panic's
//! message were all lost. [`redirect`] points both at `logs/hotline.log` in
//! the data directory, and [`keep_capped`] holds it to [`CAP`] bytes, keeping
//! one previous file as `hotline.log.1`.
//!
//! The process's own descriptors are redirected rather than a logger
//! installed, because everything already prints with `eprintln!` and the panic
//! hook writes to stderr: one change catches all of it, nothing is buffered in
//! the process, and a line printed just before a crash is on disk. Nothing is
//! added to what is printed, and what is printed is already written without
//! keys or tokens in it.
//!
//! A process whose stderr is a terminal is left alone: whoever started it is
//! reading it there, and `make dev` stays as it was.

use crate::paths;
use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How large the log may grow before it is moved aside. Two files of this
/// size are a long way back for any one problem, and nothing a person would
/// notice on their disk.
pub const CAP: u64 = 5 * 1024 * 1024;

/// How often a running app looks at the log's size. It writes a few lines an
/// hour when all is well; a loop printing in earnest still cannot get far past
/// the cap between two looks.
const CHECK_EVERY: Duration = Duration::from_secs(5 * 60);

/// The file a person is asked for when something went wrong.
pub fn path(root: &Path) -> PathBuf {
    paths::logs_dir(root).join("hotline.log")
}

fn previous(root: &Path) -> PathBuf {
    paths::logs_dir(root).join("hotline.log.1")
}

/// Sends this process's stdout and stderr to the log, unless stderr is a
/// terminal. Answers whether it did.
///
/// Starts no thread, so it can run before anything that must run while the
/// process has only one (the app's shell PATH, which sets environment
/// variables, runs after this so its `[startup]` lines are kept).
pub fn redirect(root: &Path) -> io::Result<bool> {
    if io::stderr().is_terminal() {
        return Ok(false);
    }
    std::fs::create_dir_all(paths::logs_dir(root))?;
    rotate_if_full(root, CAP)?;
    point_at(open(root)?)?;
    Ok(true)
}

/// Moves the log aside whenever it reaches [`CAP`], for as long as the
/// process runs. Call it once [`redirect`] has, and once starting a thread is
/// allowed.
pub fn keep_capped(root: &Path) -> io::Result<()> {
    let root = root.to_path_buf();
    std::thread::Builder::new()
        .name("hotline-log".into())
        .spawn(move || {
            loop {
                std::thread::sleep(CHECK_EVERY);
                let rotated = rotate_if_full(&root, CAP).and_then(|rotated| {
                    if rotated {
                        point_at(open(&root)?)
                    } else {
                        Ok(())
                    }
                });
                if let Err(error) = rotated {
                    eprintln!(
                        "[log] {} could not be rotated: {error}",
                        path(&root).display()
                    );
                }
            }
        })
        .map(drop)
}

/// Moves `hotline.log` to `hotline.log.1`, replacing the one there, once it
/// holds `cap` bytes or more. Answers whether it moved it.
pub fn rotate_if_full(root: &Path, cap: u64) -> io::Result<bool> {
    match std::fs::metadata(path(root)) {
        Ok(metadata) if metadata.len() >= cap => {
            std::fs::rename(path(root), previous(root))?;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn open(root: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path(root))
}

/// Makes `file` the process's stdout and stderr. Each swap is atomic, so a
/// line being written while the log is rotated lands whole in one file or the
/// other.
#[cfg(unix)]
fn point_at(file: File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    for target in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        // Safety: both descriptors are valid; dup2 replaces the target in place.
        if unsafe { libc::dup2(file.as_raw_fd(), target) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    // The file's own descriptor closes here; stdout and stderr keep it open.
    Ok(())
}

/// Makes `file` the process's stdout and stderr. Rust looks the standard
/// handles up on every write, so the swap takes effect at once. The handle is
/// never closed: a thread may be writing to it as it is replaced, and a closed
/// handle's number can be handed to another file, which that write would then
/// land in. One handle per rotation is what that costs.
#[cfg(windows)]
fn point_at(file: File) -> io::Result<()> {
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};
    let handle = file.into_raw_handle();
    for target in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // Safety: the handle is a file this process opened and never closes.
        if unsafe { SetStdHandle(target, handle) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("hotline-log-file-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(paths::logs_dir(&root)).unwrap();
        root
    }

    #[test]
    fn a_log_under_the_cap_stays_where_it_is() {
        let root = scratch("under");
        std::fs::write(path(&root), "short").unwrap();
        assert!(!rotate_if_full(&root, 6).unwrap());
        assert_eq!(std::fs::read_to_string(path(&root)).unwrap(), "short");
        assert!(!previous(&root).exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_full_log_becomes_the_previous_one_and_only_one_is_kept() {
        let root = scratch("full");
        std::fs::write(previous(&root), "the oldest").unwrap();
        std::fs::write(path(&root), "full up").unwrap();
        assert!(rotate_if_full(&root, 7).unwrap());
        assert!(!path(&root).exists());
        assert_eq!(std::fs::read_to_string(previous(&root)).unwrap(), "full up");
        let kept = std::fs::read_dir(paths::logs_dir(&root)).unwrap().count();
        assert_eq!(kept, 1, "the oldest is gone, not renamed again");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn no_log_yet_is_nothing_to_rotate() {
        let root = scratch("none");
        assert!(!rotate_if_full(&root, 1).unwrap());
        let _ = std::fs::remove_dir_all(root);
    }
}
