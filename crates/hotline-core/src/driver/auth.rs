//! Desktop-owned, ephemeral harness sign-in. Nothing here is a transcript update.
use crate::accounts::env::EnvPlan;
use crate::wire::AuthStatus;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use tokio_util::sync::CancellationToken;

const OUTPUT_LIMIT: usize = 64 * 1024;
const INPUT_LIMIT: usize = 4096;
pub(crate) const COMMAND_FAILED: &str = "The harness sign-in command failed. You can try again.";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignIn {
    pub harness_name: String,
    pub methods: Vec<SignInMethod>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SignInMethod {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

pub(crate) struct Attempt {
    pub id: String,
    pub cancel: CancellationToken,
    pub owner: CancellationToken,
    status: Mutex<AuthStatus>,
    input: Mutex<Option<mpsc::SyncSender<String>>>,
}

impl Attempt {
    pub fn new(owner: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            id: uuid::Uuid::new_v4().to_string(),
            cancel: CancellationToken::new(),
            owner,
            status: Mutex::new(AuthStatus {
                state: "running",
                output: String::new(),
                error: None,
            }),
            input: Mutex::new(None),
        })
    }
    pub fn running(&self) -> bool {
        lock(&self.status).state == "running"
    }
    pub fn state(&self) -> (&'static str, Option<String>) {
        let status = lock(&self.status);
        (status.state, status.error.clone())
    }
    pub fn output(&self, bytes: &[u8]) {
        let mut status = lock(&self.status);
        if self.owner.is_cancelled() || self.cancel.is_cancelled() || status.state != "running" {
            return;
        }
        status.output.push_str(&String::from_utf8_lossy(bytes));
        if status.output.len() > OUTPUT_LIMIT {
            let mut cut = status.output.len() - OUTPUT_LIMIT;
            while !status.output.is_char_boundary(cut) {
                cut += 1;
            }
            status.output.drain(..cut);
        }
    }
    pub fn poll(&self) -> AuthStatus {
        let mut status = lock(&self.status);
        let mut answer = AuthStatus {
            state: status.state,
            output: std::mem::take(&mut status.output),
            error: status.error.clone(),
        };
        if self.owner.is_cancelled() || self.cancel.is_cancelled() {
            answer.output = String::new();
        }
        answer
    }
    pub fn finish(&self, result: Result<(), String>) {
        lock(&self.input).take();
        let mut status = lock(&self.status);
        status.state = if result.is_ok() {
            "succeeded"
        } else {
            "failed"
        };
        status.error = result.err();
        if self.owner.is_cancelled() || self.cancel.is_cancelled() {
            status.output = String::new();
        }
    }
    pub fn input(&self, input: &str) -> Result<(), String> {
        if !self.running() || self.cancel.is_cancelled() || self.owner.is_cancelled() {
            return Err("That sign-in has ended.".into());
        }
        if input.len() > INPUT_LIMIT {
            return Err("Sign-in input is too large.".into());
        }
        lock(&self.input)
            .as_ref()
            .ok_or("This sign-in is not accepting terminal input.")?
            .try_send(input.to_owned())
            .map_err(|_| "Sign-in input is busy; try again.".into())
    }
}

/// `launch` is captured when the initialized harness is spawned. `args` and `env`
/// come from the harness's advertised method, or from an account's row and a
/// checked email. No UI-supplied command, path, environment or shell
/// interpolation reaches this function.
pub(crate) async fn terminal(
    launch: super::acp::registry::Launch,
    cwd: PathBuf,
    args: Vec<String>,
    env: EnvPlan,
    attempt: Arc<Attempt>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || terminal_blocking(launch, cwd, args, env, attempt))
        .await
        .map_err(|_| "The sign-in terminal stopped unexpectedly.".to_string())?
}

#[cfg(unix)]
fn terminal_blocking(
    launch: super::acp::registry::Launch,
    cwd: PathBuf,
    args: Vec<String>,
    env: EnvPlan,
    attempt: Arc<Attempt>,
) -> Result<(), String> {
    use portable_pty::{CommandBuilder, PtySize};
    use std::io::ErrorKind;
    use std::time::Duration;
    let fail = |_| "The sign-in terminal could not be started.".to_string();
    if attempt.cancel.is_cancelled() || attempt.owner.is_cancelled() {
        return Err("Sign-in cancelled.".into());
    }
    let pair = portable_pty::native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(fail)?;
    let mut command = CommandBuilder::new(&launch.command);
    command.args(&launch.args);
    command.args(&args);
    command.cwd(&cwd);
    env.apply_pty(&mut command);
    // Own one nonblocking master, not blocking reader/writer threads: a trusted
    // harness can move a descendant to another process group and keep the slave
    // open indefinitely. Neither EOF nor a writable slave is a cleanup condition.
    let fd = pair
        .master
        .as_raw_fd()
        .ok_or("The sign-in terminal could not be started.")?;
    // SAFETY: pair.master owns this descriptor until the loop and cleanup end.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    // SAFETY: the same descriptor, still owned by pair.master.
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("The sign-in terminal could not be started.".into());
    }
    let mut child = pair.slave.spawn_command(command).map_err(fail)?;
    let pid = child.process_id();
    drop(pair.slave);
    let (tx, rx) = mpsc::sync_channel::<String>(16);
    *lock(&attempt.input) = Some(tx);
    let mut pending = String::new();
    let mut written = 0;
    let mut bytes = [0; 4096];
    let mut exited = false;
    let result = loop {
        if attempt.cancel.is_cancelled() || attempt.owner.is_cancelled() {
            break Err("Sign-in cancelled.".into());
        }
        // At most one read and write per tick, so a continuously noisy harness
        // cannot starve cancellation or child-exit observation.
        // SAFETY: the descriptor and the writable buffer are live for this call.
        let n = unsafe { libc::read(fd, bytes.as_mut_ptr().cast(), bytes.len()) };
        if n > 0 {
            attempt.output(&bytes[..n as usize]);
        } else if n < 0 {
            let error = std::io::Error::last_os_error();
            if !matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted)
                && error.raw_os_error() != Some(libc::EIO)
            {
                break Err("Could not read the sign-in terminal.".into());
            }
        }
        if written == pending.len() {
            pending.clear();
            written = 0;
            if let Ok(input) = rx.try_recv() {
                pending = input;
            }
        }
        if written < pending.len() {
            let remaining = &pending.as_bytes()[written..];
            // SAFETY: the descriptor and the readable slice are live for this call.
            let n = unsafe { libc::write(fd, remaining.as_ptr().cast(), remaining.len()) };
            if n > 0 {
                written += n as usize;
            } else if n < 0 {
                let error = std::io::Error::last_os_error();
                if !matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) {
                    break Err("Could not write to the sign-in terminal.".into());
                }
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                exited = true;
                break if status.success() {
                    Ok(())
                } else {
                    Err(COMMAND_FAILED.into())
                };
            }
            Err(_) => break Err("Could not observe the sign-in terminal.".into()),
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    // Stop admission and discard queued/plaintext IO before waiting for the child.
    // No IO worker survives this function; dropping the master never waits for EOF.
    lock(&attempt.input).take();
    drop(rx);
    drop(pending);
    bytes.fill(0);
    if attempt.cancel.is_cancelled() || attempt.owner.is_cancelled() {
        lock(&attempt.status).output = String::new();
    }
    // portable-pty starts the captured child as a session/process-group leader.
    // Kill that group and reap the direct child. This is NOT a process sandbox:
    // descendants that change groups or daemonize can survive; closing our master
    // releases Hotline's IO regardless. Do not discover targets by name or path.
    if let Some(pid) = pid {
        // SAFETY: only the child/group captured at spawn are targeted. An unreaped
        // child still owns its PID; never signal the individual PID after reaping.
        unsafe {
            libc::killpg(pid as libc::pid_t, libc::SIGKILL);
            if !exited {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
    drop(pair.master);
    // Even an OS that cannot complete SIGKILL must not strand a worker or retain
    // terminal input. On this exceptional failure the child may remain unreaped;
    // report failure rather than claim it exited or detach an indefinite reaper.
    let reap_deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !exited {
        match child.try_wait() {
            Ok(Some(_)) => exited = true,
            Ok(None) if std::time::Instant::now() < reap_deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            _ => return Err("Could not confirm the sign-in terminal stopped.".into()),
        }
    }
    result
}

#[cfg(not(unix))]
fn terminal_blocking(
    _launch: super::acp::registry::Launch,
    _cwd: PathBuf,
    _args: Vec<String>,
    _env: EnvPlan,
    _attempt: Arc<Attempt>,
) -> Result<(), String> {
    Err("Terminal sign-in is not supported on this platform.".into())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::accounts::env::env_plan;

    #[cfg(unix)]
    fn term() -> (String, String) {
        ("TERM".into(), "xterm-256color".into())
    }

    #[test]
    fn output_is_bounded_and_drained_and_disconnection_forgets_it() {
        let attempt = Attempt::new(CancellationToken::new());
        assert_eq!(attempt.state(), ("running", None));
        attempt.output("é".repeat(OUTPUT_LIMIT).as_bytes());
        assert_eq!(attempt.poll().output.len(), OUTPUT_LIMIT);
        assert_eq!(attempt.poll().output, "");
        attempt.output(b"fixture-secret");
        attempt.owner.cancel();
        assert_eq!(attempt.poll().output, "");
        assert!(attempt.input("anything").is_err());
        attempt.finish(Err("x".into()));
        assert_eq!(attempt.state(), ("failed", Some("x".into())));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn auth_terminal_fixture_succeeds_fails_and_cancels_its_process_group() {
        use super::super::acp::registry::Launch;
        use std::time::Duration;
        for outcome in ["success", "fail", "cancel", "disconnect"] {
            let attempt = Attempt::new(CancellationToken::new());
            let code = if outcome == "success" {
                "test -t 0 && test -t 1 || exit 9; stty -echo; printf 'ready:%s:%s\\n' \"$FIXTURE\" \"$TERM\"; read answer; test \"$answer\" = yes"
            } else if outcome == "fail" {
                "echo failure; exit 7"
            } else {
                "echo waiting; sleep 60 & wait"
            };
            let launch = Launch {
                command: "/bin/sh".into(),
                args: vec!["-c".into()],
                env: Vec::new(),
            };
            let running = tokio::spawn(terminal(
                launch,
                std::env::temp_dir(),
                vec![code.into()],
                env_plan(
                    &[],
                    &[term(), ("FIXTURE".into(), "advertised-environment".into())],
                    None,
                ),
                attempt.clone(),
            ));
            if outcome != "fail" {
                let mut output = String::new();
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        output.push_str(&attempt.poll().output);
                        if output.contains(if outcome == "success" {
                            "ready:advertised-environment:xterm-256color"
                        } else {
                            "waiting"
                        }) {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                match outcome {
                    "success" => attempt.input("yes\n").unwrap(),
                    "cancel" => attempt.cancel.cancel(),
                    "disconnect" => attempt.owner.cancel(),
                    _ => unreachable!("the test's outcomes are success, cancel and disconnect"),
                }
            }
            let result = tokio::time::timeout(Duration::from_secs(5), running)
                .await
                .expect("PTY worker and direct child exited")
                .unwrap();
            assert_eq!(
                result.is_ok(),
                outcome == "success",
                "{outcome}: {result:?}"
            );
            attempt.finish(result);
            assert!(attempt.input("after exit").is_err());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn auth_terminal_cancel_and_disconnect_do_not_wait_for_another_process_groups_slave() {
        use super::super::acp::registry::Launch;
        use std::time::Duration;

        // Always kill the exact descendant reported by our fixture, including on
        // assertion failure. It intentionally escapes the login's process group.
        struct Fixture {
            attempt: Arc<Attempt>,
            descendant: Option<libc::pid_t>,
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                self.attempt.cancel.cancel();
                if let Some(pid) = self.descendant {
                    // SAFETY: kill takes no memory; the pid is the one the fixture captured.
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                    }
                }
            }
        }

        for disconnect in [false, true] {
            for fill_input in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let pids_path = dir.path().join("pids");
                let attempt = Attempt::new(CancellationToken::new());
                let mut fixture = Fixture {
                    attempt: attempt.clone(),
                    descendant: None,
                };
                // Job control puts sleep in a separate group, but the same
                // session. Ignore HUP so closing the master cannot mask the bug.
                // Raw mode and no reader also let us fill the PTY input buffer.
                let code = "trap '' HUP; stty raw -echo; set -m; sleep 60 & printf '%s %s\\n' \"$$\" \"$!\" > \"$PIDS\"; wait";
                let launch = Launch {
                    command: "/bin/sh".into(),
                    args: vec!["-c".into()],
                    env: Vec::new(),
                };
                let running = tokio::spawn(terminal(
                    launch,
                    dir.path().to_path_buf(),
                    vec![code.into()],
                    env_plan(
                        &[],
                        &[
                            term(),
                            ("PIDS".into(), pids_path.to_string_lossy().into_owned()),
                        ],
                        None,
                    ),
                    attempt.clone(),
                ));
                let (leader, descendant) = tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let pids = std::fs::read_to_string(&pids_path)
                            .unwrap_or_default()
                            .split_whitespace()
                            .filter_map(|pid| pid.parse::<libc::pid_t>().ok())
                            .collect::<Vec<_>>();
                        if let [leader, descendant] = pids.as_slice() {
                            fixture.descendant = Some(*descendant);
                            // Wait until the child has actually changed groups.
                            // SAFETY: getpgid only reads the pid it is given.
                            if unsafe { libc::getpgid(*descendant) } == *descendant {
                                break (*leader, *descendant);
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("fixture reported its captured child PIDs");
                assert_ne!(leader, descendant);
                // SAFETY: getpgid and getsid only read the pid they are given.
                assert_eq!(unsafe { libc::getpgid(leader) }, leader);
                // SAFETY: as above.
                assert_eq!(unsafe { libc::getsid(descendant) }, leader);
                if fill_input {
                    let input = "x".repeat(INPUT_LIMIT);
                    // More than the kernel input queue can hold. The bounded
                    // application queue eventually refuses further input too.
                    for _ in 0..40 {
                        let _ = attempt.input(&input);
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    assert!(
                        lock(&attempt.input)
                            .as_ref()
                            .unwrap()
                            .try_send(input)
                            .is_err()
                    );
                }
                attempt.output(b"fixture-secret");
                if disconnect {
                    attempt.owner.cancel();
                } else {
                    attempt.cancel.cancel();
                }
                let result = tokio::time::timeout(Duration::from_secs(2), running)
                    .await
                    .expect("cancellation must settle without slave EOF or writable input")
                    .unwrap();
                assert_eq!(result, Err("Sign-in cancelled.".into()));
                assert!(lock(&attempt.input).is_none());
                assert_eq!(lock(&attempt.status).output, "");
                assert_eq!(attempt.poll().output, "");
                assert!(attempt.input("after cancel").is_err());
                assert_eq!(
                    // SAFETY: signal 0 only checks that the captured pid exists.
                    unsafe { libc::kill(leader, 0) },
                    -1,
                    "direct child was reaped"
                );
                assert_eq!(
                    // SAFETY: signal 0 only checks that the captured pid exists.
                    unsafe { libc::kill(descendant, 0) },
                    0,
                    "escaped slave holder was still alive when cancellation settled"
                );
                // Fixture::drop kills this captured survivor, not a name/path match.
            }
        }
    }
}
