//! Container runtime detection.
//!
//! Toad shells out to the runtime CLI — docker and podman agree on the
//! `create`/`start`/`stop`/`rm`/`inspect` subset we need — and takes no SDK
//! dependency. Every candidate reports `available` or a `reason`, and
//! rootless runtimes rank first so a machine that has both prefers the one
//! that is not a root daemon.

use crate::contract::{ComputerRuntime, RuntimeReport};
use crate::paths;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// How long a probe that hangs (a wedged daemon) may take. A session start
/// should fail in seconds, not sit on `docker info` forever.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The CLIs Toad knows how to drive. `command` is the binary name on PATH.
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

    fn all() -> [Self; 3] {
        [Self::Docker, Self::Podman, Self::AppleContainer]
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

/// Every runtime Toad knows, rootless-available first, then available, then
/// the absentees with their reasons.
pub async fn detect() -> Vec<RuntimeReport> {
    detect_with(&BinSearch::from_env()).await
}

pub async fn detect_with(bins: &BinSearch) -> Vec<RuntimeReport> {
    let mut reports = Vec::with_capacity(3);
    for runtime in Runtime::all() {
        reports.push(probe(runtime, bins).await);
    }
    reports.sort_by(|left, right| {
        right
            .available
            .cmp(&left.available)
            .then(right.rootless.cmp(&left.rootless))
    });
    reports
}

async fn probe(runtime: Runtime, bins: &BinSearch) -> RuntimeReport {
    let report = |available: bool, reason: Option<String>, rootless: bool| RuntimeReport {
        runtime: runtime.wire(),
        available,
        reason,
        rootless,
    };

    if runtime == Runtime::AppleContainer && !cfg!(target_os = "macos") {
        return report(false, Some("macOS only".into()), false);
    }

    let Some(cmd) = bins.resolve(runtime.command()) else {
        return report(
            false,
            Some(format!("{} not found on PATH", runtime.command())),
            false,
        );
    };

    match output(&cmd, &["version"]).await {
        Ok(_) => {}
        Err(reason) => return report(false, Some(reason), false),
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
    report(true, None, rootless)
}

async fn output(cmd: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let child = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("{} could not be started: {error}", cmd.display()))?;
    match tokio::time::timeout(PROBE_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = stderr.trim();
            if !detail.is_empty() {
                return Err(detail.to_string());
            }
            let detail = stdout.trim();
            if !detail.is_empty() {
                return Err(detail.to_string());
            }
            Err(format!(
                "{} not responding",
                cmd.file_name()
                    .unwrap_or_else(|| OsStr::new("runtime"))
                    .to_string_lossy()
            ))
        }
        Ok(Err(error)) => Err(format!("{} could not be started: {error}", cmd.display())),
        Err(_) => Err(format!("{} not responding", cmd.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "toad-core-runtime-{name}-{}-{}",
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
    async fn detect_settled(search: &BinSearch) -> Vec<RuntimeReport> {
        let mut reports = detect_with(search).await;
        for _ in 0..5 {
            let busy = reports.iter().any(|report| {
                report
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("Text file busy"))
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

    #[tokio::test]
    async fn a_script_that_answers_version_is_available_and_a_failing_one_carries_the_reason() {
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
        assert!(docker.available, "{docker:?}");
        assert!(!docker.rootless);
        assert_eq!(docker.reason, None);

        let podman = reports
            .iter()
            .find(|report| report.runtime == ComputerRuntime::Podman)
            .unwrap();
        assert!(!podman.available, "{podman:?}");
        assert!(
            podman
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("daemon exploded")),
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
            .filter(|report| report.available)
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
    async fn a_missing_binary_is_unavailable_with_the_path_reason() {
        let path = scratch("missing");
        let reports = detect_settled(&BinSearch::only(path.into_os_string())).await;
        let docker = reports
            .iter()
            .find(|report| report.runtime == ComputerRuntime::Docker)
            .unwrap();
        assert!(!docker.available);
        assert_eq!(docker.reason.as_deref(), Some("docker not found on PATH"));
    }
}
