//! The real `hotline` binary, on a temporary room, with no display and no
//! session bus: it serves, keeps its room to itself, keeps a key in the file
//! store across a restart, and stops cleanly on SIGTERM.

use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const HOTLINE: &str = env!("CARGO_BIN_EXE_hotline");

fn hotline(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(HOTLINE);
    command
        .args(args)
        .arg("--data")
        .arg(root)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("HOTLINE_DATA_DIR");
    command
}

fn serve(root: &Path) -> Child {
    let child = hotline(root, &["serve", "--store", "file"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let door = root.join("door.json");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !door.exists() {
        assert!(Instant::now() < deadline, "the desk never wrote door.json");
        std::thread::sleep(Duration::from_millis(100));
    }
    child
}

fn wire(root: &Path, cmd: &str, params: &str) -> Output {
    let mut child = hotline(root, &["wire", cmd])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(params.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn stop(mut child: Child) -> Output {
    // SAFETY: a signal to our own child.
    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "the desk did not stop on SIGTERM"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_served_desk_keeps_its_room_its_keys_and_stops_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("room");

    let desk = serve(&root);
    let status = hotline(&root, &["status"]).output().unwrap();
    assert!(status.status.success(), "{status:?}");
    let said = String::from_utf8_lossy(&status.stdout);
    assert!(
        said.contains("running") && said.contains("store   file"),
        "{said}"
    );

    // A second desk on the same room is refused while the first runs.
    let second = hotline(&root, &["serve", "--store", "file"])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&second.stderr).contains("Another Hotline desk"));

    let saved = wire(
        &root,
        "credential.create",
        r#"{"providerId":"openrouter","label":"harness","secret":"sk-or-harness-0123456789"}"#,
    );
    assert!(saved.status.success(), "{saved:?}");

    let stopped = stop(desk);
    assert!(stopped.status.success(), "{stopped:?}");
    assert!(!root.join("door.json").exists());
    let down = hotline(&root, &["status"]).output().unwrap();
    assert_eq!(down.status.code(), Some(3));

    // The key is only in the file store, and survives the restart.
    for entry in walk(&root) {
        if entry.starts_with(root.join("secrets")) {
            continue;
        }
        let bytes = std::fs::read(&entry).unwrap_or_default();
        assert!(
            !String::from_utf8_lossy(&bytes).contains("sk-or-harness"),
            "{} holds the key in the clear",
            entry.display()
        );
    }
    let desk = serve(&root);
    let listed = wire(&root, "credential.list", "{}");
    assert!(String::from_utf8_lossy(&listed.stdout).contains("harness"));
    let status = hotline(&root, &["status"]).output().unwrap();
    assert!(String::from_utf8_lossy(&status.stdout).contains("OpenRouter"));
    assert!(stop(desk).status.success());

    // Started on the other store, the room is refused rather than emptied.
    let native = hotline(&root, &["serve", "--store", "native"])
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&native.stderr).contains("file store"));
}

fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}
