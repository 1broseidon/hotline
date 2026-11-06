//! The browser's X display: an Xvfb of its own, nobody's screen. GTK draws
//! the teammates' pages there in windows that are never shown, which WebKit
//! lays out, draws, pictures and takes input in as in the app's hidden
//! pane (`linux.rs`).
//!
//! An X server lets any process that connects watch and type into every
//! window on it, and Xvfb listens on Linux's abstract socket namespace even
//! with `-nolisten unix`, where any user's process can reach it. So it is
//! started with `-auth` and a cookie of 16 random bytes in a file only this
//! user can read, in a directory only this user can open, and refuses every
//! client without it: GTK is handed the cookie through `XAUTHORITY`. It
//! picks a free display number itself (`-displayfd`), and dies with the
//! browser (`PR_SET_PDEATHSIG`).

use std::io::Read;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long Xvfb has to say its display number. It is ready in tens of
/// milliseconds.
const START_WAIT: Duration = Duration::from_secs(10);

/// The descriptor Xvfb writes its display number to.
const DISPLAY_FD: i32 = 3;

/// Xvfb on `PATH`, if it is installed.
pub fn xvfb() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("Xvfb"))
        .find(|candidate| candidate.is_file())
}

/// The running display: stopped with this.
pub struct Display {
    xvfb: Child,
    pub number: u32,
    /// The cookie files' directory, removed with this.
    _private: tempfile::TempDir,
}

impl Drop for Display {
    fn drop(&mut self) {
        let _ = self.xvfb.kill();
        let _ = self.xvfb.wait();
    }
}

/// One Xauthority entry for any host: the X cookie for display `number`, or
/// with no number, as the server reads its own `-auth` file.
fn entry(number: &str, cookie: &[u8]) -> Vec<u8> {
    let mut out = 0xffff_u16.to_be_bytes().to_vec();
    for part in [&b""[..], number.as_bytes(), b"MIT-MAGIC-COOKIE-1", cookie] {
        let length = u16::try_from(part.len()).expect("each part is a few bytes");
        out.extend(length.to_be_bytes());
        out.extend(part);
    }
    out
}

/// Writes `bytes` to a new file only this user can read.
fn private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .map_err(|error| format!("the browser's display cookie could not be written: {error}"))
}

/// Starts Xvfb on a free display with a fresh cookie, and points GTK at it.
/// Called while this process has one thread: it sets the environment.
pub fn start() -> Result<Display, String> {
    let program = xvfb().ok_or("Xvfb is not installed (the xvfb package)")?;
    let private = tempfile::Builder::new()
        .prefix("hotline-browser-")
        .tempdir()
        .map_err(|error| format!("the browser's display could not get a folder: {error}"))?;
    let mut cookie = [0u8; 16];
    getrandom::fill(&mut cookie).map_err(|error| format!("no cookie could be made: {error}"))?;
    let server_auth = private.path().join("server");
    private_file(&server_auth, &entry("", &cookie))?;

    let mut ends = [0; 2];
    // SAFETY: a pipe into two descriptors this function then owns.
    if unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(format!(
            "the browser's display could not start: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: fresh descriptors from the pipe above, owned here alone.
    let (read_end, write_end) =
        unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) };
    let write_fd = std::os::fd::AsRawFd::as_raw_fd(&write_end);
    let mut command = Command::new(program);
    command
        .args(["-displayfd", &DISPLAY_FD.to_string(), "-auth"])
        .arg(&server_auth)
        .args(["-nolisten", "tcp", "-nolisten", "unix", "-noreset"])
        .args(["-screen", "0", "1920x1200x24"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(write_fd, DISPLAY_FD) < 0
                || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut xvfb = command
        .spawn()
        .map_err(|error| format!("Xvfb did not start: {error}"))?;
    drop(write_end);
    let number = match read_number(read_end) {
        Ok(number) => number,
        Err(error) => {
            let _ = xvfb.kill();
            let _ = xvfb.wait();
            return Err(error);
        }
    };
    let client_auth = private.path().join("client");
    private_file(&client_auth, &entry(&number.to_string(), &cookie))?;
    // SAFETY: this process has one thread yet: nothing reads the
    // environment while it changes.
    unsafe {
        std::env::set_var("DISPLAY", format!(":{number}"));
        std::env::set_var("XAUTHORITY", &client_auth);
        std::env::set_var("GDK_BACKEND", "x11");
        // No accessibility bus: a server has no session bus to find it on.
        std::env::set_var("NO_AT_BRIDGE", "1");
        std::env::remove_var("WAYLAND_DISPLAY");
    }
    Ok(Display {
        xvfb,
        number,
        _private: private,
    })
}

/// The display number Xvfb writes, a line, once it listens.
fn read_number(read_end: OwnedFd) -> Result<u32, String> {
    let mut pipe = std::fs::File::from(read_end);
    let deadline = Instant::now() + START_WAIT;
    let mut said = Vec::new();
    let mut byte = [0u8; 1];
    while !said.ends_with(b"\n") {
        let left = deadline.saturating_duration_since(Instant::now());
        let mut ready = libc::pollfd {
            fd: std::os::fd::AsRawFd::as_raw_fd(&pipe),
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = libc::c_int::try_from(left.as_millis()).unwrap_or(libc::c_int::MAX);
        // SAFETY: one descriptor, owned by `pipe`.
        if unsafe { libc::poll(&raw mut ready, 1, millis) } <= 0 {
            return Err("Xvfb did not start in time".into());
        }
        match pipe.read(&mut byte) {
            Ok(1) => said.push(byte[0]),
            _ => return Err("Xvfb stopped as it started".into()),
        }
    }
    String::from_utf8_lossy(&said)
        .trim()
        .parse()
        .map_err(|_| "Xvfb said no display number".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_xauthority_entry_is_any_hosts_cookie_for_its_display() {
        let cookie = [7u8; 16];
        let made = entry("12", &cookie);
        let mut expected = vec![0xff, 0xff, 0, 0, 0, 2, b'1', b'2', 0, 18];
        expected.extend(b"MIT-MAGIC-COOKIE-1");
        expected.extend([0, 16]);
        expected.extend(cookie);
        assert_eq!(made, expected);
        assert_eq!(
            entry("", &cookie)[4..6],
            [0, 0],
            "the server's names no display"
        );
    }
}
