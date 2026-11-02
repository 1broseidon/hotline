//! The one lock order is `start_gate -> policy_update_lock`: a message
//! closing a quiet chapter holds its teammate's start gate and then takes the
//! policy lock. A policy change over the wire takes the policy lock for its
//! invalidate and durable append, and lets it go before the reattach that
//! waits on each start gate, so the two never wait on each other.

use super::commands;
use super::*;
use crate::session::tests::{Fake, Scripted, enrol, persona};

/// A real room with Ada running, behind the wire's handle.
async fn running_ada() -> (tempfile::TempDir, Log, Arc<Room>, Arc<dyn RoomHandle>) {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    enrol(&log, &persona("ada"));
    let room = Room::with_agents(
        log.clone(),
        Arc::new(crate::session::tests::DeskKeys),
        Fake::new(Scripted::turns(vec![Vec::new()])),
    );
    room.start("ada").await.unwrap();
    let handle: Arc<dyn RoomHandle> = Arc::new(CoreHandle { room: room.clone() });
    (root, log, room, handle)
}

/// Runs `command` while this test holds Ada's start gate, as a message does,
/// and takes the policy lock once Ada's session is revoked, as that message's
/// chapter close would. The command has let go of the lock by then, and goes
/// on to its reattach once the gate is free.
async fn runs_beside_a_held_start_gate(command: Value) {
    let (_root, log, room, handle) = running_ada().await;
    let gate = room.start_gate("ada").lock_owned().await;
    let command: Command = serde_json::from_value(command).unwrap();
    let running = tokio::spawn({
        let (log, handle) = (log.clone(), handle.clone());
        async move { commands::run(command, &log, &handle, &Parts::default()).await }
    });
    crate::test_support::until("ada's session is revoked", || {
        room.info("ada").state == SessionState::Stopped
    })
    .await;
    let policy = tokio::time::timeout(
        Duration::from_secs(5),
        room.policy_update_lock().lock_owned(),
    )
    .await
    .expect("the command lets go of the policy lock before it waits on a start gate");
    drop(policy);
    drop(gate);
    tokio::time::timeout(Duration::from_secs(10), running)
        .await
        .expect("the command finishes once the gate is free")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn a_persona_update_lets_go_of_the_policy_lock_before_its_reattach() {
    runs_beside_a_held_start_gate(json!({
        "cmd": "persona.update",
        "params": {"id": "ada", "patch": {"reach": "workspace"}},
    }))
    .await;
}

#[tokio::test]
async fn a_settings_update_lets_go_of_the_policy_lock_before_its_reattach() {
    runs_beside_a_held_start_gate(json!({
        "cmd": "settings.update",
        "params": {"patch": {"mcpServers": []}},
    }))
    .await;
}

#[tokio::test]
async fn an_mcp_sign_out_lets_go_of_the_policy_lock_before_its_reattach() {
    runs_beside_a_held_start_gate(json!({
        "cmd": "mcp.auth_sign_out",
        "params": {"serverId": "docs"},
    }))
    .await;
}
