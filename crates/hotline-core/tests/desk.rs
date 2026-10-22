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

// The tests, one file per topic in `tests/desk/`. The inline module is what
// puts them there: a crate root's own `mod` would look beside it in `tests/`,
// where Cargo would take each file for a binary of its own.
mod desk {
    use super::*;

    mod capture;
    mod connections;
    mod logins;
    mod real_agents;
    mod skills;
    mod teammates;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    mod workspace;
}
