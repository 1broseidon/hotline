//! The brake as the person answers it: Stop exchange holds the two apart
//! until the person writes to one of them, and a pause waits for its card,
//! whatever else the person says meanwhile.

use super::*;

/// Ada and Bob one item short of the cap, then paused by Ada's twelfth.
async fn paused(room: &Arc<Room>) -> String {
    room.log
        .append(
            &StreamId::Room,
            &json!({"kind": "exchange_pair", "id": "ada~bob", "a": "ada", "b": "bob",
                "exchanges": 11, "paused": false, "requests": []}),
        )
        .unwrap();
    let id = send(room, "ada", "bob", "Twelfth", json!({"wants": "work"})).await;
    assert_eq!(pair(room).unwrap()["paused"], true);
    id
}

fn pending_cards(room: &Room, persona_id: &str) -> usize {
    room.tape(persona_id)
        .iter()
        .filter(|event| event["kind"] == "exchange_paused" && event["status"] == "pending")
        .count()
}

#[tokio::test]
async fn after_stop_nothing_passes_between_the_two_until_the_person_writes_to_one_of_them() {
    let (room, _) = room("inbox-stop-holds");
    at_rest(&room, "bob").await;
    let twelfth = paused(&room).await;
    room.stop_exchange("ada", "bob").unwrap();
    let stopped = pair(&room).unwrap();
    assert_eq!(stopped["paused"], false);
    assert_eq!(stopped["stopped"], true);
    assert_eq!(pending_cards(&room, "ada"), 0, "the card is answered");

    let refused = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["bob"], "title": "One more"}),
    )
    .await
    .unwrap_err();
    assert!(
        refused.contains("The person stopped the exchange between you and Bob")
            && refused.contains("Nothing was sent."),
        "{refused}"
    );
    let answer = tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": twelfth, "answer": "Done anyway."}]}),
    )
    .await
    .unwrap_err();
    assert!(answer.contains("stopped the exchange"), "{answer}");
    let taken = tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": twelfth, "state": "taken"}]}),
    )
    .await
    .unwrap_err();
    assert!(taken.contains("stopped the exchange"), "{taken}");
    assert!(room.sides("bob").is_empty(), "no work started");
    assert_eq!(items(&room, "ada").len(), 0, "nothing reached Ada");
    assert_eq!(pair(&room).unwrap()["exchanges"], 0);

    // The person writes to one of the two: they may reach each other again,
    // counting from nothing.
    room.prompt("bob", "carry on with Ada", None, None)
        .await
        .unwrap();
    let released = pair(&room).unwrap();
    assert_eq!(released["exchanges"], 0);
    assert!(released.get("stopped").is_none(), "{released}");
    send(&room, "ada", "bob", "Thirteenth", json!({})).await;
    assert_eq!(pair(&room).unwrap()["exchanges"], 1);
}

#[tokio::test]
async fn a_pause_waits_for_its_card_and_an_unrelated_person_line_does_not_release_it() {
    let (room, agents) = room("inbox-pause-holds");
    let session = at_rest(&room, "ada").await;
    paused(&room).await;
    room.prompt("ada", "what's for lunch?", None, None)
        .await
        .unwrap();
    until("ada's turn", || agents.told().len() == 1).await;
    until("at rest", || !lock(&session.turns).running).await;
    let held = pair(&room).unwrap();
    assert_eq!(held["paused"], true, "{held}");
    assert_eq!(held["exchanges"], 12, "{held}");
    assert_eq!(pending_cards(&room, "ada"), 1, "the card still asks");
    let refused = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["bob"], "title": "One more"}),
    )
    .await
    .unwrap_err();
    assert!(refused.contains("paused at 12"), "{refused}");

    room.resume_exchange("ada", "bob").unwrap();
    assert_eq!(pending_cards(&room, "ada"), 0);
    send(&room, "ada", "bob", "After keep going", json!({})).await;
    assert_eq!(pair(&room).unwrap()["exchanges"], 1);
}
