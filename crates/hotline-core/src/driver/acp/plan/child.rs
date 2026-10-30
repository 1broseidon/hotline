//! A harness's CLI started for one question about its login and ended after
//! it: Claude Code in stream-JSON mode, `codex app-server`. Nothing a
//! teammate runs is touched; the child is the desk's own, in a scratch
//! folder, in a process group of its own, and is ended by the id it was
//! spawned with.
//!
//! It speaks one JSON value per line both ways. When the conversation is
//! over its stdin is closed, which is how both CLIs are told to exit; one
//! still running after a short grace is killed, its whole group with it.

#[cfg(unix)]
use super::super::{GROUPS, lock};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};

/// How long a child has to exit by itself once its stdin is closed.
const GRACE: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(2)
};

/// A line longer than this is not an answer worth reading: a CLI that
/// writes one is not the one the desk thinks it is.
const LINE_LIMIT: usize = 4 * 1024 * 1024;

pub(super) struct Probe {
    child: Child,
    /// The id it was spawned with, which is also its group's.
    id: Option<u32>,
    stdin: Option<ChildStdin>,
    lines: Lines<BufReader<ChildStdout>>,
    /// Whether it has been reaped, after which its id is nobody's to kill.
    reaped: bool,
}

impl Probe {
    /// Starts `binary` with `args` in `cwd`, with the desk's environment,
    /// then the teammate's launch environment, then `extra`, over it.
    pub(super) fn spawn(
        binary: &Path,
        args: &[&str],
        env: &[(String, String)],
        extra: &[(&str, &str)],
        cwd: &Path,
    ) -> std::io::Result<Self> {
        std::fs::create_dir_all(cwd)?;
        let mut command = tokio::process::Command::new(binary);
        command
            .args(args)
            .envs(env.iter().cloned())
            .envs(extra.iter().copied())
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        crate::process_windows::prepare(&mut command);
        let mut child = command.spawn()?;
        let id = child.id();
        #[cfg(unix)]
        if let Some(id) = id {
            lock(&GROUPS).push(id as libc::pid_t);
        }
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            kill_group(id);
            let _ = child.start_kill();
            return Err(std::io::Error::other("the CLI gave no pipes"));
        };
        Ok(Self {
            child,
            id,
            stdin: Some(stdin),
            lines: BufReader::new(stdout).lines(),
            reaped: false,
        })
    }

    /// Writes one value as one line.
    pub(super) async fn send(&mut self, message: &Value) -> std::io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| std::io::Error::other("stdin is closed"))?;
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        stdin.write_all(&line).await?;
        stdin.flush().await
    }

    /// Reads lines until one `pick` takes, and gives what it took. `None`
    /// when the child closes its stdout first.
    pub(super) async fn answer<T>(
        &mut self,
        mut pick: impl FnMut(&Value) -> Option<T>,
    ) -> Option<T> {
        while let Ok(Some(line)) = self.lines.next_line().await {
            if line.len() > LINE_LIMIT {
                return None;
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(taken) = pick(&value) {
                return Some(taken);
            }
        }
        None
    }

    /// Closes stdin, gives the child a moment to exit by itself, then kills
    /// its group by the id it was spawned with, and reaps it.
    pub(super) async fn end(mut self) {
        drop(self.stdin.take());
        if tokio::time::timeout(GRACE, self.child.wait())
            .await
            .is_err()
        {
            kill_group(self.id);
            let _ = self.child.start_kill();
            let _ = self.child.wait().await;
        }
        self.reaped = true;
        forget(self.id);
    }
}

/// A probe dropped without [`Probe::end`], by a caller that gave up on it:
/// its group goes with it.
impl Drop for Probe {
    fn drop(&mut self) {
        if !self.reaped {
            kill_group(self.id);
        }
    }
}

/// Kills a child's whole group by the id it was spawned with, and stops
/// counting it among the live ones. Only for a child not yet reaped.
fn kill_group(id: Option<u32>) {
    #[cfg(unix)]
    if let Some(id) = id {
        forget(Some(id));
        // Safety: the group is the one this probe made for its child, which
        // has not been reaped, so the id is still its.
        unsafe { libc::killpg(id as libc::pid_t, libc::SIGKILL) };
    }
    #[cfg(windows)]
    let _ = id;
}

/// Stops counting a group among the live ones.
fn forget(id: Option<u32>) {
    #[cfg(unix)]
    if let Some(id) = id {
        lock(&GROUPS).retain(|live| *live != id as libc::pid_t);
    }
    #[cfg(windows)]
    let _ = id;
}
