//! What the desktop app prints reaches `logs/hotline.log` when nobody is
//! reading its output (BRO-267).
//!
//! A fresh process, because pointing stdout and stderr at a file would take
//! a test runner's own output with it, and because a launch from the Finder or
//! the Start menu is a process whose stderr is not a terminal from the start.
use hotline_core::log_file::{self, CAP};
use std::process::{Command, Stdio};

fn main() {
    if let Some(root) = std::env::var_os("HOTLINE_LOG_TEST_CHILD") {
        let root = std::path::PathBuf::from(root);
        assert!(
            log_file::redirect(&root).unwrap(),
            "stderr is not a terminal"
        );
        println!("[startup] said on stdout");
        eprintln!("[startup] said on stderr");
        panic!("a panic is kept too");
    }

    let root = std::env::temp_dir().join(format!("hotline-log-file-{}", uuid::Uuid::new_v4()));
    let log = log_file::path(&root);
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    // The last run's log is full, so this launch moves it aside first.
    std::fs::write(&log, vec![b'x'; CAP as usize]).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .env("HOTLINE_LOG_TEST_CHILD", &root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "the child panicked");
    let written = std::fs::read_to_string(&log).unwrap();
    assert!(written.contains("[startup] said on stdout"), "{written}");
    assert!(written.contains("[startup] said on stderr"), "{written}");
    assert!(written.contains("a panic is kept too"), "{written}");
    let previous = log.with_file_name("hotline.log.1");
    assert_eq!(std::fs::metadata(previous).unwrap().len(), CAP);
    std::fs::remove_dir_all(root).unwrap();
    println!("log file: output, panic and rotation at launch passed");
}
