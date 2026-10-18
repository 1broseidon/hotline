//! An ACP agent whose command never answers `initialize`, as Pork Chop's did
//! when a `mise` shim re-executed itself forever: a shell script that starts a
//! grandchild, writes both pids down, says a word on stderr and then waits.
//!
//! It reaches the core the way a catalogue agent shipped as an archive does.
//! The catalogue is written before the core starts, with the command already
//! where the archive would have unpacked it, so nothing is downloaded.
#![allow(
    dead_code,
    reason = "each stalled-agent test uses a different part of it"
)]

use serde_json::json;
use sha2::Digest;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// Waits for one process to be gone. A killed process whose parent died with
/// it lingers as a zombie until pid 1 reaps it, and a zombie still answers
/// signal 0, so this asks again until it does not.
pub async fn process_gone(pid: i32) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        // Safety: signal 0 only asks whether the process exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
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
