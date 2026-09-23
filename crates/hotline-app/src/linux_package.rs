//! Installs a downloaded .deb or .rpm update with the system's own pkexec.
//!
//! The updater plugin runs `pkexec` by bare name. Hotline's PATH is the login
//! shell's, so on a machine with Homebrew's polkit that finds a copy that
//! is not setuid and fails. The plugin then pipes a password into `sudo`,
//! and as a last resort runs `sudo` against the desktop session's own
//! terminal, which nobody can type into. That sudo waits forever, and every
//! later update queues behind it. So Hotline installs these packages itself:
//! the system binaries by absolute path, pkexec only, and a time limit.
//! The bytes arrive already verified against the update's signature.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Long enough to read the password prompt and install; short enough that a
/// stuck installer comes back as an error rather than a spinner.
pub const INSTALL_WAIT: Duration = Duration::from_secs(10 * 60);

/// pkexec's own exit codes: the dialog was dismissed, or authorization
/// failed, which includes a pkexec that cannot become root at all.
const DISMISSED: i32 = 126;
const NOT_AUTHORIZED: i32 = 127;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Package {
    Deb,
    Rpm,
}

impl Package {
    /// The package kind for an updater target such as `linux-x86_64-deb`.
    pub fn for_target(target: &str) -> Option<Self> {
        if target.ends_with("-deb") {
            Some(Self::Deb)
        } else if target.ends_with("-rpm") {
            Some(Self::Rpm)
        } else {
            None
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Deb => "deb",
            Self::Rpm => "rpm",
        }
    }

    fn looks_right(self, bytes: &[u8]) -> bool {
        match self {
            Self::Deb => bytes.starts_with(b"!<arch>\ndebian-binary"),
            Self::Rpm => bytes.starts_with(&[0xed, 0xab, 0xee, 0xdb]),
        }
    }

    fn installer(self) -> (&'static [&'static str], &'static str) {
        match self {
            Self::Deb => (&["/usr/bin/dpkg", "/bin/dpkg"], "-i"),
            Self::Rpm => (&["/usr/bin/rpm", "/bin/rpm"], "-U"),
        }
    }
}

/// The system programs one install uses, found by absolute path only.
struct Programs {
    pkexec: PathBuf,
    installer: PathBuf,
    argument: &'static str,
}

impl Programs {
    fn system(package: Package) -> Result<Self, String> {
        let (installers, argument) = package.installer();
        let pkexec = first_file(&["/usr/bin/pkexec", "/bin/pkexec"]).ok_or(
            "Hotline cannot ask for your password because pkexec is not installed. \
             Install polkit, or install this update from the release page with your package manager.",
        )?;
        let installer = first_file(installers).ok_or(
            "Hotline cannot find the system package manager to install this update. \
             Install it from the release page instead.",
        )?;
        Ok(Self {
            pkexec,
            installer,
            argument,
        })
    }
}

fn first_file(paths: &[&str]) -> Option<PathBuf> {
    paths.iter().map(PathBuf::from).find(|path| path.is_file())
}

pub fn install(package: Package, bytes: &[u8]) -> Result<(), String> {
    install_with(&Programs::system(package)?, package, bytes, INSTALL_WAIT)
}

fn install_with(
    programs: &Programs,
    package: Package,
    bytes: &[u8],
    wait: Duration,
) -> Result<(), String> {
    if !package.looks_right(bytes) {
        return Err(format!(
            "The downloaded update is not a .{} package. Hotline has not been changed.",
            package.extension()
        ));
    }
    let scratch = tempfile::Builder::new()
        .prefix("hotline-update")
        .tempdir()
        .map_err(|error| format!("Could not save the update: {error}"))?;
    let file = scratch
        .path()
        .join(format!("hotline.{}", package.extension()));
    std::fs::write(&file, bytes).map_err(|error| format!("Could not save the update: {error}"))?;
    // A file, not a pipe: nobody reads while the installer runs, and a full
    // pipe would stall it.
    let errors_path = scratch.path().join("installer.log");
    let errors = std::fs::File::create(&errors_path)
        .map_err(|error| format!("Could not start the installer: {error}"))?;
    let mut child = Command::new(&programs.pkexec)
        .arg(&programs.installer)
        .arg(programs.argument)
        .arg(&file)
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(errors)
        .spawn()
        .map_err(|error| format!("Could not start the installer: {error}"))?;
    let deadline = Instant::now() + wait;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "The installer did not finish within {} minutes, so Hotline stopped waiting. \
                     Try again, or install the update from the release page.",
                    wait.as_secs() / 60
                ));
            }
            Err(error) => return Err(format!("Could not follow the installer: {error}")),
        }
    };
    if status.success() {
        return Ok(());
    }
    let said = last_words(&errors_path);
    Err(match status.code() {
        Some(DISMISSED) => "Installation cancelled. Hotline has not been changed.".to_string(),
        Some(NOT_AUTHORIZED) => with_detail(
            "Hotline did not get permission to install the update. Hotline has not been changed.",
            &said,
        ),
        _ => with_detail(
            "The package manager could not install the update. Hotline is still running; try again.",
            &said,
        ),
    })
}

/// The installer's last line of complaint, which is the one that says why.
fn last_words(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect()
}

fn with_detail(sentence: &str, detail: &str) -> String {
    if detail.is_empty() {
        sentence.to_string()
    } else {
        format!("{sentence} {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const DEB: &[u8] = b"!<arch>\ndebian-binary   rest of the package";

    /// A pkexec stand-in that runs what it is given, and an installer that
    /// behaves as `body` says, recording its arguments beside itself.
    fn programs(dir: &Path, body: &str) -> Programs {
        let script = |name: &str, text: &str| {
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{text}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        };
        Programs {
            pkexec: script("pkexec", "exec \"$@\""),
            installer: script(
                "dpkg",
                &format!("echo \"$@\" > \"{}/args\"\n{body}", dir.display()),
            ),
            argument: "-i",
        }
    }

    #[test]
    fn a_package_is_installed_by_the_given_programs_with_its_saved_file() {
        let dir = tempfile::tempdir().unwrap();
        let programs = programs(dir.path(), "exit 0");
        install_with(&programs, Package::Deb, DEB, Duration::from_secs(5)).unwrap();
        let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
        assert!(args.starts_with("-i /"), "{args}");
        assert!(args.trim_end().ends_with("/hotline.deb"), "{args}");
    }

    #[test]
    fn a_dismissed_prompt_a_refusal_and_a_failed_install_each_say_so() {
        let dir = tempfile::tempdir().unwrap();
        let run = |body: &str| {
            install_with(
                &programs(dir.path(), body),
                Package::Deb,
                DEB,
                Duration::from_secs(5),
            )
            .unwrap_err()
        };
        assert_eq!(
            run("exit 126"),
            "Installation cancelled. Hotline has not been changed."
        );
        assert!(
            run("echo 'pkexec must be setuid root' >&2; exit 127")
                .ends_with("pkexec must be setuid root")
        );
        let failed = run("echo 'dpkg: error: dpkg frontend lock is locked' >&2; exit 2");
        assert!(
            failed.starts_with("The package manager could not install"),
            "{failed}"
        );
        assert!(failed.ends_with("dpkg frontend lock is locked"), "{failed}");
    }

    #[test]
    fn an_installer_that_never_returns_is_given_up_on() {
        let dir = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let error = install_with(
            &programs(dir.path(), "sleep 30"),
            Package::Deb,
            DEB,
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(error.starts_with("The installer did not finish"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn bytes_that_are_not_the_expected_package_are_refused_before_asking() {
        let dir = tempfile::tempdir().unwrap();
        let error = install_with(
            &programs(dir.path(), "exit 0"),
            Package::Rpm,
            DEB,
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(error.contains("not a .rpm package"), "{error}");
        assert!(!dir.path().join("args").exists(), "the installer never ran");
    }

    #[test]
    fn only_deb_and_rpm_targets_are_installed_here() {
        assert_eq!(Package::for_target("linux-x86_64-deb"), Some(Package::Deb));
        assert_eq!(Package::for_target("linux-x86_64-rpm"), Some(Package::Rpm));
        assert_eq!(Package::for_target("linux-x86_64-appimage"), None);
    }
}
