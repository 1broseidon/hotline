//! What more than one harness needs: disposable credentials that never touch
//! the operator's keychain, and the window's half of the wire.
//!
//! Each test binary compiles this module and uses a different part of it, so
//! what one binary leaves unused is allowed, never expected.
use futures_util::{SinkExt, StreamExt};
use hotline_core::{credentials::SecretStore, desk::Desk, log::Log, vault::Vault};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    io,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

#[derive(Default)]
struct MemoryStore(Mutex<HashMap<String, Vec<u8>>>);
impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.0.lock().unwrap().insert(key.into(), bytes.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> io::Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

#[allow(dead_code, reason = "each test binary uses a different part of common")]
pub fn store() -> Arc<dyn SecretStore> {
    static STORE: OnceLock<Arc<MemoryStore>> = OnceLock::new();
    STORE.get_or_init(Default::default).clone()
}

#[allow(dead_code, reason = "each test binary uses a different part of common")]
pub fn open_desk(root: &Path) -> io::Result<Desk> {
    Desk::open_with_store(root, store())
}

#[allow(dead_code, reason = "each test binary uses a different part of common")]
pub fn open_vault(root: &Path, log: Log) -> io::Result<Vault> {
    Vault::open_with_store(root, log, store())
}

#[cfg(unix)]
pub mod stalled;

#[allow(
    dead_code,
    reason = "only the harnesses that run an ACP teammate use it"
)]
pub mod acp_fixture;

/// Fails the test by name when `tool` cannot be started here. A harness whose
/// command runs an interpreter would otherwise time out waiting on what the
/// command was meant to do, and say nothing about the interpreter.
#[allow(dead_code, reason = "each test binary uses a different part of common")]
pub fn require(tool: &str) {
    let ran = std::process::Command::new(tool).arg("--version").output();
    assert!(
        ran.is_ok_and(|output| output.status.success()),
        "needs {tool}: it is not on PATH or does not run"
    );
}

/// Stands in for the system's link opener, so a teammate's `open_link` in a
/// harness opens no tab on the machine running it. The opener the core runs
/// (`open`, or `xdg-open`) is found on `PATH`; a script by that name goes
/// first on it and writes each link it is handed to the returned file.
#[cfg(unix)]
#[allow(dead_code, reason = "each test binary uses a different part of common")]
pub fn browser() -> &'static Path {
    static OPENED: OnceLock<std::path::PathBuf> = OnceLock::new();
    OPENED.get_or_init(|| {
        use std::os::unix::fs::PermissionsExt;
        let bin = tempfile::tempdir().unwrap().keep();
        let opened = bin.join("opened");
        let script = format!("#!/bin/sh\necho \"$1\" >> '{}'\n", opened.display());
        for name in ["open", "xdg-open"] {
            let path = bin.join(name);
            std::fs::write(&path, &script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::var("PATH").unwrap_or_default();
        // Safety: std takes its environment lock for this and for every read
        // it makes, Command's PATH lookup included, and nothing in these
        // harnesses reads the environment through libc directly.
        unsafe { std::env::set_var("PATH", format!("{}:{path}", bin.display())) };
        opened
    })
}

/// How long a command's reply may take before the test calls it lost. A reply
/// that never comes fails the test rather than hanging its binary.
#[allow(dead_code, reason = "each test binary uses a different part of common")]
pub const ANSWER: Duration = Duration::from_secs(60);

/// Waits until `condition` holds, and fails the test with `what` if it does
/// not within `ANSWER`. A harness waits on the signal itself this way, never
/// on a sleep it hopes is long enough.
#[allow(dead_code, reason = "each test binary uses a different part of common")]
pub async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + ANSWER;
    while !condition() {
        assert!(tokio::time::Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The window's half of the wire, as a harness plays it. Every wait is
/// bounded.
#[allow(dead_code, reason = "each test binary uses a different part of common")]
pub struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: i64,
    /// Frames that arrived while waiting for something else: a subscription's
    /// snapshot or event lands whenever the room has one.
    pub inbox: VecDeque<Value>,
}

#[allow(dead_code, reason = "each test binary uses a different part of common")]
impl Client {
    pub async fn connect(port: u16, token: &str) -> Client {
        let (socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token={token}"))
            .await
            .unwrap();
        Client {
            socket,
            next_id: 1,
            inbox: VecDeque::new(),
        }
    }

    async fn read(&mut self) -> Value {
        loop {
            match self.socket.next().await.expect("the door closed") {
                Ok(Message::Text(text)) => return serde_json::from_str(&text).unwrap(),
                Ok(_) => continue,
                Err(error) => panic!("the socket failed: {error}"),
            }
        }
    }

    async fn write(&mut self, mut frame: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        frame["id"] = json!(id);
        self.socket
            .send(Message::text(frame.to_string()))
            .await
            .unwrap();
        id
    }

    /// Sends a command and answers its id without waiting for the reply.
    pub async fn send(&mut self, cmd: &str, params: Value) -> i64 {
        self.write(json!({ "cmd": cmd, "params": params })).await
    }

    /// A command, answered within `ANSWER`. Frames for subscriptions that
    /// arrive first are kept for `next_where`.
    pub async fn call(&mut self, cmd: &str, params: Value) -> Value {
        let id = self.send(cmd, params).await;
        self.reply(id, ANSWER).await
    }

    /// The reply to `id`, within `patience`, for a test whose point is how
    /// long the reply takes.
    pub async fn reply(&mut self, id: i64, patience: Duration) -> Value {
        self.next_where(patience, |frame| {
            frame.get("id").and_then(Value::as_i64) == Some(id) && frame.get("sub").is_none()
        })
        .await
    }

    /// Subscribes to `target` and answers the subscription's id once the door
    /// has accepted it.
    pub async fn subscribe(&mut self, target: Value) -> i64 {
        let id = self.write(json!({ "sub": target })).await;
        let accepted = self.reply(id, ANSWER).await;
        assert_eq!(accepted["ok"], true, "{accepted}");
        id
    }

    /// The next frame that satisfies `wanted`, from the inbox first and then
    /// the socket, within `patience`.
    pub async fn next_where(
        &mut self,
        patience: Duration,
        wanted: impl Fn(&Value) -> bool,
    ) -> Value {
        if let Some(at) = self.inbox.iter().position(&wanted) {
            return self.inbox.remove(at).unwrap();
        }
        let deadline = tokio::time::Instant::now() + patience;
        loop {
            let Ok(frame) = tokio::time::timeout_at(deadline, self.read()).await else {
                panic!(
                    "nothing wanted arrived within {patience:?}; the inbox held:\n{}",
                    self.latest()
                );
            };
            if wanted(&frame) {
                return frame;
            }
            self.inbox.push_back(frame);
        }
    }

    /// The event on subscription `sub` that `wanted` picks, live or in the
    /// snapshot it was sent, within `ANSWER`. A subscription's snapshot is
    /// taken after its `ok`, so a line a command wrote just after subscribing
    /// may be in the snapshot and never sent as an event. A snapshot found
    /// goes back to the inbox less the event taken, for the next wait to read.
    pub async fn live_or_snapshot(&mut self, sub: i64, wanted: impl Fn(&Value) -> bool) -> Value {
        let mut frame = self
            .next_where(ANSWER, |frame| {
                frame["sub"] == sub
                    && (wanted(&frame["event"])
                        || frame["snapshot"]
                            .as_array()
                            .is_some_and(|events| events.iter().any(&wanted)))
            })
            .await;
        let Some(events) = frame["snapshot"].as_array_mut() else {
            return frame["event"].clone();
        };
        let at = events.iter().position(&wanted).unwrap();
        let event = events.remove(at);
        self.inbox.push_front(frame);
        event
    }

    /// The last dozen frames that arrived unwanted, newest last and each cut
    /// short, so a timeout on a slow runner says which step stalled.
    fn latest(&self) -> String {
        let skipped = self.inbox.len().saturating_sub(12);
        let shown: Vec<String> = self
            .inbox
            .iter()
            .skip(skipped)
            .map(|frame| frame.to_string().chars().take(240).collect())
            .collect();
        shown.join("\n")
    }

    /// Whether a reply to `id` has arrived, without waiting for one.
    pub async fn replied(&mut self, id: i64) -> bool {
        while let Ok(frame) = tokio::time::timeout(Duration::from_millis(50), self.read()).await {
            self.inbox.push_back(frame);
        }
        self.inbox
            .iter()
            .any(|frame| frame.get("id").and_then(Value::as_i64) == Some(id))
    }
}
