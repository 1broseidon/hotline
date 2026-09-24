//! One desk per room.
//!
//! A second launch while Hotline runs — the dock, the Start menu, a launcher
//! clicked twice, `open -n` — brings the running window forward and leaves,
//! rather than opening the same room again. Two desks on one data directory
//! would both fire its schedules, both answer its phones and both write its
//! streams.
//!
//! The guard is an exclusive lock on `desk.lock` in the data directory, taken
//! before the room opens and held for the life of the process. The system
//! lets go of it however the process ends, so a crash never leaves it behind,
//! and nothing the desk starts inherits it. Beside it, `desk.wake` names a
//! loopback port where the running desk listens for one word, `show`, and
//! answers `shown` once its window is up.
//!
//! The lock is the guard; the wake is a courtesy on top of it. A held lock
//! whose holder does not answer is waited on, because a restart after an
//! update starts the new desk while the old one is still leaving, and a desk
//! that is still opening answers only once its window exists. A holder that
//! neither answers nor lets go within the wait keeps the room, and this
//! launch ends.
//!
//! The lock is the room's, not the app's: a desk on another data directory
//! (`HOTLINE_DATA_DIR`) is another room and runs beside this one. A
//! development build takes no lock at all, so a checkout runs next to the
//! installed app.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a launch waits on a desk that holds the room without answering:
/// longer than a restart takes to let go, and than a desk takes to open.
const PATIENCE: Duration = Duration::from_secs(20);
/// How long one `show` waits for `shown`.
const ANSWER: Duration = Duration::from_secs(2);
/// Between one look at the lock and the next.
const AGAIN: Duration = Duration::from_millis(200);

const LOCK: &str = "desk.lock";
const WAKE: &str = "desk.wake";

pub(crate) enum Claim {
    /// This process holds the room.
    Held(Instance),
    /// Another desk holds it, and either showed its window or did not let go.
    Elsewhere,
}

/// The room's lock, and where the next launch can find this desk.
pub(crate) struct Instance {
    lock: File,
    wake: Option<TcpListener>,
}

/// Takes the room's lock, or wakes the desk that has it.
pub(crate) fn claim(root: &Path) -> io::Result<Claim> {
    claim_within(root, PATIENCE)
}

fn claim_within(root: &Path, patience: Duration) -> io::Result<Claim> {
    fs::create_dir_all(root)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(LOCK))?;
    let deadline = Instant::now() + patience;
    loop {
        match lock.try_lock() {
            Ok(()) => {
                let wake = listen(root);
                return Ok(Claim::Held(Instance { lock, wake }));
            }
            Err(TryLockError::WouldBlock) => {
                if wake(root) || Instant::now() >= deadline {
                    return Ok(Claim::Elsewhere);
                }
                std::thread::sleep(AGAIN);
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
}

/// A loopback port for the next launch to knock on, written where it will
/// look. Without one the lock still holds, and a later launch waits out its
/// patience and ends instead of showing this window.
fn listen(root: &Path) -> Option<TcpListener> {
    let path = root.join(WAKE);
    let written = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).and_then(|listener| {
        let staged = root.join(format!("{WAKE}.{}", std::process::id()));
        fs::write(&staged, listener.local_addr()?.port().to_string())?;
        fs::rename(&staged, &path)?;
        Ok(listener)
    });
    match written {
        Ok(listener) => Some(listener),
        Err(error) => {
            // A port from an earlier desk would send the knock to whatever
            // holds that port now.
            let _ = fs::remove_file(&path);
            eprintln!("[instance] a second launch cannot reach this window: {error}");
            None
        }
    }
}

/// Asks the desk that holds the room to show its window. True only when it
/// says it did.
fn wake(root: &Path) -> bool {
    let Some(port) = fs::read_to_string(root.join(WAKE))
        .ok()
        .and_then(|text| text.trim().parse::<u16>().ok())
    else {
        return false;
    };
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, ANSWER) else {
        return false;
    };
    if stream.set_read_timeout(Some(ANSWER)).is_err() || stream.write_all(b"show\n").is_err() {
        return false;
    }
    let mut answer = String::new();
    BufReader::new((&stream).take(16))
        .read_line(&mut answer)
        .is_ok()
        && answer.trim() == "shown"
}

impl Instance {
    /// Holds the room for the rest of the process and answers every later
    /// launch: `show` is asked to bring the window forward, and `shown` goes
    /// back only when it says it did — so a desk that is leaving, whose
    /// window will never come, sends the knock back to wait for its lock.
    pub(crate) fn answer(self, show: impl Fn() -> bool + Send + 'static) {
        let Instance { lock, wake } = self;
        std::thread::spawn(move || {
            let _lock = lock;
            let Some(wake) = wake else {
                loop {
                    std::thread::park();
                }
            };
            for stream in wake.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let _ = stream.set_read_timeout(Some(ANSWER));
                let mut asked = String::new();
                if BufReader::new((&stream).take(16))
                    .read_line(&mut asked)
                    .is_err()
                    || asked.trim() != "show"
                {
                    continue;
                }
                if show() {
                    let _ = (&stream).write_all(b"shown\n");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn held(claim: io::Result<Claim>) -> Instance {
        match claim.unwrap() {
            Claim::Held(instance) => instance,
            Claim::Elsewhere => panic!("the room was not free"),
        }
    }

    #[test]
    fn a_second_launch_shows_the_running_window_and_leaves() {
        let root = tempfile::tempdir().unwrap();
        // A port left by a desk that is gone is never knocked on: the lock is
        // free, so the room is taken without looking.
        fs::write(root.path().join(WAKE), "1").unwrap();
        let first = held(claim_within(root.path(), Duration::from_secs(5)));
        let shown = Arc::new(AtomicUsize::new(0));
        first.answer({
            let shown = shown.clone();
            move || {
                shown.fetch_add(1, Ordering::SeqCst);
                true
            }
        });

        let second = claim_within(root.path(), Duration::from_secs(5)).unwrap();
        assert!(matches!(second, Claim::Elsewhere));
        assert_eq!(shown.load(Ordering::SeqCst), 1);
    }

    /// A restart: the old desk still holds the room for a moment, and cannot
    /// show a window it is closing. The new one waits for the lock instead of
    /// leaving, which would end Hotline altogether.
    #[test]
    fn a_desk_that_is_leaving_is_waited_for_not_taken_for_a_running_one() {
        let root = tempfile::tempdir().unwrap();
        let leaving = held(claim_within(root.path(), Duration::from_secs(5)));
        let started = Instant::now();
        let gone = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(leaving);
        });

        let next = claim_within(root.path(), Duration::from_secs(10));
        assert!(matches!(next, Ok(Claim::Held(_))));
        assert!(started.elapsed() >= Duration::from_millis(300));
        gone.join().unwrap();
    }

    /// A desk that neither answers nor lets go still keeps its room.
    #[test]
    fn a_desk_that_never_answers_keeps_the_room() {
        let root = tempfile::tempdir().unwrap();
        let silent = held(claim_within(root.path(), Duration::from_secs(5)));
        silent.answer(|| false);

        let started = Instant::now();
        let second = claim_within(root.path(), Duration::from_millis(500)).unwrap();
        assert!(matches!(second, Claim::Elsewhere));
        assert!(started.elapsed() >= Duration::from_millis(500));
    }

    /// Another data directory is another room.
    #[test]
    fn a_desk_on_another_data_directory_runs_beside_this_one() {
        let one = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let _first = held(claim_within(one.path(), Duration::from_secs(5)));
        let _second = held(claim_within(other.path(), Duration::from_secs(5)));
    }
}
