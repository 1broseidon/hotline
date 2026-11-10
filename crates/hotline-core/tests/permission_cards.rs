//! A permission card nobody answers, through the real core, a real ACP child
//! and the real wire (task-419).
//!
//! Seen on a desk left alone: a harness asked to run a shell command, waited
//! its ten minutes, and was told nobody answered. It carried on with its
//! turn. The card stayed up with its buttons, the thread went on reading as
//! waiting on the person, and the person's answer, typed into the chat when
//! the buttons did nothing, was read as approval of something else. Here the
//! wait is seconds, and the agent keeps its turn open after it is told, as
//! that one did.
use crate::common::{self, Client, acp_fixture, until};

use hotline_core::desk::Desk;
use hotline_core::driver::acp::Bounds;
use hotline_core::log::{Log, StreamId};
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

const TOKEN: &str = "disposable-permission-cards-test";
/// How long a card waits here: long enough for the window's half to see the
/// teammate waiting on it, short enough for a test.
const WAIT: Duration = Duration::from_secs(2);
/// What the wire says to an answer nothing is behind.
const REFUSED: &str = "That request is no longer waiting for an answer.";

/// The latest line of each permission card on the tape, oldest card first.
fn cards(log: &Log, persona_id: &str) -> Vec<Value> {
    let mut cards: Vec<Value> = Vec::new();
    for event in log.load(&StreamId::Tape(persona_id.to_string())) {
        if event["kind"] != "permission" {
            continue;
        }
        match cards.iter_mut().find(|card| card["id"] == event["id"]) {
            Some(card) => *card = event,
            None => cards.push(event),
        }
    }
    cards
}

/// The commands the agent has been told the outcome of, each with what it was
/// told, in the order it heard.
fn told(log: &Log, persona_id: &str) -> Vec<(String, String)> {
    log.load(&StreamId::Tape(persona_id.to_string()))
        .into_iter()
        .filter(|event| {
            event["kind"] == "tool"
                && matches!(event["status"].as_str(), Some("completed" | "failed"))
        })
        .map(|event| {
            (
                event["title"].as_str().unwrap_or_default().to_string(),
                event["output"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_card_nobody_answers_expires_where_it_stands_and_a_late_answer_is_refused() {
    let root = tempfile::tempdir().unwrap();
    acp_fixture::install(root.path());
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let desk = Desk::open_with_acp_bounds(
        root.path(),
        common::store(),
        Bounds {
            permission: WAIT,
            ..Bounds::default()
        },
    )
    .unwrap();
    let log = desk.log.clone();
    let parts = desk.parts();
    let door = Door::bind(desk.log.clone(), TOKEN.to_string(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    let mut client = Client::connect(port, TOKEN).await;

    let made = client
        .call(
            "persona.create",
            json!({"draft": {
                "name": "Pork Chop", "goal": "Code", "cwd": workspace.to_string_lossy(),
                "backendId": acp_fixture::BACKEND,
            }}),
        )
        .await;
    assert_eq!(made["ok"], true, "{made}");
    let ada = made["result"]["id"].as_str().unwrap().to_owned();
    let started = client
        .call("session.start", json!({"personaId": ada}))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let tape = client.subscribe(json!({"tape": ada})).await;
    let roster = client.subscribe(json!({"view": "roster"})).await;
    let waiting = |waiting: bool| {
        let ada = ada.clone();
        move |frame: &Value| {
            frame["sub"] == roster
                && frame["event"]["persona"]["id"] == ada.as_str()
                && frame["event"]["waiting"] == waiting
        }
    };

    // Two commands, the second held back until the harness says, and then the
    // turn stays open: the agent goes on working after it is refused, as the
    // one on the desk did.
    let go_on = root.path().join("go-on");
    let release = root.path().join("release");
    let prompt = format!(
        "CALL ask_permission {}\nCALL wait_for {}\nCALL ask_permission {}\nWAIT_FOR {}",
        json!({"command": "rm -rf build"}),
        json!({"path": go_on}),
        json!({"command": "rm -rf dist"}),
        release.display()
    );
    let sent = client
        .call("session.prompt", json!({"personaId": ada, "text": prompt}))
        .await;
    assert_eq!(sent["ok"], true, "{sent}");

    // The first card goes up live, and the teammate reads as waiting. Both
    // are read off the wire as they were sent, so neither races the wait.
    let first = client
        .next_where(common::ANSWER, |frame| {
            frame["sub"] == tape && frame["event"]["kind"] == "permission"
        })
        .await["event"]
        .clone();
    assert_eq!(first["title"], "Run rm -rf build");
    assert!(first.get("decision").is_none(), "the card is live: {first}");
    let first_id = first["requestId"].as_str().unwrap().to_owned();
    client.next_where(common::ANSWER, waiting(true)).await;
    // Rows from before the card are not rows from after it.
    client.inbox.clear();

    // Nobody answers. The agent is told so when the wait runs out.
    until("the agent is told nobody answered", || {
        !told(&log, &ada).is_empty()
    })
    .await;
    assert_eq!(
        told(&log, &ada),
        [(
            "rm -rf build".to_string(),
            "User refused permission to run tool".to_string()
        )]
    );

    // The card says it expired, on the tape, where it stood, while the turn
    // that raised it is still running: it was the wait that settled it, not
    // the turn's end.
    let expired = cards(&log, &ada).remove(0);
    assert_eq!(
        expired["decision"], "expired",
        "the agent was told nobody answered and its card still reads as live: {expired}"
    );
    assert_eq!(expired["decidedOptionName"], "Expired unanswered");
    assert_eq!(expired["id"], first["id"], "one card, superseded");
    assert!(
        !log.load(&StreamId::Tape(ada.clone()))
            .iter()
            .any(|event| event["kind"] == "turn"),
        "the turn is still open"
    );
    // And the teammate stops reading as waiting on the person.
    client.next_where(common::ANSWER, waiting(false)).await;

    // The agent goes on to its second command, which asks in its turn.
    std::fs::write(&go_on, "").unwrap();
    until("the second card is up", || cards(&log, &ada).len() == 2).await;
    let second_id = cards(&log, &ada)[1]["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(first_id, second_id, "each request has an id of its own");

    // The answer the person gives when they come back names the first card.
    // It is refused over both commands a seat may send, with the reason.
    let thread = json!({"kind": "dm", "key": ada});
    let late = client
        .call(
            "thread.answer",
            json!({"thread": thread, "answer": {
                "kind": "permission", "requestId": first_id, "optionId": "allow",
            }}),
        )
        .await;
    let late_from_a_phone = client
        .call(
            "session.answer_permission",
            json!({"personaId": ada, "requestId": first_id, "optionId": "allow"}),
        )
        .await;

    assert_eq!(late["ok"], false, "{late}");
    assert_eq!(late["error"], REFUSED, "{late}");
    assert_eq!(late_from_a_phone["ok"], false, "{late_from_a_phone}");
    assert_eq!(late_from_a_phone["error"], REFUSED, "{late_from_a_phone}");

    // The late answer reached nothing: not the expired request, which the
    // agent heard about once, and not the later one, which is still waiting
    // and still unanswered.
    assert_eq!(told(&log, &ada).len(), 1, "the agent heard nothing more");
    let after = cards(&log, &ada);
    assert_eq!(after[0]["decision"], "expired", "{}", after[0]);
    assert!(
        after[1].get("decision").is_none(),
        "an answer to the first card answered the second: {}",
        after[1]
    );

    // The second card is answered in time, by its own id, and its command runs.
    let answered = client
        .call(
            "thread.answer",
            json!({"thread": thread, "answer": {
                "kind": "permission", "requestId": second_id, "optionId": "allow",
            }}),
        )
        .await;
    assert_eq!(answered["ok"], true, "{answered}");
    until("the second command runs", || told(&log, &ada).len() == 2).await;
    assert_eq!(
        told(&log, &ada)[1],
        ("rm -rf dist".to_string(), "ran".to_string())
    );
    let settled = cards(&log, &ada);
    assert_eq!(settled[0]["decision"], "expired");
    assert_eq!(settled[1]["decision"], "allow");

    // The turn ends when the agent ends it, and leaves both cards as they were.
    std::fs::write(&release, "").unwrap();
    until("the turn ends", || {
        log.load(&StreamId::Tape(ada.clone()))
            .iter()
            .any(|event| event["kind"] == "turn")
    })
    .await;
    let ended = cards(&log, &ada);
    assert_eq!(ended[0]["decision"], "expired");
    assert_eq!(ended[1]["decision"], "allow");
    assert_eq!(ended[1]["decidedOptionName"], "Allow");
}
