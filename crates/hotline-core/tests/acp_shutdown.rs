//! Quitting takes every ACP agent's process group with it (BRO-267).
//!
//! The app's exit does not drop the desk, so no driver's `Drop` runs; a
//! launcher that ignores its closed stdin was left running under pid 1 after
//! Hotline quit. The exit path calls `end_every_agent`, which is what this
//! drives, while the agent is still starting, as Pork Chop's was. Its own test
//! binary, because ending every agent would end other tests' too.
#![cfg(unix)]
mod common;

use common::stalled::{self, Client};
use hotline_core::wire::Door;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const TOKEN: &str = "disposable-acp-shutdown-test";

#[tokio::test(flavor = "multi_thread")]
async fn ending_every_agent_kills_a_starting_agents_process_group() {
    let root = std::env::temp_dir().join(format!("hotline-acp-shutdown-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    let pids = stalled::install(&root);
    let desk = common::open_desk(&root).unwrap();
    let door = Door::bind(desk.log.clone(), TOKEN.into(), Arc::new(desk)).unwrap();
    let port = door.port();
    let door_task = tokio::spawn(door.run());
    let mut client = Client::connect(port, TOKEN).await;
    let cwd = root.join("workspace").to_string_lossy().into_owned();
    let made = client
        .call(
            "persona.create",
            json!({"draft": {"name":"Pork Chop", "goal":"Code", "cwd":cwd, "backendId":stalled::BACKEND}}),
            Duration::from_secs(5),
        )
        .await;
    let pork_chop = made["result"]["id"].as_str().unwrap().to_owned();

    // The default bound is minutes; the start is still waiting when the app quits.
    client
        .send("session.start", json!({"personaId":pork_chop}))
        .await;
    let (group, grandchild) = stalled::spawned(&pids).await;
    assert!(stalled::group_alive(group));

    hotline_core::driver::acp::end_every_agent();

    assert!(stalled::group_gone(group).await, "the launcher was killed");
    assert!(
        stalled::process_gone(grandchild).await,
        "and what it started"
    );

    drop(client);
    door_task.abort();
    let _ = std::fs::remove_dir_all(root);
}
