use super::*;

/// A linked desk's channel is answered under `Seat::Link` (ADR 0003): its
/// roster, its messages and a ping, and nothing else. Every other command
/// the contract has is refused before it runs, and so is every subscription;
/// the desk and owner seats, for their part, can never speak for a linked
/// desk.
#[tokio::test]
async fn a_linked_desk_may_only_publish_its_teammates_and_deliver_messages_through_the_real_handler()
 {
    use crate::credentials::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    crate::linked::save(
        &desk.log,
        &crate::linked::Record {
            desk_id: "laptop".into(),
            name: "laptop".into(),
            dials: false,
            key: Some("a-key".into()),
            teammates: Vec::new(),
        },
    )
    .unwrap();

    // Every command the contract names, whatever it is given, is refused.
    let declared = <Command as ts_rs::TS>::decl(&ts_rs::Config::default());
    let names: Vec<&str> = declared
        .split("\"cmd\": \"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    assert!(names.len() > 100, "{}", names.len());
    for name in names {
        if name == "ping" || name.starts_with("federation.") {
            continue;
        }
        let answer = federation(
            &json!({"id": 1, "cmd": name, "params": {}}),
            "laptop",
            &room,
        )
        .await
        .unwrap();
        assert_eq!(answer["ok"], false, "{name}: {answer}");
    }
    // A command it can read is refused as forbidden, not merely unreadable.
    for (cmd, params) in [
        ("remote.status", json!({})),
        ("remote.pairing", json!({})),
        ("desks.links", json!({})),
        ("desks.link", json!({"deskId": "grizzly"})),
        ("desks.unlink", json!({"deskId": "laptop"})),
        (
            "desks.link_accept",
            json!({"deskId": "mallory", "name": "Mallory", "key": "a-key"}),
        ),
        ("settings.update", json!({"patch": {}})),
        ("persona.delete", json!({"id": "lou"})),
        ("session.start", json!({"personaId": "lou"})),
        ("session.prompt", json!({"personaId": "lou", "text": "hi"})),
        (
            "session.answer_permission",
            json!({"personaId": "lou", "requestId": "collab:x", "optionId": "allow_always"}),
        ),
        (
            "teammates.exchange_stop",
            json!({"a": "laptop/clem", "b": "lou"}),
        ),
        ("thread.list", json!({})),
        ("models.list", json!({})),
        ("backends.list", json!({})),
        (
            "credential.create",
            json!({"providerId": "anthropic", "label": "x", "secret": "y"}),
        ),
    ] {
        let answer = federation(
            &json!({"id": 2, "cmd": cmd, "params": params}),
            "laptop",
            &room,
        )
        .await
        .unwrap();
        assert_eq!(answer["code"], FORBIDDEN, "{cmd}: {answer}");
    }
    for target in [
        json!("room"),
        json!({"view": "roster"}),
        json!({"tape": "lou"}),
    ] {
        let answer = federation(&json!({"id": 3, "sub": target}), "laptop", &room)
            .await
            .unwrap();
        assert_eq!(answer["code"], FORBIDDEN, "{answer}");
    }
    // What it may do, it may do as the desk the channel proved, and only
    // while that desk is linked.
    let ping = federation(&json!({"id": 4, "cmd": "ping"}), "laptop", &room)
        .await
        .unwrap();
    assert_eq!(ping["ok"], true);
    let roster = json!({"id": 5, "cmd": "federation.roster", "params": {"teammates": []}});
    assert_eq!(
        federation(&roster, "laptop", &room).await.unwrap()["ok"],
        true
    );
    let stranger = federation(&roster, "stranger", &room).await.unwrap();
    assert_eq!(stranger["ok"], false);
    assert_eq!(stranger["error"], "This desk is not linked with that one.");
    // The desk and its owners never speak for a linked desk.
    for seat in [Seat::Desk, Seat::Owner] {
        for (cmd, params) in [
            ("federation.roster", json!({"teammates": []})),
            (
                "federation.deliver",
                json!({"delivery": {"kind": "result", "requestId": "r", "reply": "", "failed": false}}),
            ),
            ("federation.unlink", json!({})),
        ] {
            let answer = remote_control_answer(
                seat,
                &room,
                &desk.log,
                json!({"id": 6, "cmd": cmd, "params": params}),
            )
            .await;
            assert_eq!(answer["code"], FORBIDDEN, "{seat:?} {cmd}: {answer}");
        }
    }
    assert!(crate::linked::record(&desk.log, "laptop").is_some());
}
