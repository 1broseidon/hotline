//! The DevTools protocol over a pipe, which is how the desk asks a throwaway
//! headless Chromium for its own cookies.
//!
//! The browser is launched with [`PIPE_ARG`]: it reads commands from its file
//! descriptor 3 and writes answers to its descriptor 4, each message one JSON
//! text ended by a NUL byte. Nothing listens on a port, so no other process on
//! the machine (another user, or a teammate's shell that can reach loopback)
//! can connect to the instance while it holds the person's decrypted cookies:
//! only the desk holds the other ends of the two pipes.

use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::pipe;
use tokio::process::{Child, Command};

/// Chromium's switch for DevTools over descriptors 3 and 4 instead of a port.
pub(super) const PIPE_ARG: &str = "--remote-debugging-pipe";

/// The most one message from the browser may hold. A whole cookie jar is a
/// few megabytes; past this something is wrong and the read stops rather than
/// growing without end.
const MOST: usize = 64 * 1024 * 1024;

/// One message as the pipe carries it: the JSON text, then a NUL. JSON writes
/// a NUL inside a string as `\u0000`, so the text itself never holds one.
pub(super) fn encode(message: &Value) -> Vec<u8> {
    let mut bytes = message.to_string().into_bytes();
    bytes.push(0);
    bytes
}

/// Splits what the browser writes back into whole messages, however the
/// reads happen to cut it: a message may arrive across several reads, and
/// one read may end several.
#[derive(Default)]
pub(super) struct Frames {
    pending: Vec<u8>,
}

impl Frames {
    /// The messages `bytes` completes, without their NULs, in order. What is
    /// left of an unfinished one is kept for the next read.
    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        let mut done = Vec::new();
        let mut rest = bytes;
        while let Some(end) = rest.iter().position(|byte| *byte == 0) {
            self.pending.extend_from_slice(&rest[..end]);
            done.push(std::mem::take(&mut self.pending));
            rest = &rest[end + 1..];
        }
        self.pending.extend_from_slice(rest);
        if self.pending.len() > MOST {
            self.pending = Vec::new();
            return Err("The browser's answer was too large.".into());
        }
        Ok(done)
    }
}

/// The desk's ends of the browser's two pipes.
pub(super) struct Pipe {
    commands: pipe::Sender,
    answers: pipe::Receiver,
    frames: Frames,
    ready: VecDeque<Vec<u8>>,
    last_id: u64,
}

/// Spawns `command` with descriptor 3 reading what the desk writes and
/// descriptor 4 writing what the desk reads, and returns the desk's ends.
pub(super) fn spawn(command: &mut Command) -> io::Result<(Child, Pipe)> {
    // Both pairs are made close-on-exec (atomically on Linux; on macOS just
    // after, as for every pipe std makes), so neither end leaks into anything
    // else the desk starts; only the copies made at 3 and 4 below survive
    // into the browser.
    let (child_reads, commands) = io::pipe()?;
    let (answers, child_writes) = io::pipe()?;
    let (read, write) = (child_reads.as_raw_fd(), child_writes.as_raw_fd());
    // SAFETY: the closure runs in the forked child before exec. It calls only
    // `fcntl` and `dup2`, which are async-signal-safe, allocates nothing and
    // takes no lock. The two descriptors it names are open in the parent
    // until `spawn` returns, because `child_reads` and `child_writes` are
    // held until after it, so they are open in the child too.
    unsafe {
        command.pre_exec(move || onto_three_and_four(read, write));
    }
    let child = command.spawn()?;
    // The browser holds its own ends now. Closing ours here means its exit
    // reads as the end of the answers rather than a wait for nothing.
    drop((child_reads, child_writes));
    let pipe = Pipe {
        commands: pipe::Sender::from_owned_fd(OwnedFd::from(commands))?,
        answers: pipe::Receiver::from_owned_fd(OwnedFd::from(answers))?,
        frames: Frames::default(),
        ready: VecDeque::new(),
        last_id: 0,
    };
    Ok((child, pipe))
}

/// In the child: put `read` at descriptor 3 and `write` at 4. Each is first
/// copied to 5 or above, close-on-exec, so neither can be overwritten by the
/// other's move when one of them already sits at 3 or 4; `dup2` then clears
/// close-on-exec on the two the browser keeps.
fn onto_three_and_four(read: RawFd, write: RawFd) -> io::Result<()> {
    fn checked(result: libc::c_int) -> io::Result<libc::c_int> {
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(result)
        }
    }
    // SAFETY: `fcntl` with F_DUPFD_CLOEXEC and `dup2` take plain descriptor
    // numbers and touch no memory of ours; a bad descriptor is an error code.
    unsafe {
        let read = checked(libc::fcntl(read, libc::F_DUPFD_CLOEXEC, 5))?;
        let write = checked(libc::fcntl(write, libc::F_DUPFD_CLOEXEC, 5))?;
        checked(libc::dup2(read, 3))?;
        checked(libc::dup2(write, 4))?;
    }
    Ok(())
}

impl Pipe {
    /// Sends one command and waits for its answer: the `result`, or the
    /// browser's own `error` object as the inner `Err`. The outer `Err` is the
    /// pipe failing. Events and other answers in between are passed over.
    pub(super) async fn call(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Result<Value, Value>, String> {
        self.last_id += 1;
        let id = self.last_id;
        self.commands
            .write_all(&encode(
                &json!({"id": id, "method": method, "params": params}),
            ))
            .await
            .map_err(|error| format!("Could not ask the browser for its cookies: {error}"))?;
        let mut chunk = vec![0; 64 * 1024];
        loop {
            while let Some(frame) = self.ready.pop_front() {
                let Ok(reply) = serde_json::from_slice::<Value>(&frame) else {
                    continue;
                };
                if reply.get("id").and_then(Value::as_u64) != Some(id) {
                    continue;
                }
                if let Some(error) = reply.get("error") {
                    return Ok(Err(error.clone()));
                }
                return Ok(Ok(reply.get("result").cloned().unwrap_or(Value::Null)));
            }
            let read = self
                .answers
                .read(&mut chunk)
                .await
                .map_err(|error| format!("The browser's debugging pipe failed: {error}"))?;
            if read == 0 {
                return Err("The browser closed its debugging pipe before answering.".into());
            }
            self.ready.extend(self.frames.push(&chunk[..read])?);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pipe_message_is_its_json_ended_by_one_nul_and_reads_back_however_it_is_cut() {
        let first = json!({"id": 1, "method": "Storage.getCookies", "params": {"note": "a\u{0}b"}});
        let second = json!({"id": 2, "result": {"cookies": []}});
        let mut wire = encode(&first);
        // A NUL inside a string is escaped, so the only NUL ends the message.
        assert_eq!(wire.iter().filter(|byte| **byte == 0).count(), 1);
        assert_eq!(wire.last(), Some(&0));
        wire.extend(encode(&second));

        // Every way of cutting the two messages into two reads gives both back
        // whole and in order, including a cut just before or after a NUL.
        for cut in 0..=wire.len() {
            let mut frames = Frames::default();
            let mut got = frames.push(&wire[..cut]).unwrap();
            got.extend(frames.push(&wire[cut..]).unwrap());
            let got: Vec<Value> = got
                .iter()
                .map(|frame| serde_json::from_slice(frame).unwrap())
                .collect();
            assert_eq!(got, [first.clone(), second.clone()], "cut at {cut}");
        }

        // One byte at a time, and an empty read in between, change nothing.
        let mut frames = Frames::default();
        let mut got = Vec::new();
        for byte in &wire {
            got.extend(frames.push(&[]).unwrap());
            got.extend(frames.push(std::slice::from_ref(byte)).unwrap());
        }
        assert_eq!(got.len(), 2);
        assert_eq!(serde_json::from_slice::<Value>(&got[1]).unwrap(), second);
    }

    #[test]
    fn an_unended_answer_past_the_limit_stops_the_read() {
        let mut frames = Frames::default();
        let chunk = vec![b'x'; 1024 * 1024];
        let mut refused = None;
        for _ in 0..=MOST / chunk.len() {
            if let Err(error) = frames.push(&chunk) {
                refused = Some(error);
                break;
            }
        }
        assert_eq!(
            refused.as_deref(),
            Some("The browser's answer was too large.")
        );
    }
}
