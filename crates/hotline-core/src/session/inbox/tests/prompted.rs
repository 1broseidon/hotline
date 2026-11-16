//! What a colleague's message prompted says so: a file sent, a card raised
//! or a job made in a quiet turn that one inbox item woke names that item
//! (`fromItem`), as its post does (`quiet.rs`), so the desk draws "From
//! Goldie's message · title" over it. A turn the person reads, or one that
//! several items woke together, names none.

use super::*;
use crate::contract::{HumanAnswer, PersonaComputer};
use crate::driver::CapabilityEpoch;

/// The item a test's quiet turn was woken by.
const WOKE: &str = "item-from-goldie";

/// Bob's main conversation has a turn in flight, quiet and woken by
/// `items`, as the inbox's wake makes one (`dm.rs`), for as long as the
/// answer is held.
fn quiet_turn<'a>(room: &'a Room, items: &[&str]) -> crate::session::post::PostingTurn<'a> {
    let dm = ThreadId::dm("bob");
    let turn = room.open_posting(&dm, None);
    let woke: Vec<String> = items.iter().map(|item| (*item).to_string()).collect();
    room.quiet_turn(&dm, &woke);
    turn
}

/// The newest event of `kind` on Bob's tape.
fn newest(room: &Room, kind: &str) -> Value {
    room.tape("bob")
        .into_iter()
        .rfind(|event| event["kind"] == kind)
        .unwrap_or_else(|| panic!("no {kind} on Bob's tape: {:?}", room.tape("bob")))
}

#[tokio::test]
async fn a_file_sent_from_a_quiet_turn_one_item_woke_names_it() {
    let (room, _) = room("prompted-file");
    let name = format!("prompted-file-{}.txt", std::process::id());
    std::fs::write(std::env::temp_dir().join(&name), "the logs").unwrap();
    let turn = quiet_turn(&room, &[WOKE]);
    TeammateTools::new(&room, "bob")
        .call("send_file", &json!({"source": "workspace", "path": name}))
        .await
        .unwrap();
    drop(turn);
    let file = newest(&room, "agent");
    assert!(file["attachments"].is_array(), "{file}");
    assert_eq!(file["fromItem"], WOKE, "{file}");
    std::fs::remove_file(std::env::temp_dir().join(&name)).unwrap();
}

/// A `request_human` card, and an `ask_question` one, name the item, and a
/// card answered keeps naming it; one raised in a turn several items woke,
/// or in one the person reads, names none.
#[tokio::test]
async fn a_card_raised_in_a_quiet_turn_one_item_woke_names_it_and_keeps_it_when_answered() {
    let (room, _) = room("prompted-card");
    let turn = quiet_turn(&room, &[WOKE]);
    let asked = TeammateTools::new(&room, "bob")
        .call("request_human", &json!({"reason": "Sign the crane permit"}))
        .await
        .unwrap();
    let action = asked.split('`').nth(1).unwrap().to_string();
    TeammateTools::new(&room, "bob")
        .with_capability(CapabilityEpoch::default().lease())
        .call(
            "ask_question",
            &json!({"question": "Ship the crane today?"}),
        )
        .await
        .unwrap();
    drop(turn);
    let cards: Vec<Value> = room
        .tape("bob")
        .into_iter()
        .filter(|event| event["kind"] == "human_action")
        .collect();
    assert_eq!(cards.len(), 2, "{cards:?}");
    assert!(
        cards.iter().all(|card| card["fromItem"] == WOKE),
        "{cards:?}"
    );

    room.answer_human("bob", &action, HumanAnswer::Done, None)
        .unwrap();
    let answered = room
        .tape("bob")
        .into_iter()
        .rfind(|event| event["id"] == format!("human:{action}"))
        .unwrap();
    assert_eq!(answered["status"], "done");
    assert_eq!(answered["fromItem"], WOKE, "{answered}");

    let two = quiet_turn(&room, &[WOKE, "item-from-quill"]);
    TeammateTools::new(&room, "bob")
        .call("request_human", &json!({"reason": "Check the winch"}))
        .await
        .unwrap();
    drop(two);
    assert!(newest(&room, "human_action").get("fromItem").is_none());

    let _loud = room.open_posting(&ThreadId::dm("bob"), None);
    TeammateTools::new(&room, "bob")
        .call("request_human", &json!({"reason": "Look at the harbour"}))
        .await
        .unwrap();
    assert!(newest(&room, "human_action").get("fromItem").is_none());
}

/// A permission card the agent asks in a quiet turn one item woke names it.
#[tokio::test]
async fn a_permission_the_agent_asks_in_a_quiet_turn_one_item_woke_names_it() {
    let asking = Scripted::new(vec![
        Update::Permission {
            request_id: "edit-1".into(),
            title: "Edit Cargo.toml".into(),
            options: Vec::new(),
        },
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    ]);
    let (room, _) = room_on("prompted-permission", asking);
    at_rest(&room, "bob").await;
    let id = send(&room, "ada", "bob", "Bump the version", json!({})).await;
    until("Bob's quiet turn raised its card", || {
        room.tape("bob")
            .iter()
            .any(|event| event["kind"] == "permission")
    })
    .await;
    assert_eq!(newest(&room, "permission")["fromItem"], id.as_str());
}

/// A card up in access that a quiet turn's `send` raises names the item
/// that woke the turn: the person sees a colleague is behind the request.
#[tokio::test]
async fn a_card_up_in_access_raised_by_a_quiet_turns_send_names_it() {
    let log = scratch("prompted-up-in-access");
    enrol(&log, &named("ada", "Ada", Some(Reach::Machine)));
    enrol(&log, &named("bob", "Bob", None));
    let room = Room::with_agents(log, Arc::new(DeskKeys), Fake::new(replies()));
    let turn = quiet_turn(&room, &[WOKE]);
    TeammateTools::new(&room, "bob")
        .call(
            "send",
            &json!({"to": ["ada"], "title": "Restart the runner", "wants": "work"}),
        )
        .await
        .unwrap();
    drop(turn);
    until("the card is up", || {
        room.tape("bob")
            .iter()
            .any(|event| event["kind"] == "permission")
    })
    .await;
    assert_eq!(newest(&room, "permission")["fromItem"], WOKE);
}

/// A credential request names the item, and so does the card once it is
/// answered.
#[tokio::test]
async fn a_credential_request_raised_in_a_quiet_turn_one_item_woke_names_it() {
    let log = scratch("prompted-credential");
    let mut bob = named("bob", "Bob", Some(Reach::Machine));
    bob.computer = Some(PersonaComputer {
        enabled: true,
        cpus: None,
        memory: None,
        pids: None,
        image: None,
        mounts: None,
    });
    enrol(&log, &bob);
    let vault = Arc::new(
        crate::vault::Vault::open_with_store(
            log.root(),
            log.clone(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    let room = Room::with_agents_computers_and_vault(
        log,
        Arc::new(DeskKeys),
        Fake::new(replies()),
        crate::computer::Computer::new(),
        Some(vault),
    );
    let turn = quiet_turn(&room, &[WOKE]);
    TeammateTools::new(&room, "bob")
        .with_capability(CapabilityEpoch::default().lease())
        .call(
            "request_credential",
            &json!({"site": "https://github.com", "kind": "login", "name": "GITHUB_LOGIN", "reason": "Sign in to the repository"}),
        )
        .await
        .unwrap();
    drop(turn);
    let card = newest(&room, "credential_ask");
    assert_eq!(card["fromItem"], WOKE, "{card}");
}

/// A sign-in card (`ask_cookies`) names the item.
#[cfg(unix)]
#[tokio::test]
async fn a_sign_in_card_raised_in_a_quiet_turn_one_item_woke_names_it() {
    let (fixture, _profiles) =
        crate::session::tests::cookie_offer_room("prompted-cookie", true).await;
    let room = &fixture.room;
    let dm = ThreadId::dm("ada");
    let turn = room.open_posting(&dm, None);
    room.quiet_turn(&dm, &[WOKE.to_string()]);
    room.session_tools("ada")
        .call(
            "ask_cookies",
            &json!({"site": "porkbun.com", "reason": "To open the DNS page for the domain."}),
        )
        .await
        .unwrap();
    drop(turn);
    let card = room
        .tape("ada")
        .into_iter()
        .rfind(|event| event["kind"] == "cookie_ask")
        .unwrap();
    assert_eq!(card["fromItem"], WOKE, "{card}");
}

/// A job a quiet turn one item woke makes records the item, and its
/// listing carries it; one made in a turn the person reads does not.
#[tokio::test]
async fn a_job_made_in_a_quiet_turn_one_item_woke_records_it() {
    let (room, _) = room("prompted-job");
    let turn = quiet_turn(&room, &[WOKE]);
    let tools = TeammateTools::new(&room, "bob");
    tools
        .call(
            "schedule",
            &json!({"when": "2h", "prompt": "Check the crane permit came back"}),
        )
        .await
        .unwrap();
    tools
        .call(
            "loop",
            &json!({"every": "1h", "prompt": "Watch the winch oil"}),
        )
        .await
        .unwrap();
    drop(turn);
    let _loud = room.open_posting(&ThreadId::dm("bob"), None);
    tools
        .call(
            "schedule",
            &json!({"when": "3h", "prompt": "Ask about the harbour"}),
        )
        .await
        .unwrap();
    let jobs = crate::room::schedules(&room.log);
    let from = |prompt: &str| {
        jobs.iter()
            .find(|job| job.prompt == prompt)
            .unwrap()
            .from_item
            .clone()
    };
    assert_eq!(
        from("Check the crane permit came back").as_deref(),
        Some(WOKE)
    );
    assert_eq!(from("Watch the winch oil").as_deref(), Some(WOKE));
    assert_eq!(from("Ask about the harbour"), None);
    let job = jobs
        .iter()
        .find(|job| job.prompt == "Watch the winch oil")
        .unwrap();
    assert_eq!(
        crate::session::schedule::entry(job.clone())
            .from_item
            .as_deref(),
        Some(WOKE)
    );
}
