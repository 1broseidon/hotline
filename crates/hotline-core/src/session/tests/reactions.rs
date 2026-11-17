//! Reactions, both ways, on a scripted agent: the person's emoji alone is
//! said to be a reaction and waits on no reply; a line said while the
//! teammate works is said to be one, and the teammate's next message answers
//! it as chat whatever follows; and a line handed back to run on its own is
//! heard without the working mark.

use super::*;
use crate::session::marks::{MID_TURN, REACTION};

fn says(id: &str, text: &str) -> Update {
    Update::Message {
        kind: MessageKind::Agent,
        id: id.to_string(),
        text: text.to_string(),
    }
}

fn calls(id: &str) -> Update {
    Update::ToolCall {
        call_id: id.to_string(),
        title: format!("step {id}"),
        kind: "read".to_string(),
    }
}

fn returns(id: &str) -> Update {
    Update::ToolResult {
        call_id: id.to_string(),
        ok: true,
        output: "ok".to_string(),
        images: Vec::new(),
    }
}

fn ends() -> Update {
    Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }
}

fn kind_of(room: &Room, id: &str) -> Option<String> {
    tape(room, "ada")
        .into_iter()
        .rfind(|event| event["id"] == id)
        .and_then(|event| event["kind"].as_str().map(str::to_string))
}

/// An emoji alone is said to be a reaction, and runs as a line nobody waits
/// on, so an empty answer leaves nothing. Words are said as they are.
#[tokio::test]
async fn an_emoji_alone_is_said_to_be_a_reaction_and_waits_on_no_reply() {
    let agents = Fake::new(Scripted::new(vec![ends()]));
    let room = room("reaction-door", agents.clone());
    room.start("ada").await.unwrap();
    room.prompt("ada", "👍", None, None).await.unwrap();
    until("the reaction's turn", || agents.prompts().len() == 1).await;
    until_state(&room, "ada", SessionState::Ready).await;
    assert!(agents.told()[0].ends_with(REACTION), "{}", agents.told()[0]);
    assert_eq!(agents.unasked().len(), 1, "nobody waits on a reply to it");

    room.prompt("ada", "ok", None, None).await.unwrap();
    until("the words' turn", || agents.prompts().len() == 2).await;
    assert!(!agents.told()[1].contains(REACTION), "{}", agents.told()[1]);
    assert_eq!(agents.unasked().len(), 1, "words wait on a reply");
}

/// A question said while the teammate works joins its turn with the working
/// mark. The answer is chat although more tool calls follow it and the turn
/// ends with no last word; narration before it and after it is thinking.
#[tokio::test]
async fn the_answer_to_a_question_said_mid_turn_reaches_the_chat() {
    let turn = vec![
        says("m-ack", "On it."),
        calls("c1"),
        returns("c1"),
        says("m-narrate", "Reading the logs next."),
        calls("c2"),
        returns("c2"),
        says(
            "m-answer",
            "About halfway: the row is in, the pane is next.",
        ),
        calls("c3"),
        returns("c3"),
        says("m-after", "Now the pane."),
        calls("c4"),
        returns("c4"),
        ends(),
    ];
    let gate = Arc::new(Semaphore::new(0));
    let agents = Fake::new(Scripted::turns(vec![turn]).gated(gate.clone()).consuming());
    let room = room("reaction-mid-turn", agents.clone());
    room.start("ada").await.unwrap();
    room.prompt("ada", "build it", None, None).await.unwrap();
    gate.add_permits(5);
    until("the narration is held", || {
        tape(&room, "ada")
            .iter()
            .filter(|event| event["kind"] == "tool")
            .count()
            >= 2
    })
    .await;

    room.prompt("ada", "how far along?", None, None)
        .await
        .unwrap();
    until("the question joined the turn", || {
        agents.steered().len() == 1
    })
    .await;
    assert!(
        agents.steered()[0].ends_with(MID_TURN),
        "{}",
        agents.steered()[0]
    );
    gate.add_permits(100);
    until_state(&room, "ada", SessionState::Ready).await;
    assert_eq!(kind_of(&room, "m-ack").as_deref(), Some("agent"));
    assert_eq!(kind_of(&room, "m-narrate").as_deref(), Some("thought"));
    assert_eq!(
        kind_of(&room, "m-answer").as_deref(),
        Some("agent"),
        "the answer is chat"
    );
    assert_eq!(kind_of(&room, "m-after").as_deref(), Some("thought"));
}

/// An emoji said mid-turn carries the reaction's mark, and one the teammate
/// answers with a reaction leaves no stray message: its narration after is
/// thinking as before.
#[tokio::test]
async fn an_emoji_said_mid_turn_answered_with_a_reaction_leaves_no_stray_bubble() {
    let turn = vec![
        says("m-ack", "On it."),
        calls("c1"),
        returns("c1"),
        Update::ToolCall {
            call_id: "react".to_string(),
            title: "mcp__hotline__react".to_string(),
            kind: "other".to_string(),
        },
        returns("react"),
        says("m-narrate", "Back to the build."),
        calls("c2"),
        returns("c2"),
        ends(),
    ];
    let gate = Arc::new(Semaphore::new(0));
    let agents = Fake::new(Scripted::turns(vec![turn]).gated(gate.clone()).consuming());
    let room = room("reaction-mid-emoji", agents.clone());
    room.start("ada").await.unwrap();
    room.prompt("ada", "build it", None, None).await.unwrap();
    gate.add_permits(3);
    until("the turn is working", || {
        tape(&room, "ada")
            .iter()
            .any(|event| event["kind"] == "tool")
    })
    .await;
    room.prompt("ada", "👍", None, None).await.unwrap();
    until("the emoji joined the turn", || agents.steered().len() == 1).await;
    assert!(agents.steered()[0].ends_with(REACTION));
    gate.add_permits(100);
    until_state(&room, "ada", SessionState::Ready).await;
    let bubbles: Vec<Value> = tape(&room, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "agent")
        .collect();
    assert_eq!(bubbles.len(), 1, "{bubbles:?}");
    assert_eq!(kind_of(&room, "m-narrate").as_deref(), Some("thought"));
}

/// A line the driver took but did not use is handed back to run as a turn
/// of its own, and is heard there as the person said it: no working mark.
#[tokio::test]
async fn a_line_handed_back_is_heard_without_the_working_mark() {
    let first = vec![says("m1", "On it."), calls("c1"), returns("c1"), ends()];
    let gate = Arc::new(Semaphore::new(0));
    let agents = Fake::new(
        Scripted::turns(vec![first, vec![says("m2", "Halfway."), ends()]])
            .gated(gate.clone())
            .steerable(),
    );
    let room = room("reaction-handed-back", agents.clone());
    room.start("ada").await.unwrap();
    room.prompt("ada", "build it", None, None).await.unwrap();
    gate.add_permits(1);
    until("the turn is working", || agents.prompts().len() == 1).await;
    room.prompt("ada", "how far along?", None, None)
        .await
        .unwrap();
    until("the line joined the turn", || agents.steered().len() == 1).await;
    gate.add_permits(100);
    until("the line ran on its own", || agents.prompts().len() == 2).await;
    let heard = &agents.told()[1];
    assert!(heard.contains("how far along?"), "{heard}");
    assert!(!heard.contains(MID_TURN), "{heard}");
}

/// A reaction is often the teammate's first act on the person's line, and
/// the line's read receipt is written at that same moment: both rewrite the
/// line, and neither drops the other's.
#[tokio::test]
async fn a_reaction_and_the_read_receipt_written_together_both_stand() {
    let room = room(
        "reaction-and-receipt",
        Fake::new(Scripted::new(vec![ends()])),
    );
    room.start("ada").await.unwrap();
    let session = room.session("ada").unwrap();
    for round in 0..100 {
        let id = format!("u{round}");
        room.write(
            "ada",
            &serde_json::from_value(
                json!({"kind": "user", "id": id, "ts": round, "text": "👍", "receipt": "sent"}),
            )
            .unwrap(),
        );
        lock(&session.unread).push(id.clone());
        std::thread::scope(|both| {
            both.spawn(|| room.react("ada", "👍").unwrap());
            both.spawn(|| room.mark_read(&session));
        });
        let line = tape(&room, "ada")
            .into_iter()
            .rfind(|event| event["id"] == id.as_str())
            .unwrap();
        assert_eq!(line["receipt"], "read", "round {round}: {line}");
        assert_eq!(line["reactions"], json!(["👍"]), "round {round}: {line}");
    }
}
