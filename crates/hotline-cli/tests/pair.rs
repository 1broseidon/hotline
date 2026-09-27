//! A pairing invitation belongs to the live CLI connection, not its process id.
#![cfg(unix)]

use std::{
    io::Write,
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const HOTLINE: &str = env!("CARGO_BIN_EXE_hotline");

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn command(root: &Path, args: &[&str]) -> Command {
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
fn eventually(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !condition() {
        assert!(Instant::now() < deadline, "condition did not become true");
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn wire(root: &Path, cmd: &str) -> serde_json::Value {
    let mut child = command(root, &["wire", cmd])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"{}").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn the_pair_cli_opens_a_window_only_while_its_socket_lives() {
    let root = tempfile::tempdir().unwrap();
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let endpoint = format!("https://{address}");
    let mut desk = OwnedChild(
        command(
            root.path(),
            &[
                "serve",
                "--store",
                "file",
                "--listen",
                &address.to_string(),
                "--public-url",
                &endpoint,
            ],
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap(),
    );
    eventually(|| {
        assert!(
            desk.0.try_wait().unwrap().is_none(),
            "desk exited before startup"
        );
        root.path().join("door.json").exists()
    });
    eventually(|| {
        !wire(root.path(), "remote.status")["endpoints"]
            .as_array()
            .unwrap()
            .is_empty()
    });

    let runtime = tokio::runtime::Runtime::new().unwrap();
    // Only this isolated, freshly generated test certificate is untrusted.
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let status = |path: &str| {
        runtime.block_on(async {
            client
                .get(format!("{endpoint}{path}"))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        })
    };
    assert_eq!(status("/v2/pair"), 404);
    assert_eq!(status("/pair"), 404);
    assert_eq!(status("/pair/manual/start"), 404);
    let devices = command(root.path(), &["devices"]).output().unwrap();
    assert!(devices.status.success());
    assert!(String::from_utf8_lossy(&devices.stdout).contains("No paired devices"));

    // SIGINT cancels explicitly; SIGKILL proves socket teardown alone is enough.
    for signal in [libc::SIGINT, libc::SIGKILL] {
        let mut pair = OwnedChild(
            command(root.path(), &["pair", "--companion"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        eventually(|| {
            assert!(
                pair.0.try_wait().unwrap().is_none(),
                "pair command exited early"
            );
            status("/v2/pair") == 400 // Open route refuses a request without WS upgrade.
        });
        // SAFETY: only the pid captured when this test spawned the CLI.
        assert_eq!(unsafe { libc::kill(pair.0.id() as libc::pid_t, signal) }, 0);
        eventually(|| pair.0.try_wait().unwrap().is_some());
        eventually(|| status("/v2/pair") == 404);
    }
    assert!(
        wire(root.path(), "remote.devices")
            .as_array()
            .unwrap()
            .is_empty()
    );
}
