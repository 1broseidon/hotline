//! The update command's public parser and read-only path. No network, sudo,
//! installed binaries, service manager, or real room is needed by these tests.
use std::process::Command;

const HOTLINE: &str = env!("CARGO_BIN_EXE_hotline");

#[test]
fn the_binary_reports_its_version_without_opening_a_room() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(HOTLINE)
        .arg("--version")
        .env("HOTLINE_DATA_DIR", root.path().join("must-not-exist"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("hotline {}", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn checking_an_explicit_version_is_read_only_and_needs_no_service() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(HOTLINE)
        .args(["update", "--check", "--version", env!("CARGO_PKG_VERSION")])
        .env("HOTLINE_DATA_DIR", root.path().join("must-not-exist"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains(env!("CARGO_PKG_VERSION")));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn update_rejects_unknown_flags_and_non_release_versions() {
    for args in [
        vec!["--version"],
        vec!["--version", "../../other"],
        vec!["--version", "1.2"],
        vec!["--version", "--check"],
        vec!["--data", "/tmp/not-a-room"],
        vec!["--force"],
    ] {
        let output = Command::new(HOTLINE)
            .arg("update")
            .args(&args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{args:?}: {output:?}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn an_unmanaged_binary_cannot_self_replace_or_restart_a_desk() {
    let output = Command::new(HOTLINE)
        .args(["update", "--version", "999.0.0"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("package manager") && error.contains("desktop"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn help_lists_the_update_command() {
    let output = Command::new(HOTLINE).arg("--help").output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("hotline update [--check]"));
}
