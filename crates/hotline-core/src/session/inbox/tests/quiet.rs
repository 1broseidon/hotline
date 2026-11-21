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
    assert!(
        agents.told()[0].ends_with(&crate::session::returns::note(0, false)),
        "and nobody waits on it: {}",
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
        .post_to_chat(
            "bob",
            &dm,
            "The crane permit needs your signature.",
            false,
            Some(crate::session::post::PostKind::Decision),
        )
        .await;
    assert_eq!(posted.as_deref(), Ok("Posted to the chat."));
    let again = room
        .post_to_chat(
            "bob",
            &dm,
            "And again.",
            false,
            Some(crate::session::post::PostKind::Decision),
        )
        .await;
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
        .post_to_chat(
            "bob",
            &dm,
            "Posting instead.",
            false,
            Some(crate::session::post::PostKind::Decision),
        )
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

#[tokio::test]
async fn a_notice_steered_into_a_quiet_turn_leaves_it_quiet() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on(
        "inbox-quiet-notice-steered",
        replies().gated(gate.clone()).consuming(),
    );
    pair_a_phone(&room);
    at_rest(&room, "bob").await;
    send(&room, "ada", "bob", "Deploy notes", json!({})).await;
    until("bob woke", || agents.told().len() == 1).await;
    send(&room, "ada", "bob", "Logo colours", json!({})).await;
    until("the notice joined the turn", || agents.steered().len() == 1).await;
    assert!(agents.steered()[0].contains("\"Logo colours\""));
    assert!(room.is_quiet(&ThreadId::dm("bob")), "still quiet");
    gate.add_permits(100);
    until("the turn ended", || turn_ended(&room, "bob")).await;
    tokio::time::sleep(heard::gather_delay() * 3).await;
    assert!(
        room.tape("bob")
            .iter()
            .all(|event| event["kind"] != "agent"),
        "no reply: {:?}",
        room.tape("bob")
    );
    assert!(crate::store::previews::preview(room.log.root(), "bob").is_none());
    assert!(lock(&room.push.sent).is_empty(), "no push");
}

#[tokio::test]
async fn a_notice_its_driver_did_not_take_mid_turn_starts_a_quiet_turn_with_no_bubble_unread_or_push()
 {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on("inbox-quiet-notice-turn", replies().gated(gate.clone()));
    pair_a_phone(&room);
    at_rest(&room, "bob").await;
    // The person's own turn, out loud, which the driver will not steer into.
    room.prompt("bob", "the person's work", None, None)
        .await
        .unwrap();
    until("bob is working", || agents.prompts().len() == 1).await;
    send(&room, "ada", "bob", "Deploy notes", json!({})).await;
    tokio::time::sleep(heard::gather_delay() * 3).await;
    gate.add_permits(100);
    until("the person's turn and the notice's", || {
        room.tape("bob")
            .iter()
            .filter(|event| event["kind"] == "turn")
            .count()
            == 2
    })
    .await;
    assert_eq!(
        kinds_of(&room, "bob", "said-0"),
        ["agent"],
        "the person's reply"
    );
    let told = &agents.told()[1];
    assert!(
        told.contains(heard::QUIET_WAKE_LINE) && told.contains("\"Deploy notes\""),
        "the notice's turn is the quiet wake: {told}"
    );
    assert_eq!(
        kinds_of(&room, "bob", "said-1"),
        ["thought"],
        "the notice's turn says nothing in the chat"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        lock(&room.push.sent).len(),
        1,
        "the person's reply pushes, the notice's turn does not"
    );
    let preview = crate::store::previews::preview(room.log.root(), "bob");
    assert!(
        preview
            .as_ref()
            .is_some_and(|preview| !format!("{preview:?}").contains("said-1")),
        "the rail counts nothing of the notice's turn: {preview:?}"
    );
}

/// A post from a quiet turn one item woke names that item, so the window
/// says whose message prompted it, and the turn's end says it was quiet;
/// the person's own turn after it names nothing and is not quiet.
#[tokio::test]
async fn a_quiet_turns_post_names_the_item_that_woke_it_and_its_end_says_it_was_quiet() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on("inbox-quiet-woken-by", replies().gated(gate.clone()));
    at_rest(&room, "bob").await;
    let dm = ThreadId::dm("bob");
    let id = send(&room, "ada", "bob", "Crane permit", json!({})).await;
    until("bob woke", || agents.told().len() == 1).await;
    room.post_to_chat(
        "bob",
        &dm,
        "The crane permit needs your signature.",
        false,
        Some(crate::session::post::PostKind::Decision),
    )
    .await
    .unwrap();
    gate.add_permits(2);
    until("the quiet turn ended", || turn_ended(&room, "bob")).await;
    let tape = room.tape("bob");
    let post = tape.iter().find(|event| event["kind"] == "agent").unwrap();
    assert_eq!(post["fromItem"], id.as_str(), "{post}");
    let quiet_end = tape.iter().find(|event| event["kind"] == "turn").unwrap();
    assert_eq!(quiet_end["quiet"], true, "{quiet_end}");

    room.prompt("bob", "where is the crane", None, None)
        .await
        .unwrap();
    until("the person's turn", || agents.prompts().len() == 2).await;
    gate.add_permits(2);
    until("the person's turn ended", || {
        room.tape("bob")
            .iter()
            .filter(|event| event["kind"] == "turn")
            .count()
            == 2
    })
    .await;
    let tape = room.tape("bob");
    let loud_end = tape.iter().rfind(|event| event["kind"] == "turn").unwrap();
    assert!(loud_end.get("quiet").is_none(), "{loud_end}");
    let reply = tape.iter().rfind(|event| event["kind"] == "agent").unwrap();
    assert!(reply.get("fromItem").is_none(), "{reply}");
}

/// A quiet turn that several items woke together could be posting about any
/// of them, so its post names none.
#[tokio::test]
async fn a_post_from_a_quiet_turn_several_items_woke_names_none() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on("inbox-quiet-woken-by-two", replies().gated(gate.clone()));
    at_rest(&room, "bob").await;
    let dm = ThreadId::dm("bob");
    send(&room, "ada", "bob", "Crane permit", json!({})).await;
    send(&room, "ada", "bob", "Winch oil", json!({})).await;
    until("bob woke once for both", || agents.told().len() == 1).await;
    assert!(agents.told()[0].contains("NEW 2"), "{}", agents.told()[0]);
    room.post_to_chat(
        "bob",
        &dm,
        "Two things from Ada.",
        false,
        Some(crate::session::post::PostKind::Decision),
    )
    .await
    .unwrap();
    let post = room
        .tape("bob")
        .into_iter()
        .find(|event| event["kind"] == "agent")
        .unwrap();
    assert!(post.get("fromItem").is_none(), "{post}");
    gate.add_permits(2);
    until("the quiet turn ended", || turn_ended(&room, "bob")).await;
}
