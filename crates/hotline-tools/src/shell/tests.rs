use super::*;
use rig::tool::Tool;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Two tests in one process may read the same clock; the counter keeps
/// their roots apart.
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
        let root = parent.join(format!(
            "hotline-core-shell-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
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

#[test]
fn a_zero_deadline_is_no_deadline() {
    let args: RunCommandArgs =
        serde_json::from_value(serde_json::json!({"command": "true", "timeout_seconds": 0}))
            .unwrap();
    assert_eq!(args.timeout_seconds, None);
    let args: RunCommandArgs =
        serde_json::from_value(serde_json::json!({"command": "true", "timeout_seconds": 5}))
            .unwrap();
    assert_eq!(args.timeout_seconds, Some(5));
    let args: RunCommandArgs =
        serde_json::from_value(serde_json::json!({"command": "true"})).unwrap();
    assert_eq!(args.timeout_seconds, None);
}

fn finished_ok(output: &str) -> bool {
    !output.contains("[exit status")
}

/// The sandbox tests are ignored on Linux, where bubblewrap is a package
/// a machine may not have, and run with `--ignored` where it does. Asked
/// for without it, they fail here, naming it.
#[cfg(target_os = "linux")]
pub(super) fn require_sandbox() {
    let probe = std::process::Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "--unshare-pid", "/bin/true"])
        .output();
    assert!(
        probe.is_ok_and(|output| output.status.success()),
        "needs bubblewrap: `bwrap` is missing or cannot make a sandbox here"
    );
    shell_available(Reach::Workspace).expect("the sandbox policy must work when bwrap works");
}

#[cfg(target_os = "macos")]
fn require_sandbox() {
    shell_available(Reach::Workspace).expect("macOS must enforce the protected shell policy");
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
                command:
                    "while true; do echo x >> heartbeat; sleep 0.05; done & sleep 120; touch after"
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
    assert!(!root.path().join("after").exists());
}

/// Linux tears down the PID namespace; macOS signals the owned process group.
/// Wait for the child to start so a slow sandbox setup cannot turn this
/// into a test that merely cancels the launcher before it ran anything.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
#[cfg_attr(target_os = "linux", ignore = "needs bubblewrap")]
async fn cancellation_reaches_a_started_sandbox_and_its_children() {
    require_sandbox();
    let root = TestDirectory::new();
    let workspace = workspace(root.path(), Reach::Workspace);
    let task = tokio::spawn(async move {
        RunCommand::new(workspace)
            .call(
                &mut ToolContext::new(),
                RunCommandArgs {
                    command: "while true; do echo x >> heartbeat; sleep 0.05; done & sleep 120; touch after"
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
    assert!(!root.path().join("after").exists());
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
            &OutputTail::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.state, CommandState::Cancelled);
    assert!(!root.path().join("unwanted.txt").exists());
}

/// Observing the sleep itself avoids passing by cancelling before the shell
/// reaches the wait. The group id comes from our shell, never a name match.
#[cfg(unix)]
async fn sleeping_group(root: &Path) -> bool {
    let Ok(group) = fs::read_to_string(root.join("started")) else {
        return false;
    };
    let processes = Command::new("ps")
        .args(["-axo", "pgid=,comm="])
        .output()
        .await
        .expect("ps must list the command's process group");
    assert!(processes.status.success());
    String::from_utf8_lossy(&processes.stdout)
        .lines()
        .any(|line| {
            let mut fields = line.split_whitespace();
            fields.next() == Some(group.trim())
                && fields.next().is_some_and(|command| {
                    Path::new(command).file_name() == Some(std::ffi::OsStr::new("sleep"))
                })
        })
}

#[cfg(unix)]
#[tokio::test]
async fn cancellation_during_sleep_never_runs_the_next_statement() {
    for attempt in 1..=50 {
        let root = tempfile::tempdir().unwrap();
        let workspace = workspace(root.path(), Reach::Machine);
        let cancel = CancellationToken::new();
        let task = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                RunCommand::new(workspace)
                    .run(
                        RunCommandArgs {
                            command: "echo $$ > started; sleep 30; touch after".into(),
                            timeout_seconds: None,
                        },
                        cancel,
                        &OutputTail::default(),
                    )
                    .await
            }
        });
        let started = tokio::time::timeout(Duration::from_secs(15), async {
            while !sleeping_group(root.path()).await {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        // Even a failed startup must leave no job behind.
        cancel.cancel();
        let result = task.await.unwrap().unwrap();
        assert!(started.is_ok(), "attempt {attempt}: sleep never started");
        assert_eq!(result.state, CommandState::Cancelled, "attempt {attempt}");
        assert!(
            !root.path().join("after").exists(),
            "attempt {attempt}: cancellation ran the next statement"
        );
    }
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
    let err = workspace_shell_on_path(Some(std::ffi::OsStr::new("/no-such-hotline-bin")))
        .expect_err("bwrap is not on this PATH");
    assert_eq!(err, BWRAP_MISSING);
    assert!(
        shell_available(Reach::Machine).is_ok(),
        "machine reach does not need bwrap"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[cfg_attr(target_os = "linux", ignore = "needs bubblewrap")]
async fn workspace_reach_writes_inside_the_working_directory() {
    require_sandbox();
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
#[cfg_attr(target_os = "linux", ignore = "needs bubblewrap")]
async fn workspace_reach_refuses_a_write_outside() {
    require_sandbox();
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
#[cfg_attr(target_os = "linux", ignore = "needs bubblewrap")]
async fn synthetic_parent_directories_are_read_only() {
    require_sandbox();
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
#[cfg_attr(target_os = "linux", ignore = "needs bubblewrap")]
async fn workspace_reach_refuses_reads_outside_even_from_child_processes() {
    require_sandbox();
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
#[cfg_attr(target_os = "linux", ignore = "needs bubblewrap")]
async fn workspace_reach_tmp_is_private_scratch() {
    require_sandbox();
    let root = TestDirectory::new();
    let name = format!(
        "hotline-shell-scratch-{}-{}",
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
#[cfg_attr(target_os = "linux", ignore = "needs bubblewrap")]
async fn workspace_shell_has_a_private_persistent_home_and_no_host_environment() {
    require_sandbox();
    let root = TestDirectory::new();
    let mut command = confined(
        "test -z \"$HOTLINE_TEST_SECRET\" && echo saved > \"$HOME/marker\"",
        root.path(),
        &[],
    )
    .unwrap();
    // Even variables accidentally added by a caller must not reach the shell.
    command.env("HOTLINE_TEST_SECRET", "must-not-leak");
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
#[cfg_attr(target_os = "linux", ignore = "needs bubblewrap")]
async fn a_private_home_symlink_cannot_create_files_outside() {
    require_sandbox();
    let root = TestDirectory::new();
    let outside = TestDirectory::new();
    let target = outside.path().join("must-not-be-created");
    std::os::unix::fs::symlink(&target, root.path().join(".hotline-home")).unwrap();
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
#[cfg_attr(target_os = "linux", ignore = "needs bubblewrap")]
async fn workspace_shell_runs_installed_toolchains() {
    require_sandbox();
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
    let toolchains = [
        (
            "python3",
            "python3 -c 'import ssl, sqlite3; print(\"python-ok\")'",
            "python-ok",
        ),
        ("npm", "npm --version && echo npm-ok", "npm-ok"),
        ("node", "node -e 'console.log(\"node-ok\")'", "node-ok"),
        ("go", "go run main.go", "go-ok"),
        ("rustc", "rustc main.rs -o app && ./app", "rust-ok"),
    ];
    let mut ran_any = false;
    for (tool, command, expected) in toolchains {
        if !path_has_command(tool, std::env::var_os("PATH").as_deref()) {
            eprintln!("{tool} is not on PATH");
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
        ran_any = true;
    }
    // A machine with none of them would otherwise pass having run nothing.
    assert!(
        ran_any,
        "needs python3, npm, node, go or rustc installed: none was found to run"
    );
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
