use super::*;
use crate::contract::{ComputerMount, FolderGrant, McpPolicy, PersonaComputer};
use crate::driver::{MessageKind, Update};
use crate::mcp::server::TeammateTools;
use crate::session::Room;
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
use crate::test_support::until;
use std::sync::Arc;

/// Bob as the person left him: workspace reach, one read-only folder, one
/// server granted, his computer on with one mount and one secret.
fn bob() -> Persona {
    let mut bob = persona("bob");
    bob.name = "Bob".into();
    bob.reach = Some(Reach::Workspace);
    bob.folders = Some(vec![FolderGrant {
        path: "/srv/notes".into(),
        writable: false,
    }]);
    bob.mcp_policy = McpPolicy {
        mode: PolicyMode::Some,
        server_ids: vec!["docs".into()],
    };
    bob.computer = Some(PersonaComputer {
        enabled: true,
        image: None,
        memory: None,
        cpus: None,
        pids: None,
        mounts: Some(vec![ComputerMount {
            host: "/srv/repo".into(),
            path: "/home/agent/repo".into(),
            readonly: true,
        }]),
    });
    bob.secrets = Some(vec!["NPM_TOKEN".into()]);
    bob
}

fn servers() -> Vec<String> {
    vec!["docs".into(), "mail".into()]
}

/// What Bob's record is after `change`, and what that did to the person's
/// consent.
fn after(change: impl FnOnce(&mut Persona)) -> Option<Widening> {
    let previous = bob();
    let mut updated = bob();
    change(&mut updated);
    widened(&previous, &updated, &servers(), &servers())
}

#[test]
fn each_widening_of_the_recipients_access_is_named() {
    assert_eq!(
        after(|bob| bob.reach = Some(Reach::Machine)),
        Some(Widening::Machine)
    );
    assert_eq!(
        after(|bob| bob.cwd = "/srv/elsewhere".into()),
        Some(Widening::Workspace)
    );
    assert_eq!(
        after(|bob| bob.folders.as_mut().unwrap().push(FolderGrant {
            path: "/srv/books".into(),
            writable: false,
        })),
        Some(Widening::Folder),
        "a folder added"
    );
    assert_eq!(
        after(|bob| bob.folders.as_mut().unwrap()[0].writable = true),
        Some(Widening::Folder),
        "a folder made writable"
    );
    assert_eq!(
        after(
            |bob| bob.computer.as_mut().unwrap().mounts = Some(vec![ComputerMount {
                host: "/srv/repo".into(),
                path: "/home/agent/repo".into(),
                readonly: false,
            }])
        ),
        Some(Widening::Folder),
        "a mount made writable"
    );
    assert_eq!(
        after(|bob| bob.secrets.as_mut().unwrap().push("AWS_KEY".into())),
        Some(Widening::Secret)
    );
    assert_eq!(
        after(|bob| bob.mcp_policy.server_ids.push("mail".into())),
        Some(Widening::McpServer),
        "a server granted"
    );
    assert_eq!(
        after(|bob| bob.mcp_policy.mode = PolicyMode::All),
        Some(Widening::McpServer),
        "every server granted"
    );
    let previous = {
        let mut off = bob();
        off.computer.as_mut().unwrap().enabled = false;
        off
    };
    assert_eq!(
        widened(&previous, &bob(), &servers(), &servers()),
        Some(Widening::Computer),
        "the computer turned on"
    );
    let mut all = bob();
    all.mcp_policy.mode = PolicyMode::All;
    assert_eq!(
        widened(
            &all,
            &all,
            &servers(),
            &[servers(), vec!["drive".into()]].concat()
        ),
        Some(Widening::McpServer),
        "a server added to the desk reaches a teammate granted all"
    );
}

#[test]
fn nothing_that_leaves_the_recipient_reaching_as_far_clears_consent() {
    assert_eq!(after(|bob| bob.goal = "Something else.".into()), None);
    assert_eq!(after(|bob| bob.model_id = Some("gpt".into())), None);
    assert_eq!(after(|bob| bob.backend_id = "claude-code".into()), None);
    assert_eq!(after(|bob| bob.name = "Robert".into()), None);
    assert_eq!(after(|bob| bob.folders = None), None, "a folder removed");
    assert_eq!(
        after(|bob| bob.computer.as_mut().unwrap().enabled = false),
        None
    );
    assert_eq!(
        after(|bob| bob.mcp_policy.mode = PolicyMode::None),
        None,
        "servers taken away"
    );
    assert_eq!(
        after(|bob| bob.computer.as_mut().unwrap().memory = Some("8g".into())),
        None
    );
    let mut all = bob();
    all.mcp_policy.mode = PolicyMode::All;
    assert_eq!(
        widened(&all, &all, &servers(), &["docs".to_string()]),
        None,
        "a server removed from the desk"
    );
    let mut some = bob();
    some.reach = Some(Reach::Machine);
    let mut narrowed = some.clone();
    narrowed.reach = Some(Reach::Workspace);
    assert_eq!(widened(&some, &narrowed, &servers(), &servers()), None);
}

/// The informed check reads the room stream's fold by id, latest line
/// first, so clearing the consent counts the moment it is written, and the
/// same after the stream is compacted; a fresh Always counts again.
#[test]
fn a_cleared_consent_counts_at_once_and_after_compaction() {
    let log = scratch("consent-fold");
    let mut target = bob();
    target.allowed_senders = vec!["ada".into()];
    enrol(&log, &target);
    record_informed(&log, "ada", "bob").unwrap();
    let informed_now = || informed(&log.load(&StreamId::Room), "ada", "bob");
    assert!(informed_now());

    clear(&log, &mut target, Widening::Folder).unwrap();
    assert_eq!(
        target.allowed_senders,
        [] as [String; 0],
        "the sender goes too"
    );
    assert!(!informed_now(), "before compaction");
    log.compact(&StreamId::Room).unwrap();
    assert!(!informed_now(), "after compaction");

    record_informed(&log, "ada", "bob").unwrap();
    assert!(informed_now());
    log.compact(&StreamId::Room).unwrap();
    assert!(informed_now());
}

fn finished() -> Vec<Update> {
    vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: "built".into(),
            text: "Built it.".into(),
        },
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    ]
}

/// Ada, with her own folders, and Bob with the whole machine, so Ada asking
/// Bob to work goes up in access and needs the person's word.
fn room_of_two(name: &str) -> Arc<Room> {
    let log = scratch(name);
    let mut ada = persona("ada");
    ada.reach = Some(Reach::Workspace);
    enrol(&log, &ada);
    let mut bob = bob();
    bob.reach = Some(Reach::Machine);
    enrol(&log, &bob);
    room_on(log)
}

/// A room on `log` with no container runtime: Bob's computer fails at once.
/// Where one is installed it would be pulled, and that pull outlives the
/// room it started in, which then takes up the next room's handoffs from
/// the log.
fn room_on(log: crate::log::Log) -> Arc<Room> {
    Room::with_agents_and_computers(
        log,
        Arc::new(DeskKeys),
        Fake::new(Scripted::new(finished())),
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    )
}

/// Ada sends Bob an item that wants work, and Bob takes it into a work
/// thread as soon as it lands.
async fn hand_off(room: &Arc<Room>) {
    let sent = TeammateTools::new(room, "ada")
        .call(
            "send",
            &json!({"to": ["bob"], "title": "Build it", "message": "Build it.", "wants": "work"}),
        )
        .await
        .unwrap();
    let id = serde_json::from_str::<Value>(&sent).unwrap()["sent"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let room = room.clone();
    tokio::spawn(async move {
        let landed = || {
            crate::store::inbox::received(&room.log, "bob")
                .iter()
                .any(|item| item.id == id)
        };
        until("the item lands", landed).await;
        TeammateTools::new(&room, "bob")
            .call(
                "update_inbox",
                &json!({"items": [{"id": id, "state": "taken"}]}),
            )
            .await
            .unwrap();
    });
}

/// The first-contact card waiting on Ada's tape, if one is.
fn open_card(room: &Room) -> Option<Value> {
    room.tape("ada")
        .into_iter()
        .find(|event| event["kind"] == "permission" && event.get("decision").is_none())
}

/// Whether the work's result is in Ada's inbox, which only work that landed
/// without a card has by now.
fn result_heard(room: &Room) -> bool {
    crate::store::inbox::of(&room.log, "ada")
        .iter()
        .any(|reply| reply.reply_to.is_some())
}

/// Always keeps Ada's requests to Bob from asking until Bob's access grows;
/// then the grant is cleared, settings no longer list Ada as allowed, and the
/// next request asks again in the same words.
#[tokio::test]
async fn always_asks_no_more_until_the_receivers_access_grows_and_then_asks_again() {
    let room = room_of_two("consent-asks-again");
    hand_off(&room).await;
    until("the card asks", || open_card(&room).is_some()).await;
    let first = open_card(&room).unwrap();
    room.answer_permission("ada", first["requestId"].as_str().unwrap(), "allow_always")
        .await
        .unwrap();
    until("the handoff ran", || result_heard(&room)).await;
    assert_eq!(room.persona("bob").unwrap().allowed_senders, ["ada"]);
    assert!(informed(&room.log.load(&StreamId::Room), "ada", "bob"));

    // A second request: no card.
    let cards = || {
        room.tape("ada")
            .iter()
            .filter(|event| event["kind"] == "permission")
            .count()
    };
    let sent = TeammateTools::new(&room, "ada")
        .call(
            "send",
            &json!({"to": ["bob"], "title": "And the docs", "wants": "work"}),
        )
        .await
        .unwrap();
    assert!(!sent.contains("has not yet allowed"), "{sent}");
    assert_eq!(cards(), 1);

    // Bob is given a folder: his access grew, so Always is cleared, and the
    // next request asks again with the card's own words.
    let mut target = room.persona("bob").unwrap();
    clear(&room.log, &mut target, Widening::Folder).unwrap();
    crate::room::append_persona(&room.log, &target).unwrap();
    assert_eq!(
        room.persona("bob").unwrap().allowed_senders,
        [] as [String; 0]
    );
    TeammateTools::new(&room, "ada")
        .call(
            "send",
            &json!({"to": ["bob"], "title": "One more", "wants": "work"}),
        )
        .await
        .unwrap();
    until("a card asks the person again", || cards() == 2).await;
    let again = open_card(&room).unwrap();
    assert_eq!(again["title"], first["title"]);
}

/// An Always grant is not authority a reattach, a sign-out or a restart
/// ends: after Bob's agent is switched, Ada's reattached, every session
/// reattached as an MCP sign-out does, and the desk restarted with its room
/// stream compacted, Ada's handoff runs without a card.
#[tokio::test]
async fn an_always_grant_survives_reattaches_a_sign_out_and_a_restart() {
    let room = room_of_two("consent-survives");
    room.allow_sender("bob", "ada").unwrap();
    room.start("ada").await.unwrap();
    room.start("bob").await.unwrap();
    room.reattach("bob").await.unwrap();
    room.reattach("ada").await.unwrap();
    room.invalidate_all().unwrap();
    room.reattach_all().await.unwrap();
    let log = room.log.clone();
    drop(room);
    log.compact(&StreamId::Room).unwrap();

    let room = room_on(log.clone());
    assert_eq!(room.persona("bob").unwrap().allowed_senders, ["ada"]);
    hand_off(&room).await;
    until("the handoff ran", || result_heard(&room)).await;
    assert!(open_card(&room).is_none(), "no card asked again");
}
