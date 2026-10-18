//! What the CLI harnesses share: the real `hotline` binary on a temporary
//! room, with no display and no session bus, served and stopped by the pid
//! captured when it was spawned; and a person's desktop in this process.
//!
//! Each test binary compiles this module and uses a different part of it, so
//! what one binary leaves unused is allowed, never expected.
#![allow(dead_code, reason = "each test binary uses a different part of common")]

use hotline_core::credentials::{FileStore, SecretStore};
use hotline_core::desk::Desk;
use hotline_core::remote::Remote;
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

const HOTLINE: &str = env!("CARGO_BIN_EXE_hotline");

/// The binary, on `root`. A `serve` that names no `--listen` gets a free
/// loopback address of its own, so it never takes the user's port.
pub fn hotline(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(HOTLINE);
    command
        .args(args)
        .arg("--data")
        .arg(root)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("HOTLINE_DATA_DIR");
    if args.first() == Some(&"serve") && !args.contains(&"--listen") {
        command.args(listen_args(&free_address()));
    }
    command
}

/// `hotline serve` on the file store, with `extra` flags such as
/// `--capabilities` or a `--listen` to keep, once its door is up.
pub fn serve(root: &Path, extra: &[&str]) -> Child {
    let mut args = vec!["serve", "--store", "file"];
    args.extend(extra);
    let child = hotline(root, &args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let door = root.join("door.json");
    until("the server writes door.json", || door.exists());
    child
}

/// How long a harness waits for something before calling it lost: a
/// generous minute, because a loaded runner is slow and a passing test never
/// waits it out.
pub const PATIENCE: Duration = Duration::from_secs(60);

/// Waits until `condition` holds, and fails the test with `what` if it does
/// not within `PATIENCE`. This blocks its thread, so it is for a test whose
/// server is a child process, not one serving on this test's runtime.
pub fn until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < deadline, "never: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Stops a child by the pid captured when it was spawned, and only that.
pub fn stop(mut child: Child) -> Output {
    // SAFETY: a signal to our own child.
    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    until("the child exits on SIGTERM", || {
        child.try_wait().unwrap().is_some()
    });
    child.wait_with_output().unwrap()
}

/// `hotline wire <cmd>` with `params` on its stdin.
pub fn wire(root: &Path, cmd: &str, params: &str) -> Output {
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

/// Everything a command printed, stdout then stderr.
pub fn said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// A loopback address nothing is listening on. The socket is dropped before
/// the child binds it, because a bound socket cannot be handed to a child
/// process, so another process may take the port in between.
pub fn free_address() -> SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

/// The flags that serve on `addr`, for a server served more than once, so a
/// device paired on one boot dials the same place on the next.
pub fn listen_args(addr: &SocketAddr) -> [String; 4] {
    [
        "--listen".into(),
        addr.to_string(),
        "--public-url".into(),
        format!("https://{addr}"),
    ]
}

/// The entries at the top of a data directory, sorted.
pub fn top(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// Waits until the desktop stands in on its chosen relay, and says where.
/// The desktop runs on this test's runtime, so this waits without blocking it.
pub async fn standing(remote: &Remote) -> String {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(url) = remote.status().relay.and_then(|relay| relay.url) {
            return url;
        }
        assert!(
            Instant::now() < deadline,
            "never stood in: {:?}",
            remote.status().relay.and_then(|relay| relay.error)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A person's desktop, in this process: a room, and Remote on.
pub async fn desktop(root: &Path) -> (Arc<Desk>, Arc<Remote>, Arc<dyn SecretStore>) {
    std::fs::create_dir_all(root).unwrap();
    let store: Arc<dyn SecretStore> = Arc::new(FileStore::open(root).unwrap());
    let desk = Arc::new(Desk::open_with_store(root, store.clone()).unwrap());
    // On loopback, on a socket bound here: no network needed, and no other
    // test can take its port first.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let remote = Remote::open_on(
        listener,
        root,
        desk.log.clone(),
        desk.clone(),
        store.clone(),
    )
    .unwrap();
    desk.set_remote(&remote);
    remote.configure(true, "all").await.unwrap();
    (desk, remote, store)
}
