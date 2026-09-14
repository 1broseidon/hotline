//! The shell command, run in the teammate's working directory.
//!
//! Nothing here is on a clock. An agent that is building, testing or watching
//! is doing its job, and a command killed at two minutes taught the previous
//! Toad's teammates to avoid the work rather than to finish it. The human's
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
use crate::contract::Reach;
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

// Rustup writes update hashes even when using an installed toolchain. Keep
// that metadata private while linking the read-only installed toolchains.
// Both launchers execute this only after the filesystem boundary is active.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const RUSTUP_SETUP: &str = r#"
if [ -n "${TOAD_INSTALLED_RUSTUP:-}" ]; then
    mkdir -p -- "$RUSTUP_HOME" || exit
    if [ ! -e "$RUSTUP_HOME/settings.toml" ]; then
        cp "$TOAD_INSTALLED_RUSTUP/settings.toml" "$RUSTUP_HOME/settings.toml" || exit
    fi
    if [ ! -e "$RUSTUP_HOME/toolchains" ]; then
        ln -s "$TOAD_INSTALLED_RUSTUP/toolchains" "$RUSTUP_HOME/toolchains" || exit
    fi
fi
unset TOAD_INSTALLED_RUSTUP
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
    pub(crate) command: String,
    /// A deadline the agent set for itself. Absent means none.
    #[serde(default)]
    timeout_seconds: Option<u64>,
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
        let outcome = self.run(args, CancellationToken::new()).await?;
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
pub(crate) enum CommandState {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Interrupted,
}

pub(crate) struct CommandOutcome {
    pub state: CommandState,
    pub output: String,
}

impl RunCommand {
    pub(crate) fn boundary(&self) -> &'static str {
        match self.workspace.reach() {
            Reach::Workspace if cfg!(target_os = "linux") => {
                "The shell can access the workspace, selected read-only installed tools, and private /tmp. Other host files are hidden. HOME is .toad-home inside the workspace; use it for persistent caches and user installs. Host credentials and environment variables are not inherited. Network access remains available."
            }
            Reach::Workspace => {
                "The shell is isolated from unrelated host files and other workspaces, with read-only access to supported installed runtimes. HOME is .toad-home inside the workspace. Use TMPDIR for private scratch; hardcoded /tmp is not writable. Host credentials and environment variables are not inherited. Network access remains available. macOS helper services and host control sockets are unavailable."
            }
            Reach::Machine => "The command runs with the teammate's whole-machine reach.",
        }
    }

    /// Cancellation owns the same process group as Stop, but keeps waiting
    /// after signalling it so the caller can distinguish a request from exit.
    pub(crate) async fn run(
        &self,
        args: RunCommandArgs,
        cancel: CancellationToken,
    ) -> Result<CommandOutcome, ToolError> {
        self.workspace.check_capability()?;
        let command = args.command.trim().to_string();
        if command.is_empty() {
            return Err(ToolError::new("A command is required."));
        }
        let cwd = self.workspace.display_root();
        let mut process = match self.workspace.reach() {
            Reach::Machine => unconfined(&command, cwd),
            Reach::Workspace => confined(&command, cwd).map_err(ToolError::other)?,
        };
        process
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        process.process_group(0);
        #[cfg(windows)]
        crate::process_windows::prepare(&mut process);
        self.workspace.check_capability()?;
        if cancel.is_cancelled() {
            return Ok(CommandOutcome {
                state: CommandState::Cancelled,
                output: "Cancelled before the command started.".into(),
            });
        }
        let child = process
            .spawn()
            .map_err(|error| ToolError::other(format!("The command could not start: {error}")))?;
        #[cfg(windows)]
        let job = crate::process_windows::Job::attach(child.id()).map_err(|error| {
            ToolError::other(format!(
                "Could not contain the command process tree: {error}"
            ))
        })?;
        let mut group = ProcessGroup::of(&child);
        let waiting = child.wait_with_output();
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
            text.push_str("\n[stderr]\n");
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
            CommandState::TimedOut => text.push_str(&format!("\nThe command did not finish within {} seconds.", args.timeout_seconds.unwrap_or_default())),
            _ if status != 0 => text.push_str(&format!("\n[exit status {status}]")),
            _ => {},
        }
        Ok(CommandOutcome {
            state,
            output: text,
        })
    }
}

/// Whether Toad Agent can honour `shell` for this reach. Checked when the
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

/// The sandboxed command. No `current_dir` on Linux: `bwrap --chdir` does it.
/// macOS sets cwd because `sandbox-exec` does not.
#[cfg(target_os = "linux")]
fn confined(command: &str, workspace: &Path) -> Result<Command, String> {
    linux::command(command, workspace)
}

#[cfg(target_os = "macos")]
fn confined(command: &str, workspace: &Path) -> Result<Command, String> {
    macos::command(command, workspace)
}

#[cfg(target_os = "windows")]
fn confined(_command: &str, _workspace: &Path) -> Result<Command, String> {
    Err(WINDOWS_UNCONFINED.to_string())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn confined(_command: &str, _workspace: &Path) -> Result<Command, String> {
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

    #[cfg(unix)]
    fn terminate(&self) -> std::io::Result<()> {
        let Some(id) = self.id else { return Ok(()) };
        // The id is captured from the child we spawned in its own group.
        if unsafe { libc::killpg(id as libc::pid_t, libc::SIGKILL) } == 0 {
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
        if let Some(id) = self.id {
            // Safety: `killpg` reads no memory. The group is the one this tool
            // made with `process_group(0)`, and is released above the moment
            // the command has been waited for.
            unsafe { libc::killpg(id as libc::pid_t, libc::SIGKILL) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::tool::Tool;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub(super) struct TestDirectory(PathBuf);

    impl TestDirectory {
        pub(super) fn new() -> Self {
            Self::in_parent(&std::env::temp_dir())
        }

        fn in_parent(parent: &Path) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = parent.join(format!("toad-core-shell-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        pub(super) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn workspace(root: &Path, reach: Reach) -> Workspace {
        Workspace::open(
            root.to_path_buf(),
            reach,
            root.with_extension("overflow-unused"),
        )
        .unwrap()
    }

    async fn run(workspace: Workspace, command: &str) -> Result<String, ToolError> {
        RunCommand::new(workspace)
            .call(
                &mut ToolContext::new(),
                RunCommandArgs {
                    command: command.to_string(),
                    timeout_seconds: Some(15),
                },
            )
            .await
    }

    fn finished_ok(output: &str) -> bool {
        !output.contains("[exit status")
    }

    #[cfg(target_os = "linux")]
    pub(super) fn skip_without_sandbox() -> bool {
        let probe = std::process::Command::new("bwrap")
            .args(["--ro-bind", "/", "/", "--unshare-pid", "/bin/true"])
            .output();
        if !probe.is_ok_and(|output| output.status.success()) {
            eprintln!("skipping: this machine cannot run bubblewrap");
            return true;
        }
        shell_available(Reach::Workspace).expect("the sandbox policy must work when bwrap works");
        false
    }

    #[cfg(target_os = "macos")]
    fn skip_without_sandbox() -> bool {
        shell_available(Reach::Workspace).expect("macOS must enforce the protected shell policy");
        false
    }

    /// A command's own children die with it. Without the group, `sleep` here
    /// outlives the shell that started it and keeps running after the agent
    /// has been told the command is over. The grandchild writes a heartbeat
    /// rather than a pid: `--unshare-pid` makes `$!` a namespace pid the
    /// host cannot signal.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_deadline_takes_the_command_and_everything_it_started() {
        let root = TestDirectory::new();
        let workspace = workspace(root.path(), Reach::Machine);

        let refused = RunCommand::new(workspace)
            .call(
                &mut ToolContext::new(),
                RunCommandArgs {
                    command: "while true; do echo x >> heartbeat; sleep 0.05; done & sleep 120"
                        .to_string(),
                    timeout_seconds: Some(3),
                },
            )
            .await
            .expect_err("the deadline should have ended the command");
        assert!(
            refused.to_string().contains("within 3 seconds"),
            "{refused}"
        );

        let heartbeat = root.path().join("heartbeat");
        let first = fs::read_to_string(&heartbeat).unwrap_or_default().len();
        assert!(
            first > 0,
            "the command must have started before cancellation"
        );
        std::thread::sleep(Duration::from_millis(200));
        let second = fs::read_to_string(&heartbeat).unwrap_or_default().len();
        assert_eq!(
            first, second,
            "the command's own child outlived the command"
        );
    }

    /// Linux tears down the PID namespace; macOS signals the owned process group.
    /// Wait for the child to start so a slow sandbox setup cannot turn this
    /// into a test that merely cancels the launcher before it ran anything.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn cancellation_reaches_a_started_sandbox_and_its_children() {
        if skip_without_sandbox() {
            return;
        }
        let root = TestDirectory::new();
        let workspace = workspace(root.path(), Reach::Workspace);
        let task = tokio::spawn(async move {
            RunCommand::new(workspace)
                .call(
                    &mut ToolContext::new(),
                    RunCommandArgs {
                        command: "while true; do echo x >> heartbeat; sleep 0.05; done & sleep 120"
                            .to_string(),
                        timeout_seconds: None,
                    },
                )
                .await
        });
        let heartbeat = root.path().join("heartbeat");
        let started = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if fs::metadata(&heartbeat).is_ok_and(|metadata| metadata.len() > 0) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        // Always cancel before asserting, including when startup failed.
        task.abort();
        let _ = task.await;
        assert!(started.is_ok(), "the sandbox did not start its child");
        let first = fs::read_to_string(&heartbeat).unwrap().len();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let second = fs::read_to_string(&heartbeat).unwrap().len();
        assert_eq!(first, second, "a sandbox child survived cancellation");
    }

    #[tokio::test]
    async fn cancellation_before_start_never_spawns_the_command() {
        let root = TestDirectory::new();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = RunCommand::new(workspace(root.path(), Reach::Machine))
            .run(
                RunCommandArgs {
                    command: "echo unwanted > unwanted.txt".into(),
                    timeout_seconds: None,
                },
                cancel,
            )
            .await
            .unwrap();
        assert_eq!(result.state, CommandState::Cancelled);
        assert!(!root.path().join("unwanted.txt").exists());
    }

    /// The command's own output is not cut here: the driver keeps the full
    /// result on disk when it is more than the model is handed.
    #[tokio::test]
    async fn a_long_output_is_returned_in_full() {
        let root = TestDirectory::new();
        let body = "x".repeat(300_000);
        fs::write(root.path().join("big.txt"), &body).unwrap();
        // Machine reach so this is the command's output, not the sandbox.
        let workspace = workspace(root.path(), Reach::Machine);
        let command = if cfg!(windows) {
            "type big.txt"
        } else {
            "cat big.txt"
        };
        let output = run(workspace, command).await.unwrap();
        assert_eq!(output, body);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shell_available_without_bwrap_names_the_need() {
        // A PATH that cannot see bwrap — the public function reads PATH the
        // same way, and mutating the process environment would race the rest
        // of the crate.
        let err = workspace_shell_on_path(Some(std::ffi::OsStr::new("/no-such-toad-bin")))
            .expect_err("bwrap is not on this PATH");
        assert_eq!(err, BWRAP_MISSING);
        assert!(
            shell_available(Reach::Machine).is_ok(),
            "machine reach does not need bwrap"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn workspace_reach_writes_inside_the_working_directory() {
        if skip_without_sandbox() {
            return;
        }
        let root = TestDirectory::new();
        let workspace = workspace(root.path(), Reach::Workspace);
        let output = run(workspace, "echo hi > out.txt && echo hi > /dev/null")
            .await
            .unwrap();
        assert!(finished_ok(&output), "{output}");
        assert_eq!(
            fs::read_to_string(root.path().join("out.txt"))
                .unwrap()
                .trim(),
            "hi"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn workspace_reach_refuses_a_write_outside() {
        if skip_without_sandbox() {
            return;
        }
        let root = TestDirectory::new();
        // The outside directory is not mounted into the sandbox.
        let outside = TestDirectory::in_parent(Path::new("/var/tmp"));
        let leak = outside.path().join("leak.txt");
        let workspace = workspace(root.path(), Reach::Workspace);
        let command = format!("echo hi > {}", leak.display());
        let output = run(workspace, &command).await.unwrap();
        assert!(
            output.contains("[exit status"),
            "outside write should have failed: {output}"
        );
        assert!(!leak.exists(), "outside write leaked to {}", leak.display());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn synthetic_parent_directories_are_read_only() {
        if skip_without_sandbox() {
            return;
        }
        // Outside /tmp: that mount is intentionally writable private scratch.
        let parent = TestDirectory::in_parent(Path::new("/var/tmp"));
        let root = parent.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::write(parent.path().join("host-only-canary"), "outside").unwrap();
        let listing = run(workspace(&root, Reach::Workspace), "ls -a ..")
            .await
            .unwrap();
        assert!(finished_ok(&listing), "{listing}");
        assert!(
            !listing.contains("host-only-canary"),
            "the listing exposed the host parent: {listing}"
        );
        let output = run(
            workspace(&root, Reach::Workspace),
            "echo probe > ../reach-test-parent.txt",
        )
        .await
        .unwrap();
        assert!(
            !parent.path().join("reach-test-parent.txt").exists(),
            "a write escaped to the host"
        );
        assert!(
            !finished_ok(&output),
            "synthetic parent was writable: {output}"
        );
        let output = run(
            workspace(&root, Reach::Workspace),
            "echo probe > /reach-test-root.txt",
        )
        .await
        .unwrap();
        assert!(
            !finished_ok(&output),
            "synthetic root was writable: {output}"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn workspace_reach_refuses_reads_outside_even_from_child_processes() {
        if skip_without_sandbox() {
            return;
        }
        let root = TestDirectory::new();
        let outside = TestDirectory::in_parent(Path::new("/var/tmp"));
        let file = outside.path().join("visible.txt");
        fs::write(&file, "secret").unwrap();
        std::os::unix::fs::symlink(&file, root.path().join("escape")).unwrap();
        for command in [
            format!("cat {}", file.display()),
            "cat escape".to_string(),
            format!("sh -c 'cat {}'", file.display()),
            format!("cat /proc/1/root{}", file.display()),
        ] {
            let output = run(workspace(root.path(), Reach::Workspace), &command)
                .await
                .unwrap();
            assert!(
                !finished_ok(&output),
                "outside read succeeded: {command}: {output}"
            );
            assert!(
                !output.contains("secret"),
                "outside contents leaked: {output}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn workspace_reach_tmp_is_private_scratch() {
        if skip_without_sandbox() {
            return;
        }
        let root = TestDirectory::new();
        let name = format!(
            "toad-shell-scratch-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let host = Path::new("/tmp").join(&name);
        let workspace = workspace(root.path(), Reach::Workspace);
        let output = run(workspace, &format!("touch /tmp/{name}")).await.unwrap();
        assert!(finished_ok(&output), "{output}");
        assert!(
            !host.exists(),
            "sandbox /tmp write landed on the host at {}",
            host.display()
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn workspace_shell_has_a_private_persistent_home_and_no_host_environment() {
        if skip_without_sandbox() {
            return;
        }
        let root = TestDirectory::new();
        let mut command = confined(
            "test -z \"$TOAD_TEST_SECRET\" && echo saved > \"$HOME/marker\"",
            root.path(),
        )
        .unwrap();
        // Even variables accidentally added by a caller must not reach the shell.
        command.env("TOAD_TEST_SECRET", "must-not-leak");
        let output = command.output().await.unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = run(
            workspace(root.path(), Reach::Workspace),
            "cat \"$HOME/marker\"",
        )
        .await
        .unwrap();
        assert_eq!(output.trim(), "saved");
        let other = TestDirectory::new();
        let output = run(
            workspace(other.path(), Reach::Workspace),
            "test ! -e \"$HOME/marker\"",
        )
        .await
        .unwrap();
        assert!(finished_ok(&output), "{output}");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn a_private_home_symlink_cannot_create_files_outside() {
        if skip_without_sandbox() {
            return;
        }
        let root = TestDirectory::new();
        let outside = TestDirectory::new();
        let target = outside.path().join("must-not-be-created");
        std::os::unix::fs::symlink(&target, root.path().join(".toad-home")).unwrap();
        let output = run(
            workspace(root.path(), Reach::Workspace),
            "echo should-not-run",
        )
        .await
        .unwrap();
        assert!(!finished_ok(&output), "{output}");
        assert!(!target.exists());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn workspace_shell_runs_installed_toolchains() {
        if skip_without_sandbox() {
            return;
        }
        let root = TestDirectory::new();
        fs::write(
            root.path().join("main.go"),
            "package main\nfunc main() { println(\"go-ok\") }\n",
        )
        .unwrap();
        fs::write(
            root.path().join("main.rs"),
            "fn main() { println!(\"rust-ok\"); }\n",
        )
        .unwrap();
        for (tool, command, expected) in [
            (
                "python3",
                "python3 -c 'import ssl, sqlite3; print(\"python-ok\")'",
                "python-ok",
            ),
            ("npm", "npm --version && echo npm-ok", "npm-ok"),
            ("node", "node -e 'console.log(\"node-ok\")'", "node-ok"),
            ("go", "go run main.go", "go-ok"),
            ("rustc", "rustc main.rs -o app && ./app", "rust-ok"),
        ] {
            if !path_has_command(tool, std::env::var_os("PATH").as_deref()) {
                continue;
            }
            #[cfg(target_os = "macos")]
            if !macos::supported_tool_on_path(tool) {
                eprintln!("{tool} is not installed in a supported macOS runtime layout");
                continue;
            }
            let output = RunCommand::new(workspace(root.path(), Reach::Workspace))
                .call(
                    &mut ToolContext::new(),
                    RunCommandArgs {
                        command: command.to_string(),
                        timeout_seconds: Some(120),
                    },
                )
                .await
                .unwrap();
            assert!(
                finished_ok(&output) && output.contains(expected),
                "{tool}: {output}"
            );
        }
    }

    #[tokio::test]
    async fn machine_reach_writes_outside() {
        let root = TestDirectory::new();
        let outside = TestDirectory::new();
        let leak = outside.path().join("leak.txt");
        let workspace = workspace(root.path(), Reach::Machine);
        let output = run(workspace, &format!("echo hi > \"{}\"", leak.display()))
            .await
            .unwrap();
        assert!(finished_ok(&output), "{output}");
        assert_eq!(fs::read_to_string(&leak).unwrap().trim(), "hi");
    }
}
