//! The shell command, run in the teammate's working directory.
//!
//! Nothing here is on a clock. An agent that is building, testing or watching
//! is doing its job, and a command killed at two minutes taught the previous
//! Toad's teammates to avoid the work rather than to finish it. The human's
//! Stop is the cap: cancelling the turn drops the future this call is running
//! in, and the guard on the way out kills the whole process group — so the
//! child's own children go with it, which is the difference between stopping a
//! build and orphaning its compiler. An agent that wants its own deadline
//! passes `timeout_seconds`.

use super::{ToolError, Workspace};
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// How much of the output the model is shown. Big enough for a build log; the
/// middle of anything larger is cut, because the head says what was run and
/// the tail says how it ended.
const OUTPUT_LIMIT: usize = 256 * 1024;

#[derive(Deserialize)]
pub struct RunCommandArgs {
    command: String,
    /// A deadline the agent set for itself. Absent means none.
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

/// A shell command, run in the teammate's working directory.
pub struct RunCommand {
    workspace: Workspace,
}

impl RunCommand {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for RunCommand {
    const NAME: &'static str = "shell";
    type Error = ToolError;
    type Args = RunCommandArgs;
    type Output = String;

    fn description(&self) -> String {
        "Run a shell command in the working directory and return its output. There is no time limit: a build, a test run or a long install can take as long as it takes."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command line to run, as it would be typed at a shell prompt."
                },
                "timeout_seconds": {
                    "type": "integer",
                    "description": "Give up after this many seconds. Omit it unless you want a deadline; commands are not otherwise timed."
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let command = args.command.trim().to_string();
        if command.is_empty() {
            return Err(ToolError::new("A command is required."));
        }
        let mut process = if cfg!(target_os = "windows") {
            let mut process = Command::new("cmd");
            process.args(["/C", &command]);
            process
        } else {
            let mut process = Command::new("sh");
            process.args(["-c", &command]);
            process
        };
        process
            .current_dir(self.workspace.display_root())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Windows has no process group to put it in, so the child itself
            // is what a drop can kill there.
            .kill_on_drop(true);
        #[cfg(unix)]
        process.process_group(0);

        let child = process
            .spawn()
            .map_err(|error| ToolError::other(format!("The command could not start: {error}")))?;
        let mut group = ProcessGroup::of(&child);
        let waiting = child.wait_with_output();
        let output = match args.timeout_seconds {
            Some(seconds) => tokio::time::timeout(Duration::from_secs(seconds), waiting)
                .await
                .map_err(|_| {
                    ToolError::other(format!(
                        "The command did not finish within {seconds} seconds."
                    ))
                })?,
            None => waiting.await,
        }
        .map_err(|error| ToolError::other(format!("The command could not be run: {error}")))?;
        group.finished();

        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        let errors = String::from_utf8_lossy(&output.stderr);
        if !errors.trim().is_empty() {
            text.push_str("\n[stderr]\n");
            text.push_str(&errors);
        }
        let mut text = elide(&text, OUTPUT_LIMIT);
        let status = output.status.code().unwrap_or(-1);
        if status != 0 {
            text.push_str(&format!("\n[exit status {status}]"));
        }
        Ok(text)
    }
}

/// The child's process group, killed when this is dropped.
///
/// The command is spawned into a group of its own, so this reaches everything
/// it started and not this process. Once the command has been waited for its
/// group is gone, and killing then could only reach whatever the operating
/// system next gave that number to — so a finished command releases the guard.
struct ProcessGroup {
    #[cfg_attr(not(unix), allow(dead_code))]
    id: Option<u32>,
}

impl ProcessGroup {
    fn of(child: &tokio::process::Child) -> Self {
        Self { id: child.id() }
    }

    fn finished(&mut self) {
        self.id = None;
    }
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            // Safety: `killpg` reads no memory. The group is the one this tool
            // made with `process_group(0)`, and is released above the moment
            // the command has been waited for.
            unsafe { libc::killpg(id as libc::pid_t, libc::SIGKILL) };
        }
    }
}

/// The head and the tail of the output, with one line where the middle was.
///
/// A cut in the middle is the honest one: the head holds what the command
/// said it was doing and the tail holds how it ended, and a build log that
/// only kept its first quarter would hide the error the agent ran it for.
fn elide(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let half = limit / 2;
    let head = floor_boundary(text, half);
    let tail = ceil_boundary(text, text.len() - half);
    let cut = tail - head;
    format!(
        "{}\n[… {cut} bytes elided …]\n{}",
        &text[..head],
        &text[tail..]
    )
}

fn floor_boundary(text: &str, at: usize) -> usize {
    (0..=at)
        .rev()
        .find(|at| text.is_char_boundary(*at))
        .unwrap_or(0)
}

fn ceil_boundary(text: &str, at: usize) -> usize {
    (at..=text.len())
        .find(|at| text.is_char_boundary(*at))
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Reach;

    /// A command's own children die with it. Without the group, `sleep` here
    /// outlives the shell that started it and keeps running after the agent
    /// has been told the command is over.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_deadline_takes_the_command_and_everything_it_started() {
        let root = std::env::temp_dir().join(format!("toad-core-shell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let workspace = Workspace::open(root.clone(), Reach::Workspace).unwrap();

        let refused = RunCommand::new(workspace)
            .call(
                &mut ToolContext::new(),
                RunCommandArgs {
                    command: "sleep 120 & echo $! > child.pid; sleep 120".to_string(),
                    timeout_seconds: Some(1),
                },
            )
            .await
            .expect_err("the deadline should have ended the command");
        assert!(refused.to_string().contains("within 1 seconds"));

        let child: i32 = std::fs::read_to_string(root.join("child.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // Signal 0 asks whether the process is still there.
        let died = (0..100).any(|_| {
            std::thread::sleep(Duration::from_millis(10));
            (unsafe { libc::kill(child, 0) }) != 0
        });
        assert!(died, "the command's own child outlived the command");
    }

    #[test]
    fn output_under_the_limit_is_untouched() {
        assert_eq!(elide("hello", 16), "hello");
    }

    /// The head and the tail both survive, and the line between them says how
    /// much did not.
    #[test]
    fn a_long_output_keeps_both_ends_and_says_what_it_cut() {
        let text = format!("start{}end", "x".repeat(1_000));
        let elided = elide(&text, 100);
        assert!(elided.starts_with("start"));
        assert!(elided.ends_with("end"));
        assert!(elided.contains("[… 908 bytes elided …]"), "{elided}");
    }

    /// The cut lands on a character boundary, never inside one.
    #[test]
    fn a_multibyte_output_is_cut_between_characters() {
        let text = "é".repeat(1_000);
        let elided = elide(&text, 101);
        assert!(elided.contains("[…"));
        assert!(elided.starts_with('é'));
        assert!(elided.ends_with('é'));
    }
}
