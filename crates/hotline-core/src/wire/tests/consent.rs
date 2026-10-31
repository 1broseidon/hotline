//! "Always allow" through the wire: `persona.update` and `settings.update`
//! clear the person's standing consent to hand a teammate work only when its
//! access grows, and clear `allowedSenders` with it, so the settings never
//! say Always when a handoff would ask.

use super::commands;
use super::*;
use crate::session::consent;
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona};

/// Ada always allowed to hand Bob work, on a desk with one MCP server, behind
/// the wire's handle.
fn granted() -> (tempfile::TempDir, Log, Arc<Room>, Arc<dyn RoomHandle>) {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    log.append(
        &StreamId::Room,
        &json!({"kind": "setting", "id": "mcpServers", "value": [
            {"id": "docs", "name": "Docs", "type": "http", "url": "https://docs.test/mcp"}
        ]}),
    )
    .unwrap();
    enrol(&log, &persona("ada"));
    let mut bob = persona("bob");
    bob.reach = Some(Reach::Workspace);
    bob.mcp_policy.mode = crate::contract::PolicyMode::None;
    bob.allowed_senders = vec!["ada".into()];
    enrol(&log, &bob);
    consent::record_informed(&log, "ada", "bob").unwrap();
    let room = Room::with_agents(
        log.clone(),
        Arc::new(DeskKeys),
        Fake::new(Scripted::turns(vec![Vec::new()])),
    );
    let handle: Arc<dyn RoomHandle> = Arc::new(CoreHandle { room: room.clone() });
    (root, log, room, handle)
}

async fn run(log: &Log, handle: &Arc<dyn RoomHandle>, command: Value) {
    let command: Command = serde_json::from_value(command).unwrap();
    commands::run(command, log, handle, &Parts::default())
        .await
        .unwrap();
}

/// Whether the settings still say Bob always accepts Ada, and whether a
/// handoff would pass on that without a card.
fn always(log: &Log) -> (bool, bool) {
    let bob = room::roster(log)
        .into_iter()
        .find(|persona| persona.id == "bob")
        .unwrap();
    (
        bob.allowed_senders.iter().any(|id| id == "ada"),
        consent::informed(&log.load(&StreamId::Room), "ada", "bob"),
    )
}

#[tokio::test]
async fn a_persona_update_clears_always_only_when_the_recipients_access_grows() {
    for (patch, grows) in [
        (json!({"goal": "Ship the release."}), false),
        (json!({"backendId": "claude-code"}), false),
        (json!({"modelId": "anthropic/claude"}), false),
        (json!({"reach": "machine"}), true),
        (json!({"mcpPolicy": {"mode": "all", "serverIds": []}}), true),
        (json!({"computer": {"enabled": true}}), true),
    ] {
        let (_root, log, _room, handle) = granted();
        run(
            &log,
            &handle,
            json!({"cmd": "persona.update", "params": {"id": "bob", "patch": patch}}),
        )
        .await;
        let expected = if grows { (false, false) } else { (true, true) };
        assert_eq!(always(&log), expected, "{patch}");
    }
}

#[tokio::test]
async fn a_server_added_to_the_desk_clears_always_for_a_teammate_granted_all() {
    for (mode, grows) in [("all", true), ("none", false)] {
        let (_root, log, _room, handle) = granted();
        run(
            &log,
            &handle,
            json!({"cmd": "persona.update", "params": {"id": "bob",
                "patch": {"mcpPolicy": {"mode": mode, "serverIds": []}}}}),
        )
        .await;
        grants_again(&log);
        run(
            &log,
            &handle,
            json!({"cmd": "settings.update", "params": {"patch": {"mcpServers": [
                {"id": "docs", "name": "Docs", "type": "http", "url": "https://docs.test/mcp"},
                {"id": "mail", "name": "Mail", "type": "http", "url": "https://mail.test/mcp"}
            ]}}}),
        )
        .await;
        let expected = if grows { (false, false) } else { (true, true) };
        assert_eq!(always(&log), expected, "{mode}");
    }
}

/// The person chooses Always again after the policy change above, so what
/// the server's arrival does is all that is measured.
fn grants_again(log: &Log) {
    let mut bob = room::roster(log)
        .into_iter()
        .find(|persona| persona.id == "bob")
        .unwrap();
    if !bob.allowed_senders.iter().any(|id| id == "ada") {
        bob.allowed_senders.push("ada".into());
        room::append_persona(log, &bob).unwrap();
    }
    consent::record_informed(log, "ada", "bob").unwrap();
}
