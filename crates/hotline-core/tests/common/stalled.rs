//! An ACP agent whose command never answers `initialize`, as Pork Chop's did
//! when a `mise` shim re-executed itself forever: a shell script that starts a
//! grandchild, writes both pids down, says a word on stderr and then waits.
//!
//! It reaches the core the way a catalogue agent shipped as an archive does.
//! The catalogue is written before the core starts, with the command already
//! where the archive would have unpacked it, so nothing is downloaded.
#![allow(dead_code)]

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::Digest;
use std::collections::VecDeque;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

pub const BACKEND: &str = "stalled-agent";
pub const NAME: &str = "Stalled Agent";
const ARCHIVE: &str = "https://example.invalid/stalled-agent.tar.gz";

/// Puts the agent in `root`'s catalogue and its command on disk. Answers the
/// file its pids are written to.
pub fn install(root: &Path) -> PathBuf {
    let pids = root.join("stalled.pids");
    let platform = format!(
        "{}-{}",
        if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        },
        std::env::consts::ARCH
    );
    let catalogue = json!({
        "format": 2,
        "fetchedAt": chrono::Utc::now().timestamp_millis(),
        "agents": [{
            "id": BACKEND,
            "name": NAME,
            "distribution": {"binary": {platform: {
                "archive": ARCHIVE,
                "cmd": "./stall",
                "env": {"STALL_PIDS": pids.to_string_lossy()},
            }}},
        }],
    });
    let cache = root.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("acp-registry.json"), catalogue.to_string()).unwrap();

    let folder = hex::encode(&sha2::Sha256::digest(ARCHIVE.as_bytes())[..8]);
    let dir = root.join("acp-agents").join(BACKEND).join(folder);
    std::fs::create_dir_all(&dir).unwrap();
    let command = dir.join("stall");
    std::fs::write(
        &command,
        "#!/bin/sh\n\
         sleep 600 &\n\
         echo $$ >> \"$STALL_PIDS\"\n\
         echo $! >> \"$STALL_PIDS\"\n\
         echo 'resolving the toolchain' >&2\n\
         while :; do sleep 1; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755)).unwrap();
    pids
}

/// The launcher's pid, which is its process group, and its grandchild's, once
/// the script has written them.
pub async fn spawned(pids: &Path) -> (i32, i32) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let written = std::fs::read_to_string(pids).unwrap_or_default();
        let ids: Vec<i32> = written
            .lines()
            .filter_map(|line| line.parse().ok())
            .collect();
        if let [group, grandchild] = ids[..] {
            return (group, grandchild);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the agent's command never ran"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Whether anything in this process group is still running.
pub fn group_alive(group: i32) -> bool {
    // Safety: signal 0 only asks whether the group exists.
    unsafe { libc::killpg(group, 0) == 0 }
}

/// Waits for the group to be gone, reaped included.
pub async fn group_gone(group: i32) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        if !group_alive(group) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// A model endpoint that answers every streamed request with one line.
pub async fn model_endpoint(reply: &'static str) -> (String, tokio::task::JoinHandle<()>) {
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move || async move {
            let chunks = [
                json!({"id":"chat","object":"chat.completion.chunk","created":1,"model":"vendor/butler","choices":[{"index":0,"delta":{"role":"assistant","content":reply},"finish_reason":null}]}),
                json!({"id":"chat","object":"chat.completion.chunk","created":1,"model":"vendor/butler","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            ];
            let body: String = chunks.iter().map(|chunk| format!("data: {chunk}\n\n")).collect();
            ([("Content-Type", "text/event-stream")], format!("{body}data: [DONE]\n\n"))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, server)
}

pub struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: i64,
    /// Frames that arrived while waiting for something else.
    pub inbox: VecDeque<Value>,
}

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

    /// Sends a command and answers its id without waiting for the reply.
    pub async fn send(&mut self, cmd: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        let frame = json!({ "id": id, "cmd": cmd, "params": params });
        self.socket
            .send(Message::text(frame.to_string()))
            .await
            .unwrap();
        id
    }

    /// A command, answered within `patience`.
    pub async fn call(&mut self, cmd: &str, params: Value, patience: Duration) -> Value {
        let id = self.send(cmd, params).await;
        self.reply(id, patience).await
    }

    pub async fn reply(&mut self, id: i64, patience: Duration) -> Value {
        self.next_where(patience, |frame| {
            frame.get("id").and_then(Value::as_i64) == Some(id) && frame.get("sub").is_none()
        })
        .await
    }

    pub async fn subscribe(&mut self, target: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.socket
            .send(Message::text(
                json!({ "id": id, "sub": target }).to_string(),
            ))
            .await
            .unwrap();
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
            let frame = tokio::time::timeout_at(deadline, self.read())
                .await
                .unwrap_or_else(|_| panic!("nothing wanted arrived within {patience:?}"));
            if wanted(&frame) {
                return frame;
            }
            self.inbox.push_back(frame);
        }
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
