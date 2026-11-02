//! Inbound sources over the real seat handler: the owner adds, changes and
//! removes them, a phone reads them and mutes or unmutes one, and nothing
//! else does.

use super::*;
use crate::credentials::MemoryStore;

struct Fixture {
    _root: tempfile::TempDir,
    desk: Arc<crate::desk::Desk>,
    handle: Arc<dyn RoomHandle>,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    for (id, name) in [("ada", "Ada"), ("bob", "Bob")] {
        let mut persona = crate::session::tests::persona(id);
        persona.name = name.into();
        crate::room::append_persona(&desk.log, &persona).unwrap();
    }
    let handle: Arc<dyn RoomHandle> = desk.clone();
    Fixture {
        _root: root,
        desk,
        handle,
    }
}

async fn run(fixture: &Fixture, seat: Seat, cmd: &str, params: Value) -> Value {
    remote_control_answer(
        seat,
        &fixture.handle,
        &fixture.desk.log,
        &fixture.desk.parts(),
        json!({ "id": 1, "cmd": cmd, "params": params }),
    )
    .await
}

fn github(teammates: &[&str]) -> Value {
    json!({ "draft": { "type": "github", "name": "hotline repo", "teammates": teammates } })
}

#[tokio::test]
async fn only_the_owner_adds_widens_and_removes_a_source_and_a_phone_only_mutes_it() {
    let fixture = fixture();
    let refused = run(&fixture, Seat::Phone, "inbound.add", github(&["bob"])).await;
    assert_eq!(refused["code"], FORBIDDEN, "{refused}");
    assert_eq!(hotline_room::inbound::sources(&fixture.desk.log), []);

    let added = run(&fixture, Seat::Owner, "inbound.add", github(&["bob"])).await;
    let source = &added["result"];
    assert_eq!(source["type"], "github", "{added}");
    assert_eq!(source["teammates"], json!(["bob"]));
    assert_eq!(source["maxLevel"], "note", "notes by default");
    assert_eq!(source["rate"], hotline_room::inbound::DEFAULT_RATE);
    assert!(source.get("filesAllowed").is_none() && source.get("muted").is_none());
    let id = source["id"].as_str().unwrap().to_string();

    let listed = run(&fixture, Seat::Phone, "inbound.list", json!({})).await;
    assert_eq!(listed["result"][0]["id"], id.as_str(), "{listed}");

    let muted = run(
        &fixture,
        Seat::Phone,
        "inbound.update",
        json!({ "id": id, "muted": true }),
    )
    .await;
    assert_eq!(muted["result"]["muted"], true, "{muted}");
    for (cmd, params) in [
        (
            "inbound.update",
            json!({ "id": id, "muted": false, "maxLevel": "person" }),
        ),
        (
            "inbound.update",
            json!({ "id": id, "teammates": ["ada", "bob"] }),
        ),
        ("inbound.update", json!({ "id": id, "filesAllowed": true })),
        ("inbound.update", json!({ "id": id, "rate": 100 })),
        ("inbound.remove", json!({ "id": id })),
    ] {
        let refused = run(&fixture, Seat::Phone, cmd, params.clone()).await;
        assert_eq!(refused["code"], FORBIDDEN, "{cmd} {params}: {refused}");
    }
    assert!(
        hotline_room::inbound::source(&fixture.desk.log, &id)
            .unwrap()
            .muted
    );

    // The owner widens it, and unmuting releases the brake.
    let mut braked = hotline_room::inbound::source(&fixture.desk.log, &id).unwrap();
    braked.wakes = 12;
    hotline_room::inbound::write(&fixture.desk.log, &braked).unwrap();
    let widened = run(
        &fixture,
        Seat::Owner,
        "inbound.update",
        json!({ "id": id, "muted": false, "teammates": ["ada", "bob", "ada"], "maxLevel": "nudge", "rate": 30 }),
    )
    .await;
    let source = &widened["result"];
    assert_eq!(source["teammates"], json!(["ada", "bob"]), "{widened}");
    assert_eq!(source["maxLevel"], "nudge");
    assert_eq!(source["rate"], 30);
    assert!(source.get("muted").is_none() && source.get("wakes").is_none());
    for params in [
        json!({ "id": id, "teammates": ["zed"] }),
        json!({ "id": id, "teammates": [] }),
        json!({ "id": id, "rate": 0 }),
    ] {
        let refused = run(&fixture, Seat::Owner, "inbound.update", params.clone()).await;
        assert_eq!(refused["ok"], false, "{params}: {refused}");
    }

    let removed = run(&fixture, Seat::Owner, "inbound.remove", json!({ "id": id })).await;
    assert_eq!(removed["ok"], true, "{removed}");
    let listed = run(&fixture, Seat::Desk, "inbound.list", json!({})).await;
    assert_eq!(listed["result"], json!([]), "{listed}");
}

#[tokio::test]
async fn a_source_names_a_stored_secret_and_never_holds_a_value() {
    let fixture = fixture();
    let value = "whsec_0123456789abcdef";
    let named = |secret: &str| json!({ "draft": { "type": "webhook", "name": "deploys", "teammates": ["ada"], "secret": secret } });
    let refused = run(&fixture, Seat::Owner, "inbound.add", named(value)).await;
    assert_eq!(
        refused["ok"], false,
        "a value is no stored secret's name: {refused}"
    );
    let refused = run(&fixture, Seat::Owner, "inbound.add", named("HOOK_KEY")).await;
    assert!(
        refused["error"]
            .as_str()
            .unwrap_or_else(|| panic!("{refused}"))
            .contains("No secret called HOOK_KEY"),
        "{refused}"
    );

    fixture
        .desk
        .test_vault()
        .set_shared_secret("HOOK_KEY", value)
        .unwrap();
    let added = run(&fixture, Seat::Owner, "inbound.add", named("HOOK_KEY")).await;
    assert_eq!(added["result"]["secret"], "HOOK_KEY", "{added}");
    let room = std::fs::read_to_string(fixture.desk.log.root().join("room.jsonl")).unwrap();
    assert!(room.contains("HOOK_KEY") && !room.contains(value), "{room}");
}

#[test]
fn the_phone_seat_reads_and_mutes_sources_and_nothing_more() {
    let update = |muted: Option<bool>, rate: Option<u32>| Command::InboundUpdate {
        id: "s".into(),
        muted,
        teammates: None,
        max_level: None,
        files_allowed: None,
        rate,
    };
    assert!(Seat::Phone.permits(&Command::InboundList {}));
    assert!(Seat::Phone.permits(&update(Some(true), None)));
    assert!(Seat::Phone.permits(&update(Some(false), None)));
    assert!(!Seat::Phone.permits(&update(None, None)));
    assert!(!Seat::Phone.permits(&update(Some(false), Some(5))));
    assert!(!Seat::Phone.permits(&Command::InboundRemove { id: "s".into() }));
    for seat in [Seat::Desk, Seat::Owner] {
        assert!(seat.permits(&Command::InboundRemove { id: "s".into() }));
        assert!(seat.permits(&update(None, Some(5))));
    }
    assert!(!Seat::Link.permits(&Command::InboundList {}));
}
