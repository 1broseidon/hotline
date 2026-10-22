//! The shell command, run in the teammate's working directory.
//!
//! Nothing here is on a clock. An agent that is building, testing or watching
//! is doing its job, and a command killed at two minutes taught the previous
//! Hotline's teammates to avoid the work rather than to finish it. The human's
//! Stop is the cap. Managed jobs also accept cancellation independently of
//! model requests, signal the owned process tree, and observe its exit. A
//! dropped call still kills its process group through its guard, so the
//! child's own children go with it, which is the difference between stopping a
//! build and orphaning its compiler. An agent that wants its own deadline
//! passes `timeout_seconds`.
//!
//! Linux workspace reach exposes only the workspace and selected read-only
//! toolchain installations. The shell has a private home inside the workspace
//! and a private `/tmp`; host credentials and other projects are not mounted.
//! Machine reach is the command as typed, with no wall.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

use super::{ToolError, Workspace};
use hotline_contract::Reach;
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

// Rustup writes update hashes even when using an installed toolchain. Keep
// that metadata private while linking the read-only installed toolchains.
// Both launchers execute this only after the filesystem boundary is active.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const RUSTUP_SETUP: &str = r#"
if [ -n "${HOTLINE_INSTALLED_RUSTUP:-}" ]; then
    mkdir -p -- "$RUSTUP_HOME" || exit
    if [ ! -e "$RUSTUP_HOME/settings.toml" ]; then
        cp "$HOTLINE_INSTALLED_RUSTUP/settings.toml" "$RUSTUP_HOME/settings.toml" || exit
    fi
    if [ ! -e "$RUSTUP_HOME/toolchains" ]; then
        ln -s "$HOTLINE_INSTALLED_RUSTUP/toolchains" "$RUSTUP_HOME/toolchains" || exit
    fi
fi
unset HOTLINE_INSTALLED_RUSTUP
"#;

/// Why the ledger omits `shell` when Linux cannot confine it.
#[cfg(target_os = "linux")]
const BWRAP_MISSING: &str = "The shell needs bubblewrap (`bwrap`) to stay inside the workspace; install it or give the teammate machine reach.";
/// `bwrap` is on PATH but a trivial sandbox fails (Ubuntu 24.04's AppArmor
/// restriction on unprivileged user namespaces is the usual cause).
#[cfg(target_os = "linux")]
const BWRAP_UNUSABLE: &str = "The shell needs bubblewrap (`bwrap`) to stay inside the workspace, but it cannot create a sandbox on this machine; give the teammate machine reach.";
/// Why the ledger omits `shell` on Windows under workspace reach.
#[cfg(target_os = "windows")]
const WINDOWS_UNCONFINED: &str = "The shell is not confined to the workspace on Windows; give the teammate machine reach to use it.";

#[derive(Deserialize)]
pub struct RunCommandArgs {
    pub command: String,
    /// A deadline the agent set for itself. Absent means none, and so does
    /// zero: a model that sends 0 means "no deadline", not "stop at once".
    #[serde(default, deserialize_with = "deadline")]
    timeout_seconds: Option<u64>,
}

fn deadline<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    Ok(Option::<u64>::deserialize(deserializer)?.filter(|seconds| *seconds > 0))
}

/// A shell command, run in the teammate's working directory.
#[derive(Clone)]
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
        format!(
            "Run a shell command in the working directory and return its output. {} There is no time limit unless timeout_seconds is provided.",
            self.boundary()
        )
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
                    "minimum": 1,
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
        let outcome = self
            .run(args, CancellationToken::new(), &OutputTail::default())
            .await?;
        if matches!(
            outcome.state,
            CommandState::TimedOut | CommandState::Interrupted
        ) {
            return Err(ToolError::other(outcome.output));
        }
        Ok(outcome.output)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandState {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Interrupted,
}

pub struct CommandOutcome {
    pub state: CommandState,
    pub output: String,
}

/// The line between a command's stdout and its stderr in its output.
pub const STDERR_LINE: &str = "\n[stderr]\n";

/// How much of a running command's latest output is kept to be read while it
/// runs: a screen or two, which is what anyone checking on a server reads.
const TAIL_BYTES: usize = 16 * 1024;

/// The last of a command's output while it runs, stdout and stderr together
/// in the order they arrived. The whole output is still the outcome's; this
/// is only so that somebody can look before the command ends.
#[derive(Default)]
pub struct OutputTail(std::sync::Mutex<std::collections::VecDeque<u8>>);

impl OutputTail {
    fn push(&self, bytes: &[u8]) {
        let mut tail = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tail.extend(bytes);
        let excess = tail.len().saturating_sub(TAIL_BYTES);
        tail.drain(..excess);
    }

    pub fn text(&self) -> String {
        let mut tail = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bytes = tail.make_contiguous();
        // A cut can land inside a character; start at the next whole one.
        let start = bytes
            .iter()
            .position(|byte| (*byte as i8) >= -0x40)
            .unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[start..]).into_owned()
    }
}

/// Reads one of the command's pipes to its end, keeping all of it and
/// showing it to the tail as it comes.
async fn drain(
    pipe: Option<impl tokio::io::AsyncRead + Unpin>,
    tail: &OutputTail,
) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut all = Vec::new();
    let Some(mut pipe) = pipe else {
        return Ok(all);
    };
    let mut chunk = [0u8; 8192];
    loop {
        let read = pipe.read(&mut chunk).await?;
        if read == 0 {
            return Ok(all);
        }
        all.extend_from_slice(&chunk[..read]);
        tail.push(&chunk[..read]);
    }
}

impl RunCommand {
    pub fn boundary(&self) -> String {
        let folders = self.workspace.folders();
        if self.workspace.reach() == Reach::Workspace && !folders.is_empty() {
            let named: Vec<String> = folders
                .iter()
                .map(|(path, writable)| {
                    format!(
                        "{} ({})",
                        path.display(),
                        if *writable { "read-write" } else { "read-only" }
                    )
                })
                .collect();
            return format!(
                "{} These granted folders are also open to it at their own paths: {}.",
                self.wall(),
                named.join(", ")
            );
        }
        self.wall().to_string()
    }

    fn wall(&self) -> &'static str {
        match self.workspace.reach() {
            Reach::Workspace if cfg!(target_os = "linux") => {
                "The shell can access the workspace, selected read-only installed tools, and private /tmp. Other host files are hidden. HOME is .hotline-home inside the workspace; use it for persistent caches and user installs. Host credentials and environment variables are not inherited. Network access remains available."
            }
            Reach::Workspace => {
                "The shell is isolated from unrelated host files and other workspaces, with read-only access to supported installed runtimes. HOME is .hotline-home inside the workspace. Use TMPDIR for private scratch; hardcoded /tmp is not writable. Host credentials and environment variables are not inherited. Network access remains available. macOS helper services and host control sockets are unavailable."
            }
            Reach::Machine => "The command runs with the teammate's whole-machine reach.",
        }
    }

    /// Cancellation owns the same process group as Stop, but keeps waiting
    /// after signalling it so the caller can distinguish a request from exit.
    /// The output reaches `tail` as it is written.
    pub async fn run(
        &self,
        args: RunCommandArgs,
        cancel: CancellationToken,
        tail: &OutputTail,
    ) -> Result<CommandOutcome, ToolError> {
        self.workspace.check_capability()?;
        let command = args.command.trim().to_string();
        if command.is_empty() {
            return Err(ToolError::new("A command is required."));
        }
        let cwd = self.workspace.display_root();
        let mut process = match self.workspace.reach() {
            Reach::Machine => unconfined(&command, cwd),
            Reach::Workspace => {
                confined(&command, cwd, &self.workspace.folders()).map_err(ToolError::other)?
            }
        };
        process
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        process.process_group(0);
        #[cfg(windows)]
        hotline_support::process_windows::prepare(&mut process);
        self.workspace.check_capability()?;
        if cancel.is_cancelled() {
            return Ok(CommandOutcome {
                state: CommandState::Cancelled,
                output: "Cancelled before the command started.".into(),
            });
        }
        let mut child = process
            .spawn()
            .map_err(|error| ToolError::other(format!("The command could not start: {error}")))?;
        #[cfg(windows)]
        let job = hotline_support::process_windows::Job::attach(child.id()).map_err(|error| {
            ToolError::other(format!(
                "Could not contain the command process tree: {error}"
            ))
        })?;
        let mut group = ProcessGroup::of(&child);
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
        let waiting = async {
            let (stdout, stderr, status) =
                tokio::try_join!(drain(stdout, tail), drain(stderr, tail), child.wait())?;
            std::io::Result::Ok(std::process::Output {
                status,
                stdout,
                stderr,
            })
        };
        tokio::pin!(waiting);
        let deadline = async {
            match args.timeout_seconds {
                Some(seconds) => tokio::time::sleep(Duration::from_secs(seconds)).await,
                None => std::future::pending::<()>().await,
            }
        };
        let mut stopped = None;
        let output = tokio::select! {
            biased;
            output = &mut waiting => Some(output),
            () = cancel.cancelled() => { stopped = Some(CommandState::Cancelled); None },
            () = deadline => { stopped = Some(CommandState::TimedOut); None },
        };
        let output = if let Some(state) = stopped {
            #[cfg(unix)]
            group.terminate().map_err(|error| ToolError::other(format!("Cancellation could not be sent: {error}")))?;
            #[cfg(windows)]
            job.terminate().map_err(|error| ToolError::other(format!("Cancellation could not be sent: {error}")))?;
            match tokio::time::timeout(Duration::from_secs(5), &mut waiting).await {
                Ok(output) => output,
                Err(_) => return Ok(CommandOutcome {
                    state: CommandState::Interrupted,
                    output: format!("{state:?} was requested, but process termination could not be confirmed. Files or other effects may already have changed."),
                }),
            }
        } else {
            output.expect("an uninterrupted wait returned its output")
        }.map_err(|error| ToolError::other(format!("The command could not be run: {error}")))?;
        group.finished();
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        let errors = String::from_utf8_lossy(&output.stderr);
        if !errors.trim().is_empty() {
            text.push_str(STDERR_LINE);
            text.push_str(&errors);
        }
        let status = output.status.code().unwrap_or(-1);
        let state = stopped.unwrap_or(if status == 0 {
            CommandState::Succeeded
        } else {
            CommandState::Failed
        });
        match state {
            CommandState::Cancelled => text.push_str("\n[Cancelled: the command exited after termination was requested. Existing effects were not rolled back.]"),
            CommandState::TimedOut => {
                let _ = write!(text, "\nThe command did not finish within {} seconds.", args.timeout_seconds.unwrap_or_default());
            }
            _ if status != 0 => {
                let _ = write!(text, "\n[exit status {status}]");
            }
            _ => {},
        }
        Ok(CommandOutcome {
            state,
            output: text,
        })
    }
}

/// Whether Hotline Agent can honour `shell` for this reach. Checked when the
/// tool set is built, so the ledger is true from the first turn and a call
/// is never offered that we cannot confine.
pub fn shell_available(reach: Reach) -> Result<(), String> {
    match reach {
        Reach::Machine => Ok(()),
        Reach::Workspace => workspace_shell_on_path(std::env::var_os("PATH").as_deref()),
    }
}

fn workspace_shell_on_path(path: Option<&std::ffi::OsStr>) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        if !path_has_command("bwrap", path) {
            return Err(BWRAP_MISSING.to_string());
        }
        linux::available()
    }
    #[cfg(target_os = "macos")]
    {
        let _ = path;
        macos::available()
    }
    #[cfg(target_os = "windows")]
    {
        let _ = path;
        Err(WINDOWS_UNCONFINED.to_string())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        Err(
            "The shell is not confined to the workspace on this operating system; give the teammate machine reach."
                .to_string(),
        )
    }
}

fn unconfined(command: &str, workspace: &Path) -> Command {
    #[cfg(windows)]
    let mut process = {
        // cmd.exe does not read CommandLineToArgvW escaping, so the standard
        // quoting would turn every `"` in the command into `\"`. With `/s`
        // cmd strips the outer quotes and runs the rest as typed; `/d` skips
        // AutoRun commands from the registry.
        let mut process = Command::new("cmd");
        process.raw_arg(format!("/d /s /c \"{command}\""));
        process
    };
    #[cfg(not(windows))]
    let mut process = {
        let mut process = Command::new("sh");
        process.args(["-c", command]);
        process
    };
    process.current_dir(workspace);
    process
}

/// The sandboxed command, given the workspace and each granted folder with
/// whether it may be changed. No `current_dir` on Linux: `bwrap --chdir` does
/// it. macOS sets cwd because `sandbox-exec` does not.
#[cfg(target_os = "linux")]
fn confined(
    command: &str,
    workspace: &Path,
    folders: &[(PathBuf, bool)],
) -> Result<Command, String> {
    linux::command(command, workspace, folders)
}

#[cfg(target_os = "macos")]
fn confined(
    command: &str,
    workspace: &Path,
    folders: &[(PathBuf, bool)],
) -> Result<Command, String> {
    macos::command(command, workspace, folders)
}

#[cfg(target_os = "windows")]
fn confined(
    _command: &str,
    _workspace: &Path,
    _folders: &[(PathBuf, bool)],
) -> Result<Command, String> {
    Err(WINDOWS_UNCONFINED.to_string())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn confined(
    _command: &str,
    _workspace: &Path,
    _folders: &[(PathBuf, bool)],
) -> Result<Command, String> {
    Err(
        "The shell is not confined to the workspace on this operating system; give the teammate machine reach."
            .to_string(),
    )
}

#[cfg(any(target_os = "linux", all(test, target_os = "macos")))]
fn path_has_command(name: &str, path: Option<&std::ffi::OsStr>) -> bool {
    let Some(paths) = path else {
        return false;
    };
    std::env::split_paths(paths).any(|dir| dir.join(name).is_file())
}

/// The child's process group, killed when this is dropped.
///
/// The command is spawned into a group of its own, so this reaches everything
/// it started and not this process. Once the command has been waited for its
/// group is gone, and killing then could only reach whatever the operating
/// system next gave that number to — so a finished command releases the guard.
struct ProcessGroup {
    id: Option<u32>,
}

impl ProcessGroup {
    fn of(child: &tokio::process::Child) -> Self {
        Self { id: child.id() }
    }

    fn finished(&mut self) {
        self.id = None;
    }

    #[cfg(unix)]
    fn terminate(&self) -> std::io::Result<()> {
        // A group signal is delivered member by member. If sleep dies before
        // its waiting shell, the shell can run its next statement. Stop the
        // entire group first so no waiter resumes when we kill its child.
        let stopped = self.signal(libc::SIGSTOP);
        let killed = self.signal(libc::SIGKILL);
        // A failed stop must not prevent a best-effort kill.
        stopped.and(killed)
    }

    #[cfg(unix)]
    fn signal(&self, signal: libc::c_int) -> std::io::Result<()> {
        let Some(id) = self.id else { return Ok(()) };
        // SAFETY: killpg takes no memory, and the id is captured from the
        // child we spawned in its own group.
        if unsafe { libc::killpg(id as libc::pid_t, signal) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[cfg(test)]
mod tests;
