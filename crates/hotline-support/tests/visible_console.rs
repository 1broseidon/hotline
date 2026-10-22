#![cfg(windows)]

use hotline_support::log_file;
use hotline_support::process_windows::{Job, prepare_visible};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

const NAME: &str = "a_sign_in_console_has_its_own_handles_not_the_apps_log";
const CHILD: &str = "HOTLINE_VISIBLE_CONSOLE_ROOT";
const REPORT: &str = "HOTLINE_VISIBLE_CONSOLE_REPORT";
const SAID: &str = "said in the sign-in console";
const AGAIN: [&str; 4] = ["--exact", NAME, "--include-ignored", "--nocapture"];

#[tokio::test]
#[ignore = "needs a console window"]
async fn a_sign_in_console_has_its_own_handles_not_the_apps_log() {
    if let Some(report) = std::env::var_os(REPORT) {
        let terminals = [
            std::io::stdin().is_terminal(),
            std::io::stdout().is_terminal(),
            std::io::stderr().is_terminal(),
        ];
        std::fs::write(report, format!("{terminals:?}")).unwrap();
        println!("{SAID}");
        eprintln!("{SAID}");
        return;
    }
    if let Some(root) = std::env::var_os(CHILD) {
        let root = PathBuf::from(root);
        assert!(log_file::redirect(&root).unwrap(), "stderr is a terminal");
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command.args(AGAIN).env(REPORT, root.join("report.txt"));
        prepare_visible(&mut command);
        let mut child = command.spawn().unwrap();
        let _job = Job::attach_without_kill_on_close(child.id()).unwrap();
        let status = tokio::time::timeout(Duration::from_secs(60), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success(), "{status}");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(AGAIN)
        .env(CHILD, root.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    let log = std::fs::read_to_string(log_file::path(root.path())).unwrap_or_default();
    assert!(status.success(), "{log}");
    assert_eq!(
        std::fs::read_to_string(root.path().join("report.txt")).unwrap(),
        "[true, true, true]",
        "{log}"
    );
    assert!(!log.contains(SAID), "{log}");
}
