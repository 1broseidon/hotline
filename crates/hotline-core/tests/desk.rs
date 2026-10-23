//! The Phase 0 proof: a teammate is created, a session is started and a turn
//! runs, and all of it is watched through the wire the window uses.
//!
//! The first test needs nothing but this machine. The second needs a real
//! provider key in `HOTLINE_HARNESS_ANTHROPIC_KEY` and is ignored unless run
//! with `--ignored`, because a turn that reaches a model is the only honest
//! proof that the driver, the tools and the tape agree. The third is the same
//! proof for the other kind of agent: set `HOTLINE_HARNESS_ACP` to a backend id
//! this machine can run (`cursor`, say), pass `--ignored`, and it drives a real
//! harness as a child.

use crate::common;

use common::Client;
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const TOKEN: &str = "a-token-only-this-harness-knows";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hotline-harness-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A desk on a scratch data directory, its door running.
async fn open(name: &str) -> (PathBuf, u16) {
    let root = scratch(name);
    let port = open_at(&root);
    (root, port)
}

/// The same, on a data directory somebody has already put something in.
fn open_at(root: &Path) -> u16 {
    let desk = common::open_desk(root).unwrap();
    let parts = desk.parts();
    let door = Door::bind(desk.log.clone(), TOKEN.to_string(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    port
}

fn is_sub(frame: &Value, sub: i64, key: &str) -> bool {
    frame.get("sub").and_then(Value::as_i64) == Some(sub) && frame.get(key).is_some()
}

/// Everything the agent said in the turn now running, joined, once the turn
/// has ended. A turn with tools in it may say a line before the work and the
/// answer after, so the answer is looked for in the whole of it.
async fn said_this_turn(client: &mut Client, tape: i64, patience: Duration) -> String {
    let mut said = Vec::new();
    loop {
        let frame = client
            .next_where(patience, |frame| {
                is_sub(frame, tape, "event")
                    && matches!(frame["event"]["kind"].as_str(), Some("agent" | "turn"))
            })
            .await;
        if frame["event"]["kind"] == "turn" {
            return said.join("\n");
        }
        said.push(
            frame["event"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        );
    }
}

const FAKE_OLLAMA_MODEL: &str = "custom/coder:latest";

/// A test that grows the model list mid-test keeps its own handle on `installed`.
async fn fake_ollama_listing(
    name: &str,
    installed: Arc<std::sync::Mutex<Vec<String>>>,
) -> (String, mpsc::Receiver<Value>, JoinHandle<()>) {
    use axum::{
        Router,
        body::Bytes,
        http::HeaderMap,
        routing::{get, post},
    };
    let (requests_tx, requests_rx) = mpsc::channel(4);
    let answer = format!("Hello from {name}.");
    let app = Router::new()
        .route("/api/tags", get(move |headers: HeaderMap| {
            assert!(!headers.contains_key("authorization"));
            let installed = installed.clone();
            async move {
                json!({"models": installed.lock().unwrap().iter().map(|id| json!({"name": id, "model": id})).collect::<Vec<_>>()}).to_string()
            }
        }))
        .route("/api/chat", post(move |headers: HeaderMap, bytes: Bytes| {
            assert!(!headers.contains_key("authorization"));
            let tx = requests_tx.clone();
            let answer = answer.clone();
            async move {
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                tx.send(request).await.unwrap();
                let chunk = json!({"model":FAKE_OLLAMA_MODEL, "created_at":"2026-09-09T00:00:00Z",
                    "message":{"role":"assistant", "content":answer}, "done":false});
                let done = json!({"model":FAKE_OLLAMA_MODEL, "created_at":"2026-09-09T00:00:00Z",
                    "message":{"role":"assistant", "content":""}, "done":true, "done_reason":"stop",
                    "prompt_eval_count":10, "eval_count":4});
                ([("Content-Type", "application/x-ndjson")], format!("{chunk}\n{done}\n"))
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, requests_rx, server)
}

async fn fake_ollama(name: &str) -> (String, mpsc::Receiver<Value>, JoinHandle<()>) {
    let installed = Arc::new(std::sync::Mutex::new(vec![FAKE_OLLAMA_MODEL.to_string()]));
    fake_ollama_listing(name, installed).await
}

// The tests, one file per topic in `tests/desk/`. The inline module is what
// puts them there: a crate root's own `mod` would look beside it in `tests/`,
// where Cargo would take each file for a binary of its own.
mod desk {
    use super::*;

    mod accounts;
    mod connections;
    mod logins;
    mod real_agents;
    mod skills;
    mod teammates;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    mod workspace;
}
