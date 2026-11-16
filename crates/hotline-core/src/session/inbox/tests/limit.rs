//! The hourly limit on quiet turns (task-472): a teammate that begins as
//! many quiet turns in an hour as the person allows is paused before the
//! next with the pair's card, Keep going counts afresh and lets it hear what
//! waited, and Stop holds it until the person writes to it. Each pause is a
//! line in `brakes.jsonl`. The hour itself is `limit.rs`'s unit test, on a
//! clock it chooses.

use super::*;

/// The person's limit, as Settings writes it.
fn allow(room: &Room, turns: i64) {
    room.log
        .append(
            &StreamId::Room,
            &json!({"kind": "setting", "id": "quietTurnsPerHour", "value": turns}),
        )
        .unwrap();
}

/// How many quiet turns Bob's main conversation has ended.
fn quiet_turns(room: &Room) -> usize {
    room.tape("bob")
        .iter()
        .filter(|event| event["kind"] == "turn" && event["quiet"] == true)
        .count()
}

/// Bob's limit cards, by status.
fn cards(room: &Room, status: &str) -> Vec<Value> {
    room.tape("bob")
        .into_iter()
        .filter(|event| event["kind"] == "quiet_paused" && event["status"] == status)
        .collect()
}

/// What `brakes.jsonl` noted.
fn noted(room: &Room) -> Vec<Value> {
    std::fs::read_to_string(room.log.root().join("brakes.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Ada sends Bob an item, and Bob's quiet turn for it ends.
async fn quiet_turn_for(room: &Arc<Room>, title: &str) {
    let before = quiet_turns(room);
    send(room, "ada", "bob", title, json!({})).await;
    until(&format!("Bob's quiet turn for {title}"), || {
        quiet_turns(room) == before + 1
    })
    .await;
}

/// Ada sends Bob an item past his limit: the card is up and no quiet turn
/// begins for it.
async fn paused_by(room: &Arc<Room>, title: &str, pending: usize) {
    let before = quiet_turns(room);
    send(room, "ada", "bob", title, json!({})).await;
    until("Bob is paused with the card", || {
        cards(room, "pending").len() == pending
    })
    .await;
    tokio::time::sleep(heard::gather_delay() * 4).await;
    assert_eq!(quiet_turns(room), before, "no quiet turn began for {title}");
}

#[tokio::test]
async fn a_teammate_past_its_hourly_limit_is_paused_with_the_card_and_keep_going_counts_afresh() {
    let (room, _) = room("inbox-limit-keep-going");
    allow(&room, 2);
    at_rest(&room, "bob").await;
    quiet_turn_for(&room, "First").await;
    quiet_turn_for(&room, "Second").await;
    paused_by(&room, "Third", 1).await;
    let card = &cards(&room, "pending")[0];
    assert_eq!(card["turns"], 2, "{card}");
    let brake = noted(&room);
    assert_eq!(brake.len(), 1, "{brake:?}");
    assert_eq!(brake[0]["brake"], "quiet");
    assert_eq!(brake[0]["teammate"], "bob");
    assert_eq!(brake[0]["count"], 2);
    assert_eq!(brake[0]["limit"], 2);
    assert!(brake[0]["at"].is_i64());

    room.answer_quiet_pause("bob", true).unwrap();
    assert_eq!(cards(&room, "resumed").len(), 1);
    until("Bob hears what waited", || quiet_turns(&room) == 3).await;
    // Counted afresh from Keep going: the third was one, this is two.
    quiet_turn_for(&room, "Fourth").await;
    paused_by(&room, "Fifth", 1).await;
    assert_eq!(noted(&room).len(), 2);
}

#[tokio::test]
async fn stop_holds_a_teammate_at_its_limit_until_the_person_writes_to_it() {
    let (room, agents) = room("inbox-limit-stop");
    allow(&room, 1);
    at_rest(&room, "bob").await;
    quiet_turn_for(&room, "First").await;
    paused_by(&room, "Second", 1).await;
    room.answer_quiet_pause("bob", false).unwrap();
    assert_eq!(cards(&room, "stopped").len(), 1);
    assert!(
        room.answer_quiet_pause("bob", true).is_err(),
        "a pause answered is answered"
    );
    send(&room, "ada", "bob", "Third", json!({})).await;
    tokio::time::sleep(heard::gather_delay() * 4).await;
    assert_eq!(quiet_turns(&room), 1, "nothing quiet begins while stopped");

    let asked = agents.prompts().len();
    room.prompt("bob", "What is waiting for you?", None, None)
        .await
        .unwrap();
    until("the person's turn ran", || agents.prompts().len() > asked).await;
    at_rest(&room, "bob").await;
    quiet_turn_for(&room, "Fourth").await;
    assert_eq!(quiet_turns(&room), 2, "the person's line lifted the stop");
}
