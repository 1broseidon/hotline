//! A turn of the main conversation that the inbox woke is quiet by
//! construction: nobody reads its words, so they are kept as the path under
//! the turn and tell no phone, and its one post is its way to the person. The
//! person's own line steered into it is answered out loud.

use super::*;
use crate::session::post::AGAIN;
use crate::session::tests::pair_a_phone;
use std::sync::Arc;

fn kinds_of(room: &Room, persona_id: &str, text: &str) -> Vec<String> {
    room.tape(persona_id)
        .into_iter()
        .filter(|event| event["text"] == text)
        .filter_map(|event| event["kind"].as_str().map(str::to_string))
        .collect()
}

fn turn_ended(room: &Room, persona_id: &str) -> bool {
    room.tape(persona_id)
        .iter()
        .any(|event| event["kind"] == "turn")
}

#[tokio::test]
async fn a_turn_the_inbox_woke_keeps_its_words_as_thinking_and_tells_no_phone() {
    let (room, agents) = room("inbox-quiet-words");
    pair_a_phone(&room);
    at_rest(&room, "bob").await;
    send(&room, "ada", "bob", "Test message", json!({})).await;
    until("bob woke and finished", || turn_ended(&room, "bob")).await;
    assert!(
        agents.told()[0].contains(heard::QUIET_WAKE_LINE),
        "the line says who reads the turn: {}",
        agents.told()[0]
    );
    assert_eq!(kinds_of(&room, "bob", "said-0"), ["thought"]);
    assert!(
        room.tape("bob")
            .iter()
            .all(|event| event["kind"] != "agent"),
        "no reply: {:?}",
        room.tape("bob")
    );
    assert!(
        crate::store::previews::preview(room.log.root(), "bob").is_none(),
        "nothing for the rail to count as unread"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(lock(&room.push.sent).is_empty(), "no push");
}

#[tokio::test]
async fn a_quiet_turn_posts_to_the_chat_once_and_a_turn_the_person_started_may_not() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on("inbox-quiet-post", replies().gated(gate.clone()));
    pair_a_phone(&room);
    at_rest(&room, "bob").await;
    let dm = ThreadId::dm("bob");
    send(&room, "ada", "bob", "Crane permit", json!({})).await;
    until("bob woke", || agents.told().len() == 1).await;

    let posted = room
        .post_to_chat("bob", &dm, "The crane permit needs your signature.", false)
        .await;
    assert_eq!(posted.as_deref(), Ok("Posted to the chat."));
    let again = room.post_to_chat("bob", &dm, "And again.", false).await;
    assert_eq!(again, Err(AGAIN.to_string()));
    assert_eq!(
        kinds_of(&room, "bob", "The crane permit needs your signature."),
        ["agent"],
        "the post is an ordinary message in the chat"
    );
    let post = room
        .tape("bob")
        .into_iter()
        .find(|event| event["kind"] == "agent")
        .unwrap();
    assert!(post.get("from").is_none(), "{post}");
    assert_eq!(lock(&room.push.sent).len(), 1, "the post pushes");

    gate.add_permits(2);
    until("the quiet turn ended", || turn_ended(&room, "bob")).await;
    assert_eq!(kinds_of(&room, "bob", "said-0"), ["thought"]);

    room.prompt("bob", "where is the crane", None, None)
        .await
        .unwrap();
    until("the person's turn", || agents.prompts().len() == 2).await;
    let loud = room
        .post_to_chat("bob", &dm, "Posting instead.", false)
        .await;
    assert!(
        loud.as_ref()
            .is_err_and(|error| error.contains("already the chat")),
        "{loud:?}"
    );
    gate.add_permits(2);
    until("the person is answered", || {
        kinds_of(&room, "bob", "said-1") == ["agent"]
    })
    .await;
}

#[tokio::test]
async fn the_persons_line_steered_into_a_quiet_turn_is_answered_out_loud() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on(
        "inbox-quiet-steered",
        replies().gated(gate.clone()).consuming(),
    );
    pair_a_phone(&room);
    at_rest(&room, "bob").await;
    send(&room, "ada", "bob", "Deploy notes", json!({})).await;
    until("bob woke", || agents.told().len() == 1).await;
    assert!(room.is_quiet(&ThreadId::dm("bob")));

    room.prompt("bob", "are you there", None, None)
        .await
        .unwrap();
    until("the person's line joined the turn", || {
        agents.steered().len() == 1
    })
    .await;
    gate.add_permits(2);
    until("the turn ended", || turn_ended(&room, "bob")).await;
    assert_eq!(
        kinds_of(&room, "bob", "said-0"),
        ["agent"],
        "a reply to the person"
    );
    until("the phone is told the reply", || {
        lock(&room.push.sent).len() == 1
    })
    .await;
}
