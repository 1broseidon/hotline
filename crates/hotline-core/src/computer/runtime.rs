//! Container runtime detection.
//!
//! Hotline shells out to the runtime CLI — docker and podman agree on the
//! `create`/`start`/`stop`/`rm`/`inspect` subset we need — and takes no SDK
//! dependency. Every candidate reports a state — ready, or one of the ways
//! of not being — with the runtime's own words kept beside it, and rootless
//! runtimes rank first so a machine that has both prefers the one that is
//! not a root daemon.

use crate::contract::{ComputerRuntime, RuntimeReport, RuntimeState};
use crate::paths;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// How long a probe that hangs (a wedged daemon) may take. A session start
/// should fail in seconds, not sit on `docker info` forever.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The CLIs Hotline knows how to drive. `command` is the binary name on PATH.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runtime {
    Docker,
    Podman,
    AppleContainer,
}

impl Runtime {
    pub fn command(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
            Self::AppleContainer => "container",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Docker => "Docker",
            Self::Podman => "Podman",
            Self::AppleContainer => "Apple container",
        }
    }

    pub fn wire(self) -> ComputerRuntime {
        match self {
            Self::Docker => ComputerRuntime::Docker,
            Self::Podman => ComputerRuntime::Podman,
            Self::AppleContainer => ComputerRuntime::Container,
        }
    }

    pub fn from_wire(runtime: ComputerRuntime) -> Self {
        match runtime {
            ComputerRuntime::Docker => Self::Docker,
            ComputerRuntime::Podman => Self::Podman,
            ComputerRuntime::Container => Self::AppleContainer,
        }
    }

    pub fn from_setting(value: &str) -> Option<Self> {
        match value {
            "docker" => Some(Self::Docker),
            "podman" => Some(Self::Podman),
            "container" => Some(Self::AppleContainer),
            _ => None,
        }
    }

    /// Every runtime this build can drive. Apple's container exists only
    /// on macOS, so a Linux or Windows desk never lists it, not even as
    /// unsupported: a row for a runtime the machine cannot have is noise.
    #[cfg(target_os = "macos")]
    fn all() -> &'static [Self] {
        &[Self::Docker, Self::Podman, Self::AppleContainer]
    }

    #[cfg(not(target_os = "macos"))]
    fn all() -> &'static [Self] {
        &[Self::Docker, Self::Podman]
    }
}

/// Where to look for runtime binaries. Production uses `PATH` plus the
/// packaged-Mac directories; tests hand in a directory of fake scripts and
/// turn extras off so a real Docker on this machine cannot leak in.
#[derive(Clone, Debug)]
pub struct BinSearch {
    path: Option<OsString>,
    extras: bool,
}

impl BinSearch {
    pub fn from_env() -> Self {
        Self {
            path: std::env::var_os("PATH"),
            extras: true,
        }
    }

    pub fn only(path: impl Into<OsString>) -> Self {
        Self {
            path: Some(path.into()),
            extras: false,
        }
    }

    pub fn resolve(&self, command: &str) -> Option<PathBuf> {
        paths::resolve_command_in(command, self.path.as_deref(), self.extras)
    }
}

/// Every runtime Hotline knows, rootless-available first, then available, then
/// the absentees with their reasons.
pub async fn detect() -> Vec<RuntimeReport> {
    detect_with(&BinSearch::from_env()).await
}

pub async fn detect_with(bins: &BinSearch) -> Vec<RuntimeReport> {
    let mut reports = Vec::with_capacity(3);
    for runtime in Runtime::all() {
        reports.push(probe(*runtime, bins).await);
    }
    reports.sort_by(|left, right| {
        right
            .state
            .ready()
            .cmp(&left.state.ready())
            .then(right.rootless.cmp(&left.rootless))
    });
    reports
}

async fn probe(runtime: Runtime, bins: &BinSearch) -> RuntimeReport {
    let report = |state: RuntimeState, detail: Option<String>, rootless: bool| RuntimeReport {
        runtime: runtime.wire(),
        state,
        detail,
        rootless,
    };

    if runtime == Runtime::AppleContainer && !cfg!(target_os = "macos") {
        return report(RuntimeState::Unsupported, None, false);
    }

    let Some(cmd) = bins.resolve(runtime.command()) else {
        return report(RuntimeState::NotInstalled, None, false);
    };

    // docker and podman answer `version` from the daemon, so one call says
    // installed and running. Apple's container has no `version` subcommand
    // — its CLI calls an unknown one a missing plugin — and `ls` is the
    // cheapest call that goes through its services.
    let ask: &[&str] = match runtime {
        Runtime::AppleContainer => &["ls"],
        Runtime::Docker | Runtime::Podman => &["version"],
    };
    match output(&cmd, ask).await {
        Ok(_) => {}
        Err(failure) => return report(failure.state, Some(failure.detail), false),
    }

    let rootless = match runtime {
        Runtime::Docker => output(&cmd, &["info", "--format", "{{json .SecurityOptions}}"])
            .await
            .ok()
            .is_some_and(|out| out.contains("rootless")),
        Runtime::Podman => output(&cmd, &["info", "--format", "{{.Host.Security.Rootless}}"])
            .await
            .ok()
            .is_some_and(|out| out == "true"),
        // Apple's container runs each container in its own lightweight VM;
        // there is no root daemon on the host to be rootless relative to.
        Runtime::AppleContainer => true,
    };
    report(RuntimeState::Ready, None, rootless)
}

/// How a probe fell short, and the words it fell short with: the runtime's
/// own where it had any, a sentence of ours where it did not.
struct Failure {
    state: RuntimeState,
    detail: String,
}

/// A CLI that is installed but whose daemon or services are down says so in
/// its own words; these are the ones the three runtimes use, so the window
/// can say "not running" instead of quoting them. Anything else the CLI
/// says is a failure the person has to read.
fn looks_stopped(said: &str) -> bool {
    let said = said.to_ascii_lowercase();
    [
        "cannot connect to the docker daemon",
        "is the docker daemon running",
        "failed to connect to the docker api",
        "cannot connect to podman",
        "connection refused",
        "connect: no such file or directory",
        "system services are not running",
        "not running",
    ]
    .iter()
    .any(|sign| said.contains(sign))
}

async fn output(cmd: &std::path::Path, args: &[&str]) -> Result<String, Failure> {
    let child = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| Failure {
            state: RuntimeState::Failed,
            detail: format!("{} could not be started: {error}", cmd.display()),
        })?;
    match tokio::time::timeout(PROBE_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let said = match (stderr.trim(), stdout.trim()) {
                ("", "") => "",
                ("", out) => out,
                (err, _) => err,
            };
            if said.is_empty() {
                return Err(Failure {
                    state: RuntimeState::NotResponding,
                    detail: format!(
                        "{} version exited with {} and said nothing",
                        cmd.file_name()
                            .unwrap_or_else(|| OsStr::new("runtime"))
                            .to_string_lossy(),
                        output.status
                    ),
                });
            }
            Err(Failure {
                state: if looks_stopped(said) {
                    RuntimeState::NotRunning
                } else {
                    RuntimeState::Failed
                },
                detail: said.to_string(),
            })
        }
        Ok(Err(error)) => Err(Failure {
            state: RuntimeState::Failed,
            detail: format!("{} could not be started: {error}", cmd.display()),
        }),
        Err(_) => Err(Failure {
            state: RuntimeState::NotResponding,
            detail: format!(
                "{} version did not answer within {} seconds",
                cmd.display(),
                PROBE_TIMEOUT.as_secs()
            ),
        }),
    }
}

// These fixtures execute POSIX shell scripts; native process jobs have Windows coverage.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hotline-core-runtime-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// `detect_with`, retried while a report says "Text file busy". A test in
    /// another thread forking at the instant this one's script was still
    /// open for writing hands its child that descriptor until it execs, and
    /// Linux refuses to run a file anyone holds open for writing. The window
    /// is microseconds wide and the retry is the honest fix for a test suite
    /// that writes executables while other tests spawn.
    /// The state a CLI's words land in, without a CLI.
    fn output_state(said: &str) -> RuntimeState {
        if looks_stopped(said) {
            RuntimeState::NotRunning
        } else {
            RuntimeState::Failed
        }
    }

    async fn detect_settled(search: &BinSearch) -> Vec<RuntimeReport> {
        let mut reports = detect_with(search).await;
        for _ in 0..5 {
            let busy = reports.iter().any(|report| {
                report
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("Text file busy"))
            });
            if !busy {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            reports = detect_with(search).await;
        }
        reports
    }

    fn write_script(dir: &Path, name: &str, body: &str) {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A Linux or Windows desk never lists Apple's container, not even as
    /// unsupported: the row would be for a runtime this machine cannot have.
    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn only_runtimes_this_build_can_drive_are_listed() {
        let path = scratch("gate");
        let reports = detect_settled(&BinSearch::only(path.into_os_string())).await;
        let listed: Vec<ComputerRuntime> = reports.iter().map(|report| report.runtime).collect();
        assert_eq!(listed, [ComputerRuntime::Docker, ComputerRuntime::Podman]);
        assert!(
            reports
                .iter()
                .all(|report| report.state == RuntimeState::NotInstalled)
        );
    }

    #[tokio::test]
    async fn a_script_that_answers_version_is_ready_and_a_failing_one_carries_its_words() {
        let path = scratch("probe");
        write_script(
            &path,
            "docker",
            r#"#!/bin/sh
if [ "$1" = version ]; then echo ok; exit 0; fi
if [ "$1" = info ]; then echo '[]'; exit 0; fi
exit 1
"#,
        );
        write_script(
            &path,
            "podman",
            r#"#!/bin/sh
echo "daemon exploded" >&2
exit 1
"#,
        );

        let reports = detect_settled(&BinSearch::only(path.into_os_string())).await;
        let docker = reports
            .iter()
            .find(|report| report.runtime == ComputerRuntime::Docker)
            .unwrap();
        assert_eq!(docker.state, RuntimeState::Ready, "{docker:?}");
        assert!(!docker.rootless);
        assert_eq!(docker.detail, None);

        let podman = reports
            .iter()
            .find(|report| report.runtime == ComputerRuntime::Podman)
            .unwrap();
        assert_eq!(podman.state, RuntimeState::Failed, "{podman:?}");
        assert!(
            podman
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("daemon exploded")),
            "{podman:?}"
        );
    }

    #[tokio::test]
    async fn rootless_runtimes_rank_first() {
        let path = scratch("rank");
        write_script(
            &path,
            "docker",
            r#"#!/bin/sh
if [ "$1" = version ]; then echo ok; exit 0; fi
if [ "$1" = info ]; then echo '[]'; exit 0; fi
exit 1
"#,
        );
        write_script(
            &path,
            "podman",
            r#"#!/bin/sh
if [ "$1" = version ]; then echo ok; exit 0; fi
if [ "$1" = info ]; then echo true; exit 0; fi
exit 1
"#,
        );

        let reports = detect_settled(&BinSearch::only(path.into_os_string())).await;
        let available: Vec<_> = reports
            .iter()
            .filter(|report| report.state.ready())
            .map(|report| report.runtime)
            .collect();
        assert_eq!(
            available,
            [ComputerRuntime::Podman, ComputerRuntime::Docker],
            "{reports:?}"
        );
        assert!(reports[0].rootless);
        assert!(!reports[1].rootless);
    }

    #[tokio::test]
    async fn a_missing_binary_is_not_installed_and_a_down_daemon_is_not_running() {
        let path = scratch("missing");
        let reports = detect_settled(&BinSearch::only(path.into_os_string())).await;
        let docker = reports
            .iter()
            .find(|report| report.runtime == ComputerRuntime::Docker)
            .unwrap();
        assert_eq!(docker.state, RuntimeState::NotInstalled);
        assert_eq!(docker.detail, None);
        assert_eq!(
            output_state(
                "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?"
            ),
            RuntimeState::NotRunning
        );
        assert_eq!(
            output_state(
                "Error: Plugin 'container-version' not found. - If system services are not running, start them with: container system start"
            ),
            RuntimeState::NotRunning
        );
        assert_eq!(output_state("daemon exploded"), RuntimeState::Failed);
    }
}
